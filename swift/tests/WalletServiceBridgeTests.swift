import Foundation
import Testing
@testable import Spectra

@MainActor
struct WalletServiceBridgeTests {
    @Test func storageOpenFailureCanBeRetriedWithoutWritingInMemory() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try Data("blocked".utf8).write(to: directory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let bridge = WalletServiceBridge(databasePath: directory.appendingPathComponent("state.db").path, service: try WalletService(endpoints: []))
        let error = await #expect(throws: (any Error).self, "a failed open must refuse the command") {
            try await bridge.ready().applyStateCommand(command: .setAppSetting(update: .fiatCurrency(value: .eur)))
        }
        if let error { #expect(!String(describing: error).contains("call open_state first")) }
        try FileManager.default.removeItem(at: directory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let state = try await bridge.ready().appState()
        #expect(state.settings.fiatCurrency == .usd)
        _ = try await bridge.ready().applyStateCommand(command: .setAppSetting(update: .fiatCurrency(value: .eur)))
        let reopened = WalletServiceBridge(databasePath: directory.appendingPathComponent("state.db").path, service: try WalletService(endpoints: []))
        let stored = try await reopened.ready().appState()
        #expect(stored.settings.fiatCurrency == .eur)
    }

    @Test func coldBridgeImportOpensStorageAndPersistsBeforeReturningWallet() async throws {
        let secretStore = TestSecretStore()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let path = directory.appendingPathComponent("state.db").path
        let bridge = WalletServiceBridge(databasePath: path, service: try WalletService(endpoints: []))
        try bridge.service().setSecretStore(store: secretStore)
        let outcome = try await bridge.ready().importWallets(commit: WalletImportCommit(
            password: nil,
            request: WalletImportRequest(walletName: "Imported", chain: .ethereum, kind: .phrase),
            derivationPath: nil,
            derivationOverrides: WalletDerivationOverrides(passphrase: nil, hmacKey: nil),
            seedPhrase: "test test test test test test test test test test test junk", privateKey: nil,
            restoreHeight: nil, namedAccount: nil, tonWalletVersion: nil, upgradeWalletId: nil))
        #expect(outcome.wallets.count == 1)
        #expect(outcome.wallets[0].signing == .seedPhrase(passwordProtected: false))
        #expect(try bridge.service().revealSeedPhrase(walletId: outcome.wallets[0].id, password: nil) == .phrase(phrase: "test test test test test test test test test test test junk"))
        let reopened = WalletServiceBridge(databasePath: path, service: try WalletService(endpoints: []))
        let stored = try await reopened.ready().portfolioSnapshot().wallets
        #expect(stored.count == 1)
        _ = try await bridge.ready().applyStateCommand(command: .removeWallet(walletId: outcome.wallets[0].id))
        // Removing the wallet removes its secrets with it.
        #expect(
            (try? bridge.service().revealSeedPhrase(walletId: outcome.wallets[0].id, password: nil))
                != .phrase(phrase: "test test test test test test test test test test test junk"))
    }

    /// Core refuses a private key on a chain that cannot derive from one, and
    /// the refusal reaches the reader through the string tables, with the
    /// chain named. Compared against the table rather than English so it holds
    /// in any locale.
    @Test func aPrivateKeyOnAChainWithoutKeyDerivationIsRefusedInTheReadersLanguage() async throws {
        let secretStore = TestSecretStore()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let bridge = WalletServiceBridge(
            databasePath: directory.appendingPathComponent("state.db").path, service: try WalletService(endpoints: []))
        try bridge.service().setSecretStore(store: secretStore)
        let error = await #expect(throws: SpectraBridgeError.self) {
            try await bridge.ready().importWallets(commit: WalletImportCommit(
                password: nil,
                request: WalletImportRequest(walletName: "Monero key", chain: .monero, kind: .privateKey),
                derivationPath: nil,
                derivationOverrides: WalletDerivationOverrides(passphrase: nil, hmacKey: nil),
                seedPhrase: nil, privateKey: "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318",
                restoreHeight: nil, namedAccount: nil, tonWalletVersion: nil, upgradeWalletId: nil))
        }
        let refusal = try #require(error)
        #expect(
            userErrorMessage(refusal)
                == AppLocalization.format("%@ cannot derive an address from a private key.", Chain.monero.displayName))
        #expect(try await bridge.ready().portfolioSnapshot().wallets.isEmpty)
    }

    /// A sentence with values is looked up by its template and filled in.
    @Test func coreSentencesWithValuesAreTranslatedByTemplate() {
        let message = LocalizableMessage(template: "Insufficient %@ balance.", args: ["ETH"])
        #expect(message.localizedText == AppLocalization.format("Insufficient %@ balance.", "ETH"))
        #expect(message.localizedText.contains("ETH"))
        // Text no table names reads as core sent it.
        #expect(LocalizableMessage(template: "node said no", args: []).localizedText == "Node said no")
    }
}

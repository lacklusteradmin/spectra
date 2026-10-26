import Foundation

extension AppState {
    func wallet(for walletId: String) -> WalletView? { walletDerivedCache.walletById[walletId] }
    /// Reveal a wallet's seed phrase after device authentication. The
    /// password goes to core as typed; core applies its password rule and
    /// says why a phrase was not revealed.
    func revealSeedPhrase(for wallet: WalletView, password: String? = nil) async throws -> String {
        if let failure = await authenticate(.secretMaterial,
            reason: AppLocalization.format("Authenticate to view seed phrase for %@", wallet.name)) {
            throw SeedPhraseRevealError.authenticationFailed(failure)
        }
        let supplied = password.flatMap { $0.isEmpty ? nil : $0 }
        let service = try bridge.service()
        let walletId = wallet.id
        // A synchronous call through the Keychain, the Secure Enclave and the
        // password check: off the main actor, so the UI does not stall on it.
        let reveal = try await Task.detached {
            try service.revealSeedPhrase(walletId: walletId, password: supplied)
        }.value
        switch reveal {
        case .phrase(let phrase): return phrase
        case .notStored: throw SeedPhraseRevealError.unavailable
        case .passwordRequired: throw SeedPhraseRevealError.passwordRequired
        case .incorrectPassword: throw SeedPhraseRevealError.invalidPassword
        case .passwordNotRequired: throw SeedPhraseRevealError.passwordNotRequired
        }
    }
}

extension WalletSigning {
    var isWatchOnly: Bool { self == .watchOnly }
    var isPrivateKey: Bool { if case .privateKey = self { return true } else { return false } }
    var hasSeedPhrase: Bool { if case .seedPhrase = self { return true } else { return false } }
    /// Signing, revealing or scanning needs the wallet's password.
    var requiresPassword: Bool {
        switch self {
        case .watchOnly: return false
        case .seedPhrase(let passwordProtected), .privateKey(let passwordProtected): return passwordProtected
        }
    }
}

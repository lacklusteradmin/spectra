import Foundation
import Testing

@testable import Spectra

/// The network is chosen first, and its page offers what core's setup
/// descriptor lists for it. The form then walks the chosen method's pages.
@MainActor
struct ImportMethodTests {
    private func methods(_ chain: Chain) -> [WalletSetupMethod] {
        walletSetupDescriptor(chain: chain).options.map(\.method)
    }

    @Test func everyNetworkOffersAPhraseAndOnlyWhatItSupports() {
        for chain in Chain.all {
            let offered = methods(chain)
            #expect(offered.starts(with: [.createPhrase, .importPhrase]), "\(chain.id)")
        }
        // Monero needs scan keys: no raw key, and no watching an address alone.
        #expect(methods(.monero) == [.createPhrase, .importPhrase, .watchViewKey])
        #expect(!methods(.bitcoin).contains(.watchViewKey))
        #expect(methods(.bitcoin).contains(.watchMultisig))
        #expect(methods(.bitcoinSignet).contains(.watchMultisig))
        for chain in [Chain.litecoin, .bitcoinCash, .dogecoin, .sui, .aptos, .cardano, .polkadot, .bittensor] {
            #expect(methods(chain).contains(.watchMultisig), "\(chain.id)")
        }
        // A Safe, a Tron account or a TON multisig is watched at its address.
        #expect(!methods(.ethereum).contains(.watchMultisig))
        #expect(methods(.bitcoin).contains(.watchAccountXpub))
        #expect(methods(.litecoin).contains(.watchAccountXpub))
        #expect(methods(.kaspa).contains(.watchAccountXpub))
        #expect(!methods(.ethereum).contains(.watchAccountXpub))
        #expect(methods(.ethereum).contains(.importPrivateKey))
    }

    @Test func eachMethodWalksItsOwnPages() {
        #expect(SetupFlow.forMethod(.createPhrase).pages == [.seedPhrase, .password, .backupVerification, .walletName])
        #expect(SetupFlow.forMethod(.importPhrase).pages == [.seedPhrase, .password, .walletName])
        #expect(SetupFlow.forMethod(.importPrivateKey).pages == [.seedPhrase, .password, .walletName])
        #expect(SetupFlow.forMethod(.watchAddresses).pages == [.watchAddresses, .walletName])
        #expect(SetupFlow.forMethod(.watchAccountXpub).pages == [.watchAddresses, .walletName])
        #expect(SetupFlow.forMethod(.watchViewKey).pages == [.watchAddresses, .walletName])
        #expect(SetupFlow.forMethod(.watchMultisig).pages == [.watchAddresses, .walletName])
    }

    /// The draft reads what core imports from the method it was opened for.
    @Test func theMethodDecidesWhatTheImportCarries() throws {
        let draft = WalletImportDraft()
        draft.configure(chain: .bitcoin, method: .watchAccountXpub)
        draft.watchOnlyInput = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        draft.accountXpubInput = "  xpub123  "
        #expect(draft.importKind == .watchAccountXpub(xpub: "xpub123"))
        draft.configure(chain: .bitcoin, method: .watchAddresses)
        draft.watchOnlyInput = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4\n\n"
        #expect(draft.importKind == .watchAddresses(addresses: ["bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"]))
        #expect(draft.canImportWallet)
        draft.configure(chain: .ethereum, method: .importPrivateKey)
        #expect(draft.importKind == .privateKey)
        #expect(draft.chain == .ethereum)
        // A view key is watched with its wallet's address and a restore
        // height that must be a block number.
        draft.configure(chain: .monero, method: .watchViewKey)
        draft.watchOnlyInput = " 48ZFs… "
        #expect(!draft.canImportWallet)
        draft.viewKeyInput = " ab5e… "
        #expect(draft.importKind == .watchViewKey(address: "48ZFs…", viewKey: "ab5e…"))
        #expect(draft.canImportWallet)
        draft.restoreHeightInput = "12a"
        #expect(!draft.canImportWallet)
        draft.restoreHeightInput = "3000000"
        #expect(try #require(draft.importCommit(name: "")).restoreHeight == 3_000_000)
        draft.configure(chain: .bitcoin, method: .watchMultisig)
        #expect(!draft.canImportWallet)
        draft.descriptorInput = "\n wsh(sortedmulti(2,…)) \n"
        #expect(draft.importKind == .watchMultisig(policy: "wsh(sortedmulti(2,…))"))
        #expect(draft.canImportWallet)
    }

    /// A phrase import derives along the network's first profile at account
    /// 0 unless another is chosen; a custom path replaces both; a network
    /// that derives without a path offers none and sends none.
    @Test func theCommitCarriesTheChosenProfileAndAccount() throws {
        let draft = WalletImportDraft()
        draft.configure(chain: .bitcoin, method: .importPhrase)
        #expect(draft.derivationProfiles == [.nativeSegWit, .legacy, .nestedSegWit, .taproot])
        #expect(try #require(draft.importCommit(name: "")).derivationPath == "m/84'/0'/0'/0/0")
        draft.derivationProfile = .taproot
        draft.derivationAccount = 2
        #expect(try #require(draft.importCommit(name: "")).derivationPath == "m/86'/0'/2'/0/0")
        draft.customDerivationPath = "m/84'/0'/9'/0/0"
        #expect(try #require(draft.importCommit(name: "")).derivationPath == "m/84'/0'/9'/0/0")
        draft.configure(chain: .monero, method: .importPhrase)
        #expect(draft.derivationProfiles.isEmpty)
        #expect(try #require(draft.importCommit(name: "")).derivationPath == nil)
        draft.configure(chain: .ethereum, method: .importPrivateKey)
        #expect(try #require(draft.importCommit(name: "")).derivationPath == nil)
    }

    /// A preview derives only once there is something to derive from, and
    /// never carries the password, which changes no address.
    @Test func thePreviewWaitsForACompleteSecretAndLeavesThePasswordOut() throws {
        let draft = WalletImportDraft()
        draft.configure(chain: .ethereum, method: .importPrivateKey)
        draft.walletPassword = "a long enough password"
        #expect(draft.previewCommit == nil)
        draft.privateKeyInput = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
        let commit = try #require(draft.previewCommit)
        #expect(commit.password == nil && commit.privateKey == draft.privateKeyInput)
        draft.configure(chain: .bitcoin, method: .watchAddresses)
        #expect(draft.previewCommit == nil)
        draft.watchOnlyInput = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        #expect(draft.previewCommit?.request.kind == .watchAddresses(addresses: ["bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"]))
        draft.configure(chain: .solana, method: .createPhrase)
        #expect(draft.previewCommit?.seedPhrase == draft.seedPhrase)
    }

    /// A NEAR key import asks for a named account and carries it; another
    /// network, and creating, ask for none and send none.
    @Test func aNamedAccountIsAskedForOnlyWhereTheNetworkHasThem() throws {
        let draft = WalletImportDraft()
        draft.configure(chain: .near, method: .importPrivateKey)
        #expect(draft.asksNamedAccount)
        draft.namedAccountInput = "alice.near"
        #expect(try #require(draft.importCommit(name: "")).namedAccount == "alice.near")
        draft.configure(chain: .near, method: .createPhrase)
        #expect(!draft.asksNamedAccount)
        draft.configure(chain: .ethereum, method: .importPhrase)
        draft.namedAccountInput = "alice.near"
        #expect(!draft.asksNamedAccount)
        #expect(try #require(draft.importCommit(name: "")).namedAccount == nil)
    }

    /// Creating generates exactly one phrase, on the network chosen.
    @Test func creatingGeneratesOnePhraseOnTheChosenNetwork() {
        let draft = WalletImportDraft()
        draft.configure(chain: .solana, method: .createPhrase)
        #expect(draft.isCreateMode)
        #expect(draft.chain == .solana)
        #expect(draft.seedEntry.verdict.isValid)
        #expect(draft.importKind == .phrase)
    }

    /// Restoring a Monero wallet, or a Zcash one for its shielded funds, asks
    /// where its scan starts; a typed height must be a block number, and no
    /// other network asks.
    @Test func onlyARestoredScanningWalletAsksForARestoreHeight() {
        let draft = WalletImportDraft()
        draft.configure(chain: .monero, method: .importPhrase)
        #expect(draft.asksRestoreHeight)
        draft.restoreHeightInput = "12a"
        #expect(!draft.isRestoreHeightValid)
        draft.restoreHeightInput = "3100000"
        #expect(draft.restoreHeight == 3_100_000)
        draft.configure(chain: .monero, method: .createPhrase)
        #expect(!draft.asksRestoreHeight)
        draft.configure(chain: .zcash, method: .importPhrase)
        #expect(draft.asksRestoreHeight)
        draft.configure(chain: .zcash, method: .importPrivateKey)
        #expect(!draft.asksRestoreHeight)
        draft.configure(chain: .bitcoin, method: .importPhrase)
        #expect(!draft.asksRestoreHeight)
    }

    /// Clearing a form leaves it on no network, so nothing can be imported
    /// from a stale choice.
    @Test func aClearedFormIsOnNoNetwork() {
        let draft = WalletImportDraft()
        draft.configure(chain: .ethereum, method: .importPhrase)
        draft.clear()
        #expect(draft.chain == nil)
        #expect(!draft.canImportWallet)
    }
}

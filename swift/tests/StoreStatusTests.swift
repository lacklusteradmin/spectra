import Foundation
import Testing
@testable import Spectra

/// The app draws nothing of the wallet until core has said what is stored,
/// and what it says decides between the tabs and the unreadable cover.
@MainActor
@Suite(.isolatedAppState)
struct StoreStatusTests: IsolatedAppStateSuite {
    /// A device with a wallet never shows the no-wallet welcome on its way
    /// in: the tabs open with the wallets already read.
    @Test func aReadableStoreOpensWithItsWallets() async throws {
        let first = makeState()
        first.beginWalletSetup(chain: .arbitrum, method: .watchAddresses)
        first.walletImport.draft.walletName = "Watched"
        first.walletImport.draft.watchOnlyInput = "0x000000000000000000000000000000000000dead"
        await first.importWallet()
        try #require(first.walletImport.error == nil)
        await first.stateCommands.awaitPending()

        let relaunched = makeState()
        #expect(relaunched.storeStatus == .opening)
        #expect(relaunched.wallets.isEmpty)
        await relaunched.reloadCoreProjections()
        #expect(relaunched.storeStatus == .open)
        #expect(relaunched.wallets.map(\.name) == ["Watched"])
    }

    /// A file that is not a database is unreadable, not merely failing, so
    /// the app offers the one way out instead of an empty wallet list.
    @Test func dataCoreCannotReadIsUnreadable() async throws {
        let path = directory.appendingPathComponent("garbage.sqlite")
        try Data("this is not a database".utf8).write(to: path)
        let service = try WalletService(endpoints: [])
        service.setSecretStore(store: TestSecretStore())
        let state = AppState(bridge: WalletServiceBridge(databasePath: path.path, service: service), startServices: false)
        await state.reloadCoreProjections()
        #expect(state.storeStatus == .unreadable)
        #expect(state.wallets.isEmpty)
    }
}

import Foundation
import SwiftUI
extension AppState {
    /// Open the form for adding a wallet on `chain` by `method`, one the
    /// network's setup descriptor offers.
    func beginWalletSetup(chain: Chain, method: WalletSetupMethod) {
        walletImport.begin { $0.configure(chain: chain, method: method) }
    }
    /// Open the form that gives the watched wallet `walletId` its keys, by
    /// one of the methods core offers it.
    func beginWalletUpgrade(walletId: String, chain: Chain, method: WalletSetupMethod) {
        walletImport.begin { $0.configure(chain: chain, method: method, upgrading: walletId) }
    }
    func cancelWalletImport() { walletImport.close() }
    func beginEditingWallet(_ wallet: WalletView) {
        walletImport.begin(editing: wallet) { $0.configureForEditing(wallet: wallet) }
    }
    /// Deletes a wallet the user has already confirmed. Nothing is held
    /// between the confirmation and the authentication, so a refused Face ID
    /// leaves no half-started deletion behind.
    func deleteWallet(_ wallet: WalletView) async {
        if let failure = await authenticate(.deleteWallet, reason: AppLocalization.string("Authenticate to delete wallet")) {
            commandError = failure
            return
        }
        let deletedWalletId = wallet.id
        // Core forgets the wallet's secrets, owned addresses, history
        // pagination and diagnostics rows in the same removal.
        guard await removeWallet(id: deletedWalletId) else { return }
        chainDiagnosticsState.diagnosticsRevision &+= 1
        await diagnostics.loadFromSQLite()
        if receiveFlow.walletId == deletedWalletId {
            receiveFlow.reset()
        }
        if sendFlow.walletId == deletedWalletId { cancelSend() }
        if walletImport.editingWalletId == deletedWalletId {
            walletImport.close()
        }
        selectedMainTab = .home
        if wallets.isEmpty { cancelWalletImport() }
    }
    func importWallet() async {
        guard canImportWallet, !walletImport.isBusy else { return }
        let draft = walletImport.draft
        let name = draft.walletName.trimmingCharacters(in: .whitespacesAndNewlines)
        if let walletId = walletImport.editingWalletId {
            await renameWallet(id: walletId, to: name)
            return
        }
        // Snapshot all user input before suspension. The draft may be replaced
        // while core commits, but that must not alter this operation's inputs.
        guard let commit = draft.importCommit(name: name) else { return }
        let completed = await walletImport.submit {
            let outcome = try await self.bridge.ready().importWallets(commit: commit)
            // Core's refresh engine reads the new wallets' balances and
            // history itself, and reports them through its observer.
            await self.rebuildWalletDerivedStateFromCore()
            if outcome.upgraded, let wallet = outcome.wallets.first {
                return AppLocalization.format("Added keys to the watched wallet “%@”.", wallet.name)
            }
            return outcome.rejectedAddresses.isEmpty ? nil : AppLocalization.format(
                "These addresses were not valid and were not imported: %@",
                outcome.rejectedAddresses.joined(separator: ", "))
        }
        if completed { isShowingAddWalletEntry = false }
    }
    /// Add a wallet's key, or watched address, to another network as a
    /// wallet of its own. Core reads the secret and seals the copy; the app
    /// adopts the new wallet as it does an import's.
    func copyWallet(_ commit: WalletCopyCommit) async throws -> WalletImportOutcome {
        let outcome = try await bridge.ready().copyWalletToNetwork(commit: commit)
        await rebuildWalletDerivedStateFromCore()
        return outcome
    }
    func renameWallet(id: String, to newName: String) async {
        let completed = await walletImport.submit {
            try await self.stateCommands.apply(.renameWallet(walletId: id, name: newName))
            return nil
        }
        if completed { isShowingAddWalletEntry = false }
    }
}

import Foundation
import SwiftUI
extension AppState {
    func beginSeedPhraseImport() {
        walletImport.begin { $0.configureForNewWallet() }
    }
    func beginPrivateKeyImport() {
        walletImport.begin { $0.configureForPrivateKeyImport() }
    }
    func beginWatchAddressesImport() {
        walletImport.begin { $0.configureForWatchAddressesImport() }
    }
    func beginWalletCreation() {
        walletImport.begin { $0.configureForCreatedWallet() }
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
        let paths = draft.seedDerivationPaths
        let commit = WalletImportCommit(
            password: draft.walletPasswordInput,
            request: WalletImportRequest(
                walletName: name, selectedChainIds: draft.selectedChains,
                isWatchOnlyImport: draft.isWatchOnlyMode, isPrivateKeyImport: draft.isPrivateKeyImportMode,
                watchOnlyEntries: draft.watchOnlyImportEntries),
            seedDerivationPreset: draft.seedDerivationPreset, seedDerivationPaths: paths,
            derivationOverrides: draft.resolvedDerivationOverrides,
            seedPhrase: draft.seedPhrase, privateKey: draft.privateKeyInput)
        let completed = await walletImport.submit {
            let outcome = try await self.bridge.ready().importWallets(commit: commit)
            // Core's refresh engine reads the new wallets' balances and
            // history itself, and reports them through its observer.
            await self.rebuildWalletDerivedStateFromCore()
            return outcome.rejectedAddresses.isEmpty ? nil : AppLocalization.format(
                "These addresses were not valid and were not imported: %@",
                outcome.rejectedAddresses.joined(separator: ", "))
        }
        if completed { isShowingAddWalletEntry = false }
    }
    func renameWallet(id: String, to newName: String) async {
        let completed = await walletImport.submit {
            try await self.stateCommands.apply(.renameWallet(walletId: id, name: newName))
            return nil
        }
        if completed { isShowingAddWalletEntry = false }
    }
}

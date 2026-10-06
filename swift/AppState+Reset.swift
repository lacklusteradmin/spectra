import Foundation
extension AppState {
    /// `nil` once the reset is done; otherwise why it was not, for the reset sheet.
    func resetSelectedData(scopes: Set<ResetScope>) async -> String? {
        guard !scopes.isEmpty else { return nil }
        if let failure = await authenticate(.resetData, reason: AppLocalization.string("Authenticate to reset wallet data")) {
            return failure
        }
        await stateCommands.awaitPending()
        let outcome: ResetOutcome
        do {
            outcome = try await self.bridge.ready().resetData(scopes: Array(scopes))
        } catch {
            return userErrorMessage(error)
        }
        applyCoreState(outcome.state)
        await rebuildWalletDerivedStateFromCore()
        await refreshTransactionProjection()
        let plan = outcome.plan
        if plan.resetWalletsAndSecrets { resetWalletFlows() }
        if plan.resetHistoryAndCache { resetDiagnosticsViewState() }
        // The five this platform keeps for itself: hiding balances, appearance,
        // Face ID, auto-lock and biometric-gated sends. Each writes itself back
        // to `UserDefaults` as it changes.
        if plan.resetSettingsAndEndpoints { preferences.resetToDefaults() }
        return nil
    }
    private func resetWalletFlows() {
        receiveFlow.reset()
        sendFlow.reset()
        walletImport.close()
        commandError = nil
        isShowingAddWalletEntry = false
    }
    /// Core cleared what it recorded; screens re-read rather than keep a copy.
    private func resetDiagnosticsViewState() {
        chainDiagnosticsState.diagnosticsRevision &+= 1
        lastPendingTransactionRefreshAt = nil
    }
}

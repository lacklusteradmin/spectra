import Foundation

extension AppState {
    /// Ask core to refresh for `intent` and adopt the result. Scheduled ticks
    /// are core's own; they arrive through the refresh observer.
    @discardableResult
    func performCoreRefresh(_ intent: AppRefreshIntent) async -> Bool {
        do {
            let result = try await self.bridge.ready().refreshApp(intent: intent, conditions: deviceConditions())
            return await adoptRefreshResult(result)
        } catch {
            appendOperationalLog(.error, category: "Refresh", message: error.localizedDescription)
            return false
        }
    }

    /// Core evaluated alerts and movement after its own writes and logged its
    /// failures; adopt what it says changed, once, then tell the user.
    @discardableResult
    func adoptRefreshResult(_ result: AppRefreshResult) async -> Bool {
        let portfolioReadSucceeded = await rebuildWalletDerivedStateFromCore()
        let historyReadSucceeded = result.transactionsChanged ? await refreshTransactionProjection() : true
        if let pending = result.pending {
            await notifications.deliverPendingStatusChanges(pending.changes, amounts: amounts)
        }
        lastPendingTransactionRefreshAt = result.pendingCheckedAtUnix.map(Date.init(timeIntervalSince1970:))
        if result.transactionsChanged { await sendFlow.updateVerificationNotice() }
        if result.diagnosticsChanged { await diagnostics.loadFromSQLite() }
        await notifications.deliverPriceAlerts(result.priceAlerts, amounts: amounts)
        if let movement = result.movement { await notifications.deliverPortfolioMovement(movement, amounts: amounts) }
        return portfolioReadSucceeded && historyReadSucceeded
            && result.failures.isEmpty && (result.pending?.failures.isEmpty ?? true)
    }

    @discardableResult
    func performUserInitiatedRefresh() async -> Bool {
        if let existing = userInitiatedRefreshTask { return await existing.value }
        let task = Task { @MainActor [weak self] in
            guard let self else { return false }
            return await self.performCoreRefresh(.user)
        }
        userInitiatedRefreshTask = task
        let succeeded = await task.value
        userInitiatedRefreshTask = nil
        return succeeded
    }
    var pendingTransactionRefreshStatusText: String? {
        guard let at = lastPendingTransactionRefreshAt else { return nil }
        let relative = at.formatted(
            .relative(presentation: .numeric, unitsStyle: .abbreviated).locale(AppLocalization.locale))
        return AppLocalization.format("Last checked %@", relative)
    }
}

import Foundation
extension AppState {
    /// Core resolves the whole thing — grouping, price-request set, and which
    /// coins each wallet can send or receive on. It holds the wallets, so it
    /// hands back coins rather than indices into a list the caller has to
    /// re-walk.
    @discardableResult
    func rebuildWalletDerivedStateFromCore() async -> Bool {
        do {
            let snapshot = try await self.bridge.ready().portfolioSnapshot()
            applyPortfolioSnapshot(snapshot)
            return true
        } catch {
            appendOperationalLog(.error, category: "Portfolio", message: error.localizedDescription)
            return false
        }
    }
    /// Every wallet/quote/dashboard field is adopted together on the main actor.
    /// A field is assigned only when it changed: mid-sweep snapshots arrive
    /// every few hundred milliseconds, and most leave most fields as they were,
    /// so an unchanged one must not invalidate the views that read it.
    func applyPortfolioSnapshot(_ snapshot: PortfolioSnapshot) {
        guard snapshot.revision > portfolioSnapshotRevision else { return }
        guard applyCoreState(snapshot.state) else { return }
        portfolioSnapshotRevision = snapshot.revision
        applyQuoteProjection(snapshot.state)
        if portfolioValuation != snapshot.valuation { portfolioValuation = snapshot.valuation }
        if assetPrecision != snapshot.assetPrecision { adoptAssetPrecision(snapshot.assetPrecision) }
        let derived = snapshot.derived
        let walletById = Dictionary(uniqueKeysWithValues: snapshot.wallets.map { ($0.id, $0) })
        setWalletProjection(snapshot.wallets, identityRevision: snapshot.walletIdentityRevision)
        let cache = WalletDerivedCache(
            walletById: walletById,
            portfolio: derived.portfolio,
            availableSendCoinsByWalletId: derived.sendCoinsByWalletId,
            availableReceiveCoinsByWalletId: derived.receiveCoinsByWalletId,
            sendEnabledWallets: derived.sendEnabledWalletIds.compactMap { walletById[$0] },
            receiveEnabledWallets: derived.receiveEnabledWalletIds.compactMap { walletById[$0] })
        if walletDerivedCache != cache { walletDerivedCache = cache }
        if cachedDashboardAssetGroups != snapshot.groups { cachedDashboardAssetGroups = snapshot.groups }
        if cachedAvailableDashboardPinOptions != snapshot.pinOptions { cachedAvailableDashboardPinOptions = snapshot.pinOptions }
    }
    /// A sweep reports each wallet as core commits it. Read the portfolio at
    /// most once per interval while wallets keep landing, so balances appear
    /// as they arrive rather than when the slowest chain finishes.
    func adoptBalanceProgress() {
        guard balanceProgressTask == nil else { return }
        balanceProgressTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard let self, !Task.isCancelled else { return }
            self.balanceProgressTask = nil
            await self.rebuildWalletDerivedStateFromCore()
        }
    }
    /// Refresh the bounded recent/pending projection and indexed aggregates together.
    @discardableResult
    func refreshTransactionProjection() async -> Bool {
        do {
            let snapshot = try await self.bridge.ready().transactionSnapshot()
            guard snapshot.revision > transactionSnapshotRevision else { return true }
            transactionSnapshotRevision = snapshot.revision
            setTransactionProjection(snapshot.recentAndPending)
            replaceableSends = snapshot.replaceable
            transactionCount = snapshot.totalCount
            historyPaging.adopt(walletsWithMoreHistory: Set(snapshot.walletsWithMoreHistory))
            let firstActivity = Dictionary(uniqueKeysWithValues: snapshot.earliest.map {
                ($0.walletId, Date(timeIntervalSince1970: $0.earliestCreatedAtUnix))
            })
            if cachedFirstActivityDateByWalletId != firstActivity { cachedFirstActivityDateByWalletId = firstActivity }
            historyReadError = nil
            return true
        } catch {
            historyReadError = AppLocalization.string("Unable to read transaction history. Existing records have been kept.")
            return false
        }
    }
}

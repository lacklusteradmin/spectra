import Foundation

extension AppState {
    func loadMoreOnChainHistory(for walletIds: Set<String>) async {
        await historyPaging.loadMore(for: walletIds) {
            await self.adoptHistoryRefresh(scope: .wallets(walletIds: Array($0)), loadMore: true)
        }
    }
    func refreshHistory(chain: Chain) async {
        await adoptHistoryRefresh(scope: .chains(chainIds: [chain.id]))
    }
    /// Run a history refresh and adopt what it changed.
    ///
    /// Core records the run's diagnostics rows and the chain's health itself;
    /// this re-reads them.
    private func adoptHistoryRefresh(scope: HistoryRefreshScope, loadMore: Bool = false) async {
        do {
            let results = try await self.bridge.ready().refreshHistory(
                scope: scope, loadMore: loadMore, limit: nil, intervalSecs: 0)
            chainDiagnosticsState.diagnosticsRevision &+= 1
            await diagnostics.loadFromSQLite()
            // Loading more always moves a cursor, even when every page was already stored.
            if loadMore || results.contains(where: { ($0.outcome?.added ?? 0) > 0 || ($0.outcome?.updated ?? 0) > 0 }) {
                await refreshTransactionProjection()
            }
        } catch {
            appendOperationalLog(category: "History", message: String(describing: error))
        }
    }
    @discardableResult
    func performUserInitiatedRefresh(forChain chain: Chain) async -> Bool {
        await performCoreRefresh(.chain(chainId: chain))
    }
}

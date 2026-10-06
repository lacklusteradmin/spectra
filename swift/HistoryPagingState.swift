import Foundation

/// Paging through on-chain history: which wallets core says have pages left,
/// and whether a page is being fetched.
///
/// Core owns the cursors and the stored history; this holds what the history
/// screen's "Load more" button reads.
@MainActor
@Observable
final class HistoryPagingState {
    /// Wallets whose chain history has pages left, from core's transaction snapshot.
    private(set) var walletsWithMoreHistory: Set<String> = []
    private(set) var isLoadingMore = false

    /// The only writer of `walletsWithMoreHistory`, called with each transaction snapshot.
    func adopt(walletsWithMoreHistory: Set<String>) {
        if self.walletsWithMoreHistory != walletsWithMoreHistory { self.walletsWithMoreHistory = walletsWithMoreHistory }
    }

    func canLoadMore(for walletIds: Set<String>) -> Bool {
        !isLoadingMore && !walletIds.isDisjoint(with: walletsWithMoreHistory)
    }

    /// Fetch the next page for `walletIds` through `fetch`, one fetch at a time.
    func loadMore(for walletIds: Set<String>, fetch: (Set<String>) async -> Void) async {
        guard !isLoadingMore, !walletIds.isEmpty else { return }
        isLoadingMore = true
        defer { isLoadingMore = false }
        await fetch(walletIds)
    }
}

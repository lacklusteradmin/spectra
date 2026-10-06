import Foundation
import Testing
@testable import Spectra

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct HistoryPagingStateTests {
    /// One page fetch at a time: nothing starts another while one is in
    /// flight, and the flag clears when that fetch ends.
    @Test func onePageFetchRunsAtATime() async {
        let paging = HistoryPagingState()
        paging.adopt(walletsWithMoreHistory: ["a"])
        #expect(paging.canLoadMore(for: ["a", "b"]))
        #expect(!paging.canLoadMore(for: ["b"]), "core named no page left for this wallet")

        let gate = SuspensionGate<Void>()
        let first = Task { await paging.loadMore(for: ["a"]) { _ in await gate.wait() } }
        await gate.reached()
        #expect(paging.isLoadingMore)
        #expect(!paging.canLoadMore(for: ["a"]))
        await paging.loadMore(for: ["a"]) { _ in Issue.record("a second fetch started beside the first") }

        gate.resume()
        await first.value
        #expect(!paging.isLoadingMore)
        #expect(paging.canLoadMore(for: ["a"]))
    }
}

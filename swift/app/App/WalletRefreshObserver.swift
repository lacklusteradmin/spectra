// Rust→Swift observer for core's refresh engine.
//
// Core calls these on its own threads; nothing in Swift does. Each call
// becomes one event on a stream that `AppState` drains on the main actor in
// the order core sent them, so a later Tor status or refresh result is never
// overtaken by an earlier one. Every sweep ends with a completed refresh that
// says what changed and what to notify the user about; Swift re-reads those
// projections once and delivers native effects.

import Foundation

final class WalletRefreshObserver: RefreshObserver {
    enum Event: Sendable {
        /// A wallet's balance landed. No payload and no logging: a summary
        /// carries every holding's amount, and the app reads the portfolio.
        case balanceUpdated
        case refreshComplete(AppRefreshResult)
        case torStatusChanged(TorStatus)
    }

    let events: AsyncStream<Event>
    private let continuation: AsyncStream<Event>.Continuation

    init() { (events, continuation) = AsyncStream.makeStream() }
    deinit { continuation.finish() }

    func onBalanceUpdated(chainId: Chain, walletId: String, summary: WalletState?) {
        continuation.yield(.balanceUpdated)
    }
    func onRefreshCycleComplete(refreshed: UInt32, errors: UInt32) {}
    func onRefreshComplete(result: AppRefreshResult) {
        continuation.yield(.refreshComplete(result))
    }
    func onTorStatusChanged(status: TorStatus) {
        continuation.yield(.torStatusChanged(status))
    }
}

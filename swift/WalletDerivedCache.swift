import Foundation

/// Bundled derived state of `AppState.wallets`, as core resolved it.
/// Assigned as one value by `rebuildWalletDerivedStateFromCore`, so readers see one
/// observable update per rebuild rather than a field at a time.
///
/// Every field here has a reader. State the app collects but never shows is
/// not a cache, it is a second copy of core's answer going stale in the dark.
struct WalletDerivedCache: Equatable {
    var walletById: [String: WalletView]
    var portfolio: [AssetHolding]
    var availableSendCoinsByWalletId: [String: [AssetHolding]]
    var availableReceiveCoinsByWalletId: [String: [AssetHolding]]
    var sendEnabledWallets: [WalletView]
    var receiveEnabledWallets: [WalletView]

    static let empty = WalletDerivedCache(
        walletById: [:],
        portfolio: [],
        availableSendCoinsByWalletId: [:],
        availableReceiveCoinsByWalletId: [:],
        sendEnabledWallets: [],
        receiveEnabledWallets: []
    )
}

import SwiftUI

/// What every composer page reads of the wallet projection, read once per
/// page body.
@MainActor
struct SendComposerPresentation {
    let sendWallets: [WalletView]
    let selectedWallet: WalletView?
    let availableSendCoins: [AssetHolding]
    let selectedCoin: AssetHolding?
    let selectedCoinAmountText: String?
    let selectedCoinApproximateFiatText: String?
    let addressBookEntries: [AddressBookEntry]

    init(store: AppState) {
        sendWallets = store.sendEnabledWallets
        selectedWallet = sendWallets.first(where: { $0.id == store.sendFlow.walletId })
        availableSendCoins = store.availableSendCoins(for: store.sendFlow.walletId)
        selectedCoin = availableSendCoins.first(where: { $0.holdingKey == store.sendFlow.holdingKey })
        selectedCoinAmountText = selectedCoin.map { store.amounts.formattedAssetAmount($0.amount, symbol: $0.symbol, deploymentId: $0.holdingKey) }
        selectedCoinApproximateFiatText = store.amounts.formattedFiatIfAvailable(store.sendQuoteForEnteredAmount?.amountValue)
        addressBookEntries = store.sendAddressBookEntries
    }
}

/// The small uppercase label over a group inside a send card.
@MainActor
func sendComposerSectionLabel(_ title: String) -> some View {
    Text(AppLocalization.string(title))
        .font(.caption.weight(.semibold))
        .foregroundStyle(.secondary)
        .textCase(.uppercase)
}

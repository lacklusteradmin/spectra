import SwiftUI

@MainActor
struct SendFromPage: View {
    @Bindable var store: AppState
    private static let assetBadgeSize: CGFloat = 40
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            FlowPageHeading(title: "Choose an asset to send", subtitle: "Choose an asset from your wallet.")
            // Both cards headed alike: a title over its rows.
            SpectraRowSection(title: AppLocalization.string("Sending wallet")) {
                walletRow.spectraRowPadding()
            }

            if !presentation.availableSendCoins.isEmpty {
                // One card, a row per asset, the choice marked with a
                // trailing checkmark — as every selectable list is.
                SpectraRowGroup(
                    title: AppLocalization.string("Available assets"), data: presentation.availableSendCoins,
                    dividerInset: SpectraLayout.rowHorizontal + Self.assetBadgeSize + SpectraLayout.Space.m
                ) { coin in
                    assetRow(coin: coin, isSelected: coin.holdingKey == store.sendFlow.holdingKey)
                }
            }
        }
    }

    private var walletRow: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            if let selectedWallet = presentation.selectedWallet {
                let badge = AssetHolding.nativeChainBadge(for: selectedWallet.family) ?? (nil, Color.mint)
                CoinBadge(
                    artworkName: badge.artworkName,
                    fallbackText: selectedWallet.familyName,
                    color: badge.color,
                    size: 36
                )
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(selectedWallet.name).font(.headline).lineLimit(1)
                    // The network, not the family: a Sepolia wallet read
                    // "Ethereum" and looked like a mainnet one.
                    Text(selectedWallet.networkTitle).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
            if presentation.sendWallets.count > 1 {
                Menu {
                    // Each with its network: two wallets of one name, on
                    // mainnet and on Sepolia, read the same by name alone.
                    Picker(AppLocalization.string("Wallet"), selection: Bindable(store.sendFlow).walletId) {
                        ForEach(presentation.sendWallets) { wallet in
                            VStack {
                                Text(wallet.name)
                                Text(wallet.networkTitle)
                            }
                            .tag(wallet.id)
                        }
                    }
                } label: {
                    Text(AppLocalization.string("Change"))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.tint)
                }
                .onChange(of: store.sendFlow.walletId) { _, _ in store.syncSendAssetSelection() }
            }
        }
    }

    /// One row per asset, full width.
    private func assetRow(coin: AssetHolding, isSelected: Bool) -> some View {
        Button {
            guard !isSelected else { return }
            store.sendFlow.holdingKey = coin.holdingKey
            spectraHaptic(.light)
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(artworkName: coin.artworkName, fallbackText: coin.symbol, color: coin.color, size: Self.assetBadgeSize)
                (dynamicTypeSize.isAccessibilitySize
                    ? AnyLayout(VStackLayout(alignment: .leading, spacing: SpectraLayout.Space.s))
                    : AnyLayout(HStackLayout(spacing: SpectraLayout.Space.m))) {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        Text(coin.symbol).font(.headline)
                        Text(coin.name).font(.caption).foregroundStyle(.secondary)
                    }
                    if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: SpectraLayout.Space.s) }
                    Text(store.amounts.formattedAssetAmount(coin.amount, symbol: coin.symbol, deploymentId: coin.holdingKey))
                        .font(.subheadline.weight(.medium))
                        .spectraNumericTextLayout()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Image(systemName: "checkmark")
                    .font(.headline)
                    .foregroundStyle(.tint)
                    .opacity(isSelected ? 1 : 0)
                    .accessibilityHidden(true)
            }
            .spectraRowPadding()
            .frame(minHeight: 60)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

import SwiftUI

/// One asset across every wallet: what it is worth, where and by whom it is
/// held, and how to move it. Read from core's current groups by id, so a refresh lands here
/// as it lands on Home.
struct AssetGroupDetailView: View {
    let store: AppState
    let assetGroupId: String
    private var assetGroup: DashboardAssetGroup? {
        store.cachedDashboardAssetGroups.first { $0.id == assetGroupId }
    }
    /// Where a token is deployed is Known Tokens' job; a native coin has no
    /// entry there, and no button.
    private var isKnownToken: Bool {
        store.tokenPreferences.entries.contains { $0.token.tokenId == assetGroupId }
    }
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                if let assetGroup {
                    AssetDetailHeroCard(assetGroup: assetGroup, store: store)
                    AssetMoveButtons(store: store, assetGroup: assetGroup)
                    AssetChainBreakdownCard(assetGroup: assetGroup, store: store)
                    AssetWalletBreakdownCard(assetGroup: assetGroup, store: store)
                } else {
                    SpectraEmptyStateCard(
                        title: "Asset not listed", message: "This asset is no longer in your portfolio.",
                        systemImage: "chart.pie")
                }
            }.spectraScreenPadding()
        }.background(SpectraBackdrop().ignoresSafeArea())
            .navigationTitle(assetGroup?.symbol ?? "").navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .toolbar {
                if isKnownToken {
                    ToolbarItem(placement: .topBarTrailing) {
                        NavigationLink {
                            TokenRegistryDetailView(tokens: store.tokenPreferences, groupKey: assetGroupId)
                        } label: {
                            Image(systemName: "info.circle")
                        }
                        .accessibilityLabel(AppLocalization.string("Token Details"))
                    }
                }
            }
            .refreshable { await store.performUserInitiatedRefresh() }
    }
}
/// Send and Receive for this asset, from the wallets that can: Send opens on
/// a wallet holding it, Receive on one whose network it lives on.
private struct AssetMoveButtons: View {
    let store: AppState
    let assetGroup: DashboardAssetGroup
    private var holdingKeys: Set<String> { Set(assetGroup.holdings.map(\.coin.holdingKey)) }
    private var sendTarget: (walletId: String, holdingKey: String)? {
        for wallet in store.sendEnabledWallets {
            if let coin = store.availableSendCoins(for: wallet.id).first(where: { holdingKeys.contains($0.holdingKey) }) {
                return (wallet.id, coin.holdingKey)
            }
        }
        return nil
    }
    private var receiveWalletId: String? {
        let chains = Set(assetGroup.holdings.map(\.coin.chainId))
        return store.receiveEnabledWallets.first { chains.contains($0.chainId) }?.id
    }
    var body: some View {
        let send = sendTarget
        let receive = receiveWalletId
        GlassEffectContainer(spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                Button {
                    spectraHaptic(.medium)
                    if let send { store.beginSend(walletId: send.walletId, holdingKey: send.holdingKey) }
                } label: {
                    Label(AppLocalization.string("Send"), systemImage: "arrow.up.right")
                        .font(.body.weight(.semibold)).frame(maxWidth: .infinity).padding(.vertical, SpectraLayout.Space.m)
                }.buttonStyle(.glass).disabled(send == nil)
                Button {
                    spectraHaptic(.medium)
                    if let receive { store.beginReceive(walletId: receive) }
                } label: {
                    Label(AppLocalization.string("Receive"), systemImage: "arrow.down.left")
                        .font(.body.weight(.semibold)).frame(maxWidth: .infinity).padding(.vertical, SpectraLayout.Space.m)
                }.buttonStyle(.glass).tint(.accentColor).disabled(receive == nil)
            }
        }
    }
}
private struct AssetDetailHeroCard: View {
    let assetGroup: DashboardAssetGroup
    let store: AppState
    var body: some View {
        let hidden = store.preferences.hideBalances
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: assetGroup.artworkName, fallbackText: assetGroup.symbol,
                color: assetGroup.color, size: 60
            )
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                Text(assetGroup.name).font(.title2.weight(.bold)).foregroundStyle(Color.primary)
                    .fixedSize(horizontal: false, vertical: true)
                BalanceText(text: store.amounts.formattedFiat(assetGroup.totalValue), isHidden: hidden)
                    .font(.title3.weight(.semibold)).foregroundStyle(Color.primary)
                    .spectraNumericTextLayout(minimumScaleFactor: 0.7)
                BalanceText(
                    text: store.amounts.formattedAssetAmount(
                        assetGroup.totalAmount, symbol: assetGroup.symbol, deploymentId: assetGroup.identity.holdingKey),
                    isHidden: hidden
                ).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                    .spectraNumericTextLayout(minimumScaleFactor: 0.7)
                Text(AppLocalization.format("dashboard.asset.price", store.amounts.formattedFiat(assetGroup.price), assetGroup.symbol))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraElevatedFill()
    }
}
/// Every place the row's asset is held, largest first.
///
/// The badge is the **chain's**, which is what the row is about: USDC on
/// Ethereum and USDC on Solana must not look alike. The asset's colour stands
/// in for a chain the registry has no artwork for.
private struct AssetChainBreakdownCard: View {
    let assetGroup: DashboardAssetGroup
    let store: AppState
    var body: some View {
        AssetBreakdownCard(title: "Chain Breakdown", count: assetGroup.holdings.count) { index in
            let holding = assetGroup.holdings[index]
            let badge = AssetHolding.nativeChainBadge(for: holding.coin.chain) ?? (nil, holding.coin.color)
            AssetBreakdownRow(
                badgeArtworkName: badge.artworkName, badgeColor: badge.color, badgeMark: holding.coin.chainName,
                title: holding.coin.chainName, subtitle: holding.coin.standardLabel,
                amountText: store.amounts.formattedAssetAmount(
                    holding.coin.amount, symbol: holding.coin.symbol, deploymentId: holding.coin.holdingKey),
                valueText: store.amounts.formattedFiat(holding.value),
                hidesBalance: store.preferences.hideBalances)
        }
    }
}
/// Every wallet that holds the row's asset, largest first, each summed over
/// the networks it holds it on. The badge is the wallet's, as in the wallet
/// list.
private struct AssetWalletBreakdownCard: View {
    let assetGroup: DashboardAssetGroup
    let store: AppState
    private var shares: [(share: DashboardAssetWallet, wallet: WalletView)] {
        assetGroup.wallets.compactMap { share in store.wallet(for: share.walletId).map { (share, $0) } }
    }
    var body: some View {
        let shares = shares
        AssetBreakdownCard(title: "Wallet Breakdown", count: shares.count) { index in
            let (share, wallet) = shares[index]
            let badge = AssetHolding.nativeChainBadge(for: wallet.family) ?? (nil, .mint)
            AssetBreakdownRow(
                badgeArtworkName: badge.artworkName, badgeColor: badge.color, badgeMark: wallet.familyName,
                title: wallet.name, subtitle: wallet.networkTitle,
                amountText: store.amounts.formattedAssetAmount(
                    share.amount, symbol: assetGroup.symbol, deploymentId: assetGroup.identity.holdingKey),
                valueText: store.amounts.formattedFiat(share.value),
                hidesBalance: store.preferences.hideBalances)
        }
    }
}
/// A titled card of breakdown rows with dividers between them.
private struct AssetBreakdownCard<Row: View>: View {
    let title: String
    let count: Int
    @ViewBuilder let row: (Int) -> Row
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string(title)).font(.headline).foregroundStyle(Color.primary)
            ForEach(0..<count, id: \.self) { index in
                row(index)
                if index < count - 1 { Divider().opacity(0.4) }
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraCardFill()
    }
}
/// One share of the asset: where or who, then how much and what it is worth.
private struct AssetBreakdownRow: View {
    let badgeArtworkName: String?
    let badgeColor: Color
    let badgeMark: String
    let title: String
    let subtitle: String
    let amountText: String
    let valueText: String
    let hidesBalance: Bool
    var body: some View {
        HStack(alignment: .center, spacing: SpectraLayout.Space.m) {
            CoinBadge(artworkName: badgeArtworkName, fallbackText: badgeMark, color: badgeColor, size: 30)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(title).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary).lineLimit(1)
                Text(subtitle).font(.caption2).foregroundStyle(.secondary)
            }
            Spacer(minLength: SpectraLayout.Space.m)
            VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                BalanceText(text: amountText, isHidden: hidesBalance)
                    .font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary).spectraNumericTextLayout()
                BalanceText(text: valueText, isHidden: hidesBalance)
                    .font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
            }
        }
    }
}

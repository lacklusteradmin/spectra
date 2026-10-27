import SwiftUI

struct WalletCardView: View, Equatable {
    struct Presentation: Equatable {
        let walletName: String
        let chainTitleText: String
        let totalValueText: String
        let hidesBalance: Bool
        /// Core has not read this wallet's balances yet.
        let isReadingBalances: Bool
        let assetCountText: String
        let isWatchOnly: Bool
        let badgeArtworkName: String?
        let badgeMark: String
        let badgeColor: Color
    }
    let presentation: Presentation
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.presentation == rhs.presentation }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(
                    artworkName: presentation.badgeArtworkName, fallbackText: presentation.badgeMark,
                    color: presentation.badgeColor, size: 36)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(presentation.walletName).font(.headline).foregroundStyle(Color.primary)
                    // Watching is said on the network line: a badge on the
                    // name line took the width the name needed beside a
                    // balance, and broke "Cold BTC" in two.
                    HStack(spacing: SpectraLayout.Space.xs) {
                        if presentation.isWatchOnly {
                            Image(systemName: "eye").foregroundStyle(.tint)
                                .accessibilityLabel(AppLocalization.string("Watching"))
                        }
                        Text(presentation.chainTitleText).foregroundStyle(.secondary).lineLimit(2)
                    }
                    .font(.caption2)
                }
                Spacer()
                VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                    if presentation.isReadingBalances {
                        SpectraShimmer(height: 14).frame(width: 70)
                            .accessibilityLabel(AppLocalization.string("Reading balances…"))
                    } else {
                        BalanceText(text: presentation.totalValueText, isHidden: presentation.hidesBalance)
                            .font(.headline).foregroundStyle(Color.primary).spectraNumericTextLayout()
                        Text(presentation.assetCountText).font(.caption2).foregroundStyle(.secondary)
                    }
                }
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
        }
    }
}
extension WalletCardView.Presentation {
    /// A wallet as every list of wallets shows it: Home's and Receive's.
    @MainActor
    init(wallet: WalletView, store: AppState) {
        let badge = AssetHolding.nativeChainBadge(for: wallet.family) ?? (nil, .mint)
        let assetCount = wallet.shownHoldings.count
        self.init(
            walletName: wallet.name, chainTitleText: wallet.networkTitle,
            totalValueText: store.amounts.formattedWalletTotal(walletId: wallet.id),
            hidesBalance: store.preferences.hideBalances,
            isReadingBalances: wallet.balancesReadAt == nil,
            assetCountText: AppLocalization.format("%lld assets", count: assetCount, assetCount),
            isWatchOnly: wallet.signing.isWatchOnly, badgeArtworkName: badge.0,
            badgeMark: wallet.familyName, badgeColor: badge.1)
    }
}

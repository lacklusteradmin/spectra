import SwiftUI

/// The current asset and exact wallet network remain visible while composing.
@MainActor
struct SendAssetContextView: View {
    let store: AppState

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        if let coin = presentation.selectedCoin {
            HStack(spacing: SpectraLayout.Space.s) {
                CoinBadge(artworkName: coin.artworkName, fallbackText: coin.symbol, color: coin.color, size: 24)
                Text(coin.symbol).font(.subheadline.weight(.semibold))
                Text(presentation.selectedWallet?.networkTitle ?? coin.chainName)
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(SpectraLayout.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .spectraInsetFill()
            .accessibilityElement(children: .combine)
        }
    }
}

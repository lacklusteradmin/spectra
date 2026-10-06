import Foundation
import SwiftUI
import UIKit
struct DonationsView: View {
    @State private var copiedAddress: String?
    private var copy: DonationsContentCopy { DonationsContentCopy.current }
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                heroCard
                addressesCard
            }.spectraScreenPadding()
        }.background(SpectraBackdrop().ignoresSafeArea())
            .navigationTitle(copy.navigationTitle).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .onDisappear { copiedAddress = nil }
    }
    private var heroCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Image(systemName: "heart.fill").font(.largeTitle.weight(.bold)).foregroundStyle(.tint)
            Text(copy.navigationTitle).font(.title.weight(.bold)).foregroundStyle(Color.primary)
            Text(copy.heroSubtitle).font(.subheadline).foregroundStyle(.secondary)
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraElevatedFill()
    }
    private var addressesCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("Addresses")).font(.headline).foregroundStyle(Color.primary)
            ForEach(copy.destinations, id: \.address) { destination in
                donationRow(chain: destination.chainId, title: destination.title, address: destination.address)
                if destination.address != copy.destinations.last?.address {
                    Divider().opacity(0.25)
                }
            }
            Text(AppLocalization.string("Tap an address to copy it.")).font(.caption).foregroundStyle(.secondary)
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraCardFill()
    }
    @ViewBuilder
    private func donationRow(chain: Chain, title: String, address: String) -> some View {
        let badge = AssetHolding.nativeChainBadge(for: chain) ?? (artworkName: nil, color: Color.mint)
        let isCopied = copiedAddress == address
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(artworkName: badge.artworkName, fallbackText: title, color: badge.color, size: 32)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(title).font(.body.weight(.semibold)).foregroundStyle(Color.primary)
                Text(address).font(.footnote.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    .textSelection(.enabled)
            }
            Spacer(minLength: SpectraLayout.Space.s)
            Button {
                UIPasteboard.general.string = address
                copiedAddress = address
                spectraHaptic(.light)
            } label: {
                Image(systemName: isCopied ? "checkmark" : "doc.on.doc").font(.body.weight(.semibold))
                    .accessibilityLabel(AppLocalization.string(isCopied ? "Copied" : "Copy"))
            }.buttonStyle(.glass).tint(isCopied ? .green : .accentColor)
        }.padding(.vertical, SpectraLayout.Space.xs)
    }
}

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
    /// One card, a row per address. The whole row copies — what the footer
    /// says a tap does — and says so for a moment.
    private var addressesCard: some View {
        SpectraRowGroup(
            title: AppLocalization.string("Addresses"), data: copy.destinations.map(DonationRow.init),
            footer: {
                Text(AppLocalization.string("Tap an address to copy it.")).font(.caption).foregroundStyle(.secondary)
                    .padding(.horizontal, SpectraLayout.rowHorizontal).padding(.bottom, SpectraLayout.Space.m)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        ) { row in
            donationRow(chain: row.destination.chainId, title: row.destination.title, address: row.destination.address)
        }
        .task(id: copiedAddress) {
            guard copiedAddress != nil else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            copiedAddress = nil
        }
    }
    private struct DonationRow: Identifiable {
        let destination: DonationDestination
        var id: String { destination.address }
    }
    private func donationRow(chain: Chain, title: String, address: String) -> some View {
        let badge = AssetHolding.nativeChainBadge(for: chain) ?? (artworkName: nil, color: Color.mint)
        let isCopied = copiedAddress == address
        return Button {
            UIPasteboard.general.string = address
            copiedAddress = address
            spectraHaptic(.light)
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(artworkName: badge.artworkName, fallbackText: title, color: badge.color, size: 36)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(title).font(.body.weight(.semibold)).foregroundStyle(Color.primary)
                    Text(address).font(.footnote.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: SpectraLayout.Space.s)
                Image(systemName: isCopied ? "checkmark" : "doc.on.doc").font(.body.weight(.semibold))
                    .foregroundStyle(isCopied ? Color.green : Color.accentColor)
                    .contentTransition(.symbolEffect(.replace))
            }
            .spectraRowPadding()
        }
        .buttonStyle(.plain)
        .accessibilityHint(AppLocalization.string(isCopied ? "Copied" : "Copy"))
    }
}

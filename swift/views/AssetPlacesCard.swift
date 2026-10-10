import SwiftUI

/// Where a coin lives: chain, token standard, and contract (absent for native coins).
/// Shown on a held asset's page. Show contracts so users can verify asset
/// identity independently of the ticker: whole, copyable, and read out.
struct AssetPlacesCard: View {
    let places: [AssetWikiPlace]
    let symbol: String

    var body: some View {
        SpectraRowGroup(
            title: AppLocalization.string("Lives On"),
            trailing: places.count > 1 ? "\(places.count)" : nil,
            data: places, dividerInset: SpectraLayout.rowHorizontal,
            footer: {
                if places.isEmpty {
                    Text(AppLocalization.string("No chains are listed for this asset."))
                        .font(.subheadline).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .spectraRowPadding()
                }
            }
        ) { place in
            VStack(alignment: .leading, spacing: 0) {
                // A chain still has a page — for consensus and state model,
                // which have no coin to belong to — reached from here. Only
                // a row that leads there draws a chevron.
                if let chain = CoreReferenceTables.chainWikiEntry(id: place.chainId.id) {
                    NavigationLink { ChainWikiDetailView(chain: chain) } label: { header(place, isLink: true) }
                        .buttonStyle(.plain)
                } else {
                    header(place, isLink: false)
                }
                contract(place)
            }
        }
    }

    private func header(_ place: AssetWikiPlace, isLink: Bool) -> some View {
        HStack(spacing: SpectraLayout.Space.s) {
            Text(place.chainName).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
            Spacer()
            Text(place.tokenStandard).font(.caption.weight(.semibold)).foregroundStyle(.tint)
                .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xxs)
                .spectraInsetFill(cornerRadius: SpectraLayout.Radius.inner)
            if isLink {
                Image(systemName: "chevron.right").font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .padding(.horizontal, SpectraLayout.rowHorizontal)
        .padding(.top, SpectraLayout.rowVertical)
        .frame(minHeight: 36)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityLabel(AppLocalization.format("place.label_format", symbol, place.chainName, place.tokenStandard))
    }

    @ViewBuilder
    private func contract(_ place: AssetWikiPlace) -> some View {
        Group {
            if place.contract.isEmpty {
                // Native here, so there is no contract — saying so is the
                // honest answer and it is what distinguishes the two kinds of
                // place without a second flag that could disagree.
                Text(AppLocalization.format("wiki.place.nativeTo", place.chainName))
                    .font(.footnote).foregroundStyle(.secondary)
                    .padding(.bottom, SpectraLayout.rowVertical)
            } else {
                HStack(alignment: .center, spacing: SpectraLayout.Space.xs) {
                    Text(verbatim: breakableAnywhere(place.contract)).font(.footnote.monospaced())
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel(AppLocalization.format("place.contract_format", place.contract))
                    CopyButton(value: place.contract)
                }
            }
        }
        .padding(.horizontal, SpectraLayout.rowHorizontal)
    }
}

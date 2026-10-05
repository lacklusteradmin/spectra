import SwiftUI

/// A chain's own page, one level down from the coin that runs on it.
///
/// The wiki is indexed by coin — see `CryptoWikiViews.swift`. This is what has
/// no coin to belong to: ten chains share ETH, so "Base is an optimistic
/// rollup" cannot live on ETH's page. Reached from the wiki's Chains section
/// and a held asset's lives-on rows.
struct ChainWikiDetailView: View {
    let chain: ChainWikiEntry
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(spacing: SpectraLayout.Space.m) {
                wikiHeroCard
                wikiIdentityCard
            }
            .spectraScreenPadding()
        }
        .background(SpectraBackdrop().ignoresSafeArea())
        .navigationTitle(chain.name).navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
    }

    private var wikiHeroCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {

            HStack(spacing: SpectraLayout.Space.m) {
                WikiCoinBadge(face: chain.face, size: 52)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(chain.name).font(.title3.weight(.semibold))
                }
                Spacer(minLength: 0)
            }
            Text(chain.comment).font(.subheadline).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if !chain.tags.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: SpectraLayout.Space.xs) {
                        ForEach(chain.tags, id: \.self) { tag in
                            Text(tag.title).font(.caption.weight(.semibold)).foregroundStyle(chain.face.color)
                                .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs)
                                .background(chain.face.color.opacity(0.14), in: Capsule())
                        }
                    }
                }
            }
        }
        .padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }

    private var wikiIdentityCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            wikiStatRow(label: AppLocalization.string("Family"), value: chain.family, icon: "link.circle.fill")
            Divider().opacity(0.4)
            wikiStatRow(label: AppLocalization.string("Consensus"), value: chain.consensus, icon: "checkmark.shield.fill")
            Divider().opacity(0.4)
            wikiStatRow(label: AppLocalization.string("State Model"), value: chain.stateModel, icon: "cylinder.split.1x2.fill")
        }
        .padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    private func wikiStatRow(label: String, value: String, icon: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
            Image(systemName: icon)
                .font(.subheadline.weight(.semibold)).foregroundStyle(.tint).frame(width: 22)
            Text(label).font(.subheadline).foregroundStyle(.secondary)
            Spacer(minLength: SpectraLayout.Space.m)
            Text(value).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
                .multilineTextAlignment(.trailing)
        }
    }
}

extension ChainWikiEntry {
    /// A chain draws the coin it runs on, which is what the badge already was.
    var face: WikiCoinFace {
        let entry = Chain(id: id)?.entry
        return WikiCoinFace(
            name: name, symbol: name,
            artworkName: entry?.artworkName ?? "",
            color: entry?.color.color ?? .accentColor)
    }
}

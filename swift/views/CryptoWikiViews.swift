import SwiftUI

/// The wiki is indexed by coin. `listAssetWiki()` joins both catalogs so
/// each coin has one page. Chains have their own section; where a coin is
/// deployed is Known Tokens' job, so neither repeats it.
extension AssetWikiEntry: Identifiable {
    public var id: String { tokenId }
    var accentColor: Color { color?.color ?? .accentColor }
    var face: WikiCoinFace {
        WikiCoinFace(name: name, symbol: symbol, artworkName: artworkName, color: accentColor)
    }
}

extension AssetWikiPlace: Identifiable {
    public var id: String { "\(chainId)|\(contract)" }
}

extension ChainWikiEntry: Identifiable {}

/// One filter over the whole library: what kind of coin, or what kind of chain.
private enum WikiTag: Hashable {
    case token(TokenTag)
    case chain(ChainTag)

    var title: String {
        switch self {
        case .token(let tag): tag.title
        case .chain(let tag): tag.title
        }
    }
}

// MARK: — Library (list view)

struct CryptoWikiLibraryView: View {
    @State private var searchText: String = ""
    @State private var selectedTag: WikiTag?
    private var query: String { searchText.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var filteredCoins: [AssetWikiEntry] {
        CoreReferenceTables.assetWiki.filter { entry in
            matches(tags: entry.tags.map(WikiTag.token), text: [entry.name, entry.symbol, entry.comment])
        }
    }
    private var filteredChains: [ChainWikiEntry] {
        CoreReferenceTables.chainWiki.filter { chain in
            matches(
                tags: chain.tags.map(WikiTag.chain),
                text: [chain.name, chain.comment, chain.family, chain.consensus, chain.stateModel])
        }
    }
    private func matches(tags: [WikiTag], text: [String]) -> Bool {
        if let selectedTag, !tags.contains(selectedTag) { return false }
        guard !query.isEmpty else { return true }
        return (text + tags.map(\.title)).contains { $0.localizedCaseInsensitiveContains(query) }
    }
    /// Coin tags, then chain tags, as the sections are ordered; each in core's
    /// order, and only the tags some row carries.
    private var availableTags: [WikiTag] {
        let coinTags = Set(CoreReferenceTables.assetWiki.flatMap(\.tags))
        let chainTags = Set(CoreReferenceTables.chainWiki.flatMap(\.tags))
        return TokenTag.filterOrder.filter(coinTags.contains).map(WikiTag.token)
            + ChainTag.pickerOrder.filter(chainTags.contains).map(WikiTag.chain)
    }
    var body: some View {
        let coins = filteredCoins
        let chains = filteredChains
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollView(showsIndicators: false) {
                LazyVStack(spacing: SpectraLayout.sectionSpacing) {
                    if !coins.isEmpty {
                        SpectraRowGroup(title: AppLocalization.string("Coins"), trailing: "\(coins.count)", data: coins) { asset in
                            NavigationLink {
                                AssetWikiDetailView(asset: asset)
                            } label: {
                                CryptoWikiRow(face: asset.face, subtitle: asset.symbol).equatable()
                            }
                            .buttonStyle(.plain)
                            .simultaneousGesture(TapGesture().onEnded { spectraHaptic(.light) })
                        }
                    }
                    if !chains.isEmpty {
                        SpectraRowGroup(title: AppLocalization.string("Chains"), trailing: "\(chains.count)", data: chains) { chain in
                            NavigationLink {
                                ChainWikiDetailView(chain: chain)
                            } label: {
                                CryptoWikiRow(face: chain.face, subtitle: chain.family).equatable()
                            }
                            .buttonStyle(.plain)
                            .simultaneousGesture(TapGesture().onEnded { spectraHaptic(.light) })
                        }
                    }
                }
                .spectraScreenPadding()
            }.overlay {
                if coins.isEmpty && chains.isEmpty { ContentUnavailableView.search }
            }
        }
        .navigationTitle(AppLocalization.string("Crypto Wiki"))
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, prompt: AppLocalization.string("Search coins and chains"))
        .textInputAutocapitalization(.never).autocorrectionDisabled()
        .toolbarBackground(.hidden, for: .navigationBar)
        .sensoryFeedback(.impact(weight: .light), trigger: selectedTag)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Picker(AppLocalization.string("Tag"), selection: $selectedTag) {
                        Text(AppLocalization.string("All")).tag(WikiTag?.none)
                        ForEach(availableTags, id: \.self) { tag in
                            Text(tag.title).tag(WikiTag?.some(tag))
                        }
                    }
                } label: {
                    Image(systemName: selectedTag == nil
                        ? "line.3.horizontal.decrease.circle"
                        : "line.3.horizontal.decrease.circle.fill")
                }
                .accessibilityLabel(AppLocalization.string("Filter by tag"))
            }
        }
    }
}

/// A coin or a chain in the library: badge, name, one line under it.
private struct CryptoWikiRow: View, Equatable {
    let face: WikiCoinFace
    let subtitle: String
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.face == rhs.face && lhs.subtitle == rhs.subtitle
    }
    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            WikiCoinBadge(face: face, size: 36)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(face.name).font(.headline).foregroundStyle(Color.primary)
                Text(subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 0)
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .spectraRowPadding()
    }
}

// MARK: — Asset detail

struct AssetWikiDetailView: View {
    let asset: AssetWikiEntry
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(spacing: SpectraLayout.Space.m) {
                heroCard
                if !asset.totalCirculationModel.isEmpty {
                    circulationCard
                }
            }
            .spectraScreenPadding()
        }
        .background(SpectraBackdrop().ignoresSafeArea())
        .navigationTitle(asset.name).navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
    }

    private var heroCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack(spacing: SpectraLayout.Space.m) {
                WikiCoinBadge(face: asset.face, size: 52)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(asset.name).font(.title3.weight(.semibold))
                    Text(asset.symbol).font(.subheadline.monospaced()).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            Text(asset.comment).font(.subheadline).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if !asset.tags.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: SpectraLayout.Space.xs) {
                        ForEach(asset.tags, id: \.self) { tag in
                            Text(tag.title)
                                .font(.caption.weight(.semibold)).foregroundStyle(asset.accentColor)
                                .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs)
                                .background(asset.accentColor.opacity(0.14), in: Capsule())
                        }
                    }
                }
            }
        }
        .padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }

    /// Supply caps belong to coins.
    private var circulationCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: "chart.bar.fill")
                    .font(.subheadline.weight(.semibold)).foregroundStyle(.tint).frame(width: 22)
                Text(AppLocalization.string("Circulation Model"))
                    .font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
            }
            Text(asset.totalCirculationModel).font(.subheadline).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true).padding(.leading, SpectraLayout.Space.xl)
        }
        .padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }
}

// MARK: — Chain detail

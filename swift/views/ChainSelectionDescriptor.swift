import SwiftUI

/// One chain as the chain pickers draw it, projected once from its catalog row.
struct ChainSelectionDescriptor: Identifiable {
    let id: Chain
    let titleKey: String
    /// The chain's native symbol — one field, because the picker shows one.
    let symbol: String
    let artworkName: String?
    let color: Color
    /// In `listChainTags()` order, from core.
    let tags: [ChainTag]
    let searchKeywords: [String]
    var title: String { AppLocalization.string(titleKey) }
    var isTestnet: Bool { tags.contains(.testnet) }
    /// The tags as one line under the chain's name.
    var tagLine: String { tags.map(\.title).joined(separator: " · ") }

    init(chain: Chain, entry: ChainEntry) {
        self.id = chain
        self.titleKey = entry.name
        self.symbol = entry.gasTokenSymbol
        self.artworkName = entry.artworkName
        self.color = entry.color.color
        self.tags = entry.tags
        self.searchKeywords = entry.searchKeywords
    }

    /// `chains` in the catalog's popular order. A testnet shares its mainnet's
    /// rank, so catalog order breaks the tie and it follows its mainnet.
    static func popularOrder(_ chains: [Chain]) -> [ChainSelectionDescriptor] {
        chains.enumerated()
            .compactMap { index, chain in chain.entry.map { (index: index, entry: $0, chain: chain) } }
            .sorted { ($0.entry.popularRank, $0.index) < ($1.entry.popularRank, $1.index) }
            .map { ChainSelectionDescriptor(chain: $0.chain, entry: $0.entry) }
    }

    func matches(_ query: String) -> Bool {
        title.localizedCaseInsensitiveContains(query)
            || symbol.localizedCaseInsensitiveContains(query)
            || searchKeywords.contains { $0.localizedCaseInsensitiveContains(query) }
            || tags.contains { $0.title.localizedCaseInsensitiveContains(query) }
    }
}

/// How the chain picker orders its rows.
enum ChainPickerOrder: Hashable {
    /// The catalog's `popular_rank`.
    case popular
    /// By localized name.
    case name
}

/// Which rows the chain picker shows.
enum ChainPickerFilter: Hashable {
    case all
    case tag(ChainTag)

    var title: String {
        switch self {
        case .all: AppLocalization.string("All")
        case .tag(let tag): tag.title
        }
    }
}

extension [ChainSelectionDescriptor] {
    /// The rows a picker shows. Test networks appear while the picker shows
    /// them, each after its mainnet — or when a search finds nothing else: a
    /// test network asked for by name is the answer, not "No Results" for a
    /// network the list has.
    func picked(
        filter: ChainPickerFilter, query: String, order: ChainPickerOrder, showsTestNetworks: Bool
    ) -> [ChainSelectionDescriptor] {
        let rows = matching(filter: filter, query: query, showsTestNetworks: showsTestNetworks)
        let shown = rows.isEmpty && !showsTestNetworks && !query.isEmpty
            ? matching(filter: filter, query: query, showsTestNetworks: true) : rows
        switch order {
        case .popular: return shown
        case .name: return shown.sorted { $0.title.localizedStandardCompare($1.title) == .orderedAscending }
        }
    }

    private func matching(filter: ChainPickerFilter, query: String, showsTestNetworks: Bool) -> [ChainSelectionDescriptor] {
        self.filter { row in
            if row.isTestnet, !showsTestNetworks { return false }
            if !query.isEmpty, !row.matches(query) { return false }
            switch filter {
            case .all: return true
            case .tag(let tag): return row.tags.contains(tag)
            }
        }
    }
}

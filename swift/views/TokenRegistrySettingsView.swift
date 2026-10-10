import Foundation
import SwiftUI
struct TokenRegistrySettingsView: View {
    let tokens: TokenPreferencesState
    /// The chain filter; nil includes every chain.
    private static let chainFilterOptions: [Chain?] =
        [nil] + Chain.tokenHostingChains.map { Optional($0) }
    private enum TokenRegistrySourceFilter: CaseIterable, Identifiable {
        case all
        case builtIn
        case custom
        var id: Self { self }
        var title: String {
            switch self {
            case .all: return AppLocalization.string("All")
            case .builtIn: return AppLocalization.string("Built-In")
            case .custom: return AppLocalization.string("Custom")
            }
        }
    }
    @State private var searchText: String = ""
    @State private var chainFilter: Chain? = nil
    @State private var sourceFilter: TokenRegistrySourceFilter = .all
    var body: some View {
        let groups = filteredGroups
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollView(showsIndicators: false) {
                LazyVStack(spacing: SpectraLayout.sectionSpacing) {
                    if let error = tokens.error {
                        TokenPreferenceErrorNotice(message: error) { tokens.error = nil }
                    }
                    if !groups.isEmpty {
                        SpectraRowGroup(data: groups) { group in
                            NavigationLink {
                                TokenRegistryDetailView(tokens: tokens, groupKey: group.key)
                            } label: {
                                TokenRegistryGroupRowView(group: group)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .spectraScreenPadding()
            }
            .overlay { emptyState(isFilteredEmpty: groups.isEmpty) }
        }
        .navigationTitle(AppLocalization.string("Known Tokens"))
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, prompt: AppLocalization.string("Search name, symbol, chain, or address"))
        .textInputAutocapitalization(.never).autocorrectionDisabled()
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                filterMenu
            }
            ToolbarItem(placement: .topBarTrailing) {
                NavigationLink {
                    AddCustomTokenView(tokens: tokens)
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel(AppLocalization.string("New Token"))
            }
        }
    }
    @ViewBuilder
    private func emptyState(isFilteredEmpty: Bool) -> some View {
        if tokens.entries.isEmpty {
            ProgressView()
        } else if isFilteredEmpty {
            if searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                ContentUnavailableView(
                    AppLocalization.string("No matching tokens."),
                    systemImage: "line.3.horizontal.decrease.circle",
                    description: Text(AppLocalization.string("No known tokens match the selected filters.")))
            } else {
                ContentUnavailableView.search(text: searchText)
            }
        }
    }
    private var filterMenu: some View {
        Menu {
            Picker(AppLocalization.string("Network"), selection: $chainFilter) {
                ForEach(Self.chainFilterOptions, id: \.self) { chain in
                    Text(chain?.displayName ?? AppLocalization.string("All")).tag(chain)
                }
            }
            Picker(AppLocalization.string("Source"), selection: $sourceFilter) {
                ForEach(TokenRegistrySourceFilter.allCases) { filter in Text(filter.title).tag(filter) }
            }
            if chainFilter != nil || sourceFilter != .all {
                Button(AppLocalization.string("Clear Filters")) {
                    chainFilter = nil
                    sourceFilter = .all
                }
            }
        } label: {
            Image(systemName: chainFilter != nil || sourceFilter != .all
                ? "line.3.horizontal.decrease.circle.fill"
                : "line.3.horizontal.decrease.circle")
        }
        .accessibilityLabel(AppLocalization.string("Filters"))
    }
    private var filteredGroups: [TokenRegistryGroup] {
        // Core orders the list with one token's deployments together, so the
        // groups and their rows keep that order as they are.
        let allEntries = tokens.entries
        let grouped = Dictionary(grouping: allEntries, by: \.token.tokenId)
        var seen: Set<String> = []
        let groups = allEntries.map(\.token.tokenId).filter { seen.insert($0).inserted }
            .compactMap { key -> TokenRegistryGroup? in
                guard let entries = grouped[key], let representative = entries.first else { return nil }
                return TokenRegistryGroup(
                    key: key, name: representative.token.name, symbol: representative.token.symbol, entries: entries)
            }
        let filtered: [TokenRegistryGroup] = groups.filter { group in
            if let selectedChain = chainFilter, !group.entries.contains(where: { $0.token.chainId == selectedChain }) {
                return false
            }
            switch sourceFilter {
            case .all: break
            case .builtIn: guard group.entries.contains(where: { $0.isBuiltIn }) else { return false }
            case .custom: guard group.entries.contains(where: { !$0.isBuiltIn }) else { return false }
            }
            let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard !query.isEmpty else { return true }
            let haystack =
                ([group.symbol, group.name]
                + group.entries.flatMap { entry in
                    [entry.token.chainId.displayName,
                     entry.token.tokenStandard, entry.token.contract, entry.token.coingeckoId, entry.token.coinpaprikaId]
                })
                .joined(separator: " ").lowercased()
            return haystack.contains(query)
        }
        return filtered
    }
}

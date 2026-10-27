import SwiftUI

struct PinnedAssetsView: View {
    let store: AppState
    @State private var searchText: String = ""
    @State private var isConfirmingReset = false
    private var filteredOptions: [DashboardPinOption] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let allOptions = store.cachedAvailableDashboardPinOptions
        guard !query.isEmpty else { return allOptions }
        return allOptions.filter { option in
            option.symbol.localizedCaseInsensitiveContains(query) || option.name.localizedCaseInsensitiveContains(query)
                || option.subtitle.localizedCaseInsensitiveContains(query)
        }
    }
    var body: some View {
        List {
            Section {
                ForEach(filteredOptions) { option in
                    Toggle(isOn: binding(for: option)) {
                        DashboardPinnedAssetRowView(
                            option: option,
                            subtitleText: AppLocalization.format("dashboard.pinnedAsset.symbolSubtitle", option.symbol, option.subtitle)
                        ).equatable()
                    }
                }
            } header: {
                Text(AppLocalization.string("Pinned Assets"))
            } footer: {
                Text(AppLocalization.string("Pinned assets stay in the Assets list even when their balance is zero."))
            }
        }.navigationTitle(AppLocalization.string("Pinned Assets")).searchable(
            text: $searchText, prompt: AppLocalization.string("Search assets")
        ).toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Reset")) {
                    isConfirmingReset = true
                }
            }
        }.confirmationDialog(
            AppLocalization.string("Pin only the default assets again?"), isPresented: $isConfirmingReset,
            titleVisibility: .visible
        ) {
            Button(AppLocalization.string("Reset Pins"), role: .destructive) { store.resetPinnedDashboardAssets() }
        }
    }
    private func binding(for option: DashboardPinOption) -> Binding<Bool> {
        Binding(
            get: { option.isPinned }, set: { isPinned in store.setDashboardAssetPinned(isPinned, tokenId: option.tokenId) }
        )
    }
}

struct DashboardPinnedAssetRowView: View, Equatable {
    let option: DashboardPinOption
    let subtitleText: String
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.option == rhs.option && lhs.subtitleText == rhs.subtitleText }
    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(artworkName: option.artworkName, fallbackText: option.symbol, color: option.color, size: 34)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(option.name)
                Text(subtitleText).font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

extension DashboardPinOption: Identifiable {
    public var id: String { tokenId }
    var color: Color { AssetPresentationCatalog.color(deploymentId: deploymentId) }
}

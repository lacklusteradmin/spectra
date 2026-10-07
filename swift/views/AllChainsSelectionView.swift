import SwiftUI

/// Every chain in one list, ordered by popularity or name and narrowed by a
/// tag filter and a search. Test networks join the list behind a switch.
///
/// Self-contained — takes its dependencies as bindings/closures and doesn't
/// reach into AppState. Callers pass the descriptors in popular order, the
/// chosen chain, if any, what picking one does, and optionally a header above
/// the list.
struct AllChainsSelectionView<Header: View>: View {
    @Binding var chainSearchText: String
    let title: String
    let descriptors: [ChainSelectionDescriptor]
    let selectedChains: Set<Chain>
    /// A navigation list marks rows with a chevron; a choice marks the
    /// chosen row.
    var accessory: ChainSelectionRow.Accessory = .checkmark
    let toggleSelection: (Chain) -> Void
    @ViewBuilder var header: () -> Header
    @State private var order: ChainPickerOrder = .popular
    @State private var filter: ChainPickerFilter = .all
    @State private var showsTestNetworks = false
    @State private var isShowingInfo = false
    private var trimmedQuery: String { chainSearchText.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var rows: [ChainSelectionDescriptor] {
        descriptors.picked(filter: filter, query: trimmedQuery, order: order, showsTestNetworks: showsTestNetworks)
    }
    /// "All", then every tag some row carries. Test networks are the switch's,
    /// not a filter's.
    private var filters: [ChainPickerFilter] {
        let tags = ChainTag.pickerOrder.filter { tag in
            tag != .testnet && descriptors.contains { $0.tags.contains(tag) }
        }
        return [.all] + tags.map { .tag($0) }
    }
    private var testNetworkSwitch: some View {
        Toggle(isOn: $showsTestNetworks) {
            Label(AppLocalization.string("Show test networks"), systemImage: "testtube.2")
                .font(.body.weight(.semibold))
        }
        .spectraRowPadding()
        .spectraCardFill()
    }
    private var filterBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            GlassEffectContainer(spacing: SpectraLayout.Space.s) {
                HStack(spacing: SpectraLayout.Space.s) {
                    ForEach(filters, id: \.self) { item in filterChip(item) }
                }
                .padding(.horizontal, SpectraLayout.screenHorizontal)
                .padding(.vertical, SpectraLayout.Space.xs)
            }
        }
        .scrollClipDisabled()
    }
    @ViewBuilder
    private func filterChip(_ item: ChainPickerFilter) -> some View {
        let label = Text(item.title).font(.subheadline.weight(.semibold))
        if filter == item {
            Button { filter = item } label: { label }.buttonStyle(.glassProminent)
        } else {
            Button { filter = item } label: { label }.buttonStyle(.glass)
        }
    }
    @ViewBuilder
    private var list: some View {
        if !rows.isEmpty {
            SpectraRowGroup(data: rows) { descriptor in
                ChainSelectionRow(
                    descriptor: descriptor, isSelected: selectedChains.contains(descriptor.id), accessory: accessory
                ) { toggleSelection(descriptor.id) }
            }
        } else if !trimmedQuery.isEmpty {
            ContentUnavailableView.search(text: trimmedQuery)
        } else {
            ContentUnavailableView(
                AppLocalization.string("import_flow.no_chains_selected"), systemImage: "checkmark.circle")
        }
    }
    private var toolbarMenu: some View {
        Menu {
            Picker(AppLocalization.string("Sort"), selection: $order) {
                Label(AppLocalization.string("Popular"), systemImage: "flame").tag(ChainPickerOrder.popular)
                Label(AppLocalization.string("Name"), systemImage: "textformat").tag(ChainPickerOrder.name)
            }
        } label: {
            Image(systemName: "arrow.up.arrow.down")
        }
        .accessibilityLabel(AppLocalization.string("Sort"))
    }
    @ViewBuilder
    private var gasTokenInfoSheet: some View {
        NavigationStack {
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.l) {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                        Label(AppLocalization.string("Gas Token"), systemImage: "fuelpump.fill")
                            .font(.headline)
                            .foregroundStyle(.tint)
                        Text(
                            AppLocalization.string(
                                "The symbol shown on the right of each chain is its gas token — the asset you need to pay transaction fees."
                            )
                        )
                        .font(.subheadline).foregroundStyle(.secondary)
                    }
                    .padding(SpectraLayout.Space.l)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: SpectraLayout.Radius.inner, style: .continuous).fill(Color.accentColor.opacity(0.08))
                    )
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                        Label(AppLocalization.string("L2s and Native Tokens"), systemImage: "square.stack.3d.up.fill")
                            .font(.headline)
                            .foregroundStyle(.tint)
                        Text(
                            AppLocalization.string(
                                "Some L2 chains have a separate native token (e.g. ARB, OP) but use a different asset for gas fees (e.g. ETH). Spectra shows the gas token since that's what you'll need to keep funded for transactions."
                            )
                        )
                        .font(.subheadline).foregroundStyle(.secondary)
                    }
                    .padding(SpectraLayout.Space.l)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: SpectraLayout.Radius.inner, style: .continuous).fill(Color.accentColor.opacity(0.08))
                    )
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                        Label(AppLocalization.string("Missing a Chain?"), systemImage: "plus.circle.fill")
                            .font(.headline)
                            .foregroundStyle(.tint)
                        Text(
                            AppLocalization.string(
                                "If you'd like a chain added, go to Settings → Report a Problem and let the developer know. New chains are added regularly."
                            )
                        )
                        .font(.subheadline).foregroundStyle(.secondary)
                    }
                    .padding(SpectraLayout.Space.l)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: SpectraLayout.Radius.inner, style: .continuous).fill(Color.accentColor.opacity(0.08))
                    )
                }
                .padding(SpectraLayout.Space.l)
            }
            .navigationTitle(AppLocalization.string("Chain Info"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(AppLocalization.string("Done")) { isShowingInfo = false }
                        .buttonStyle(.borderedProminent)
                }
            }
        }
    }
    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    header()
                    filterBar.padding(.horizontal, -SpectraLayout.screenHorizontal)
                    testNetworkSwitch
                    list
                }
                .spectraScreenPadding()
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .searchable(
            text: $chainSearchText, placement: .navigationBarDrawer(displayMode: .always),
            prompt: AppLocalization.string("import_flow.search_chains")
        )
        .textInputAutocapitalization(.never).autocorrectionDisabled()
        .sensoryFeedback(.selection, trigger: filter)
        .sensoryFeedback(.selection, trigger: order)
        .sensoryFeedback(.selection, trigger: showsTestNetworks)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) { toolbarMenu }
            ToolbarItem(placement: .topBarTrailing) {
                Button { isShowingInfo = true } label: {
                    Image(systemName: "info.circle")
                }
                .accessibilityLabel(AppLocalization.string("Chain Info"))
            }
        }
        .sheet(isPresented: $isShowingInfo) { gasTokenInfoSheet }
    }
}

extension AllChainsSelectionView where Header == EmptyView {
    init(
        chainSearchText: Binding<String>, title: String, descriptors: [ChainSelectionDescriptor],
        selectedChains: Set<Chain>, toggleSelection: @escaping (Chain) -> Void
    ) {
        self.init(
            chainSearchText: chainSearchText, title: title, descriptors: descriptors, selectedChains: selectedChains,
            toggleSelection: toggleSelection, header: { EmptyView() })
    }
}

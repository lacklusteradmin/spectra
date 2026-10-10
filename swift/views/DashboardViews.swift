import SwiftUI
struct DashboardView: View {
    @Bindable var store: AppState
    @State private var dashboardPage: DashboardPage = .assets
    @State private var isNavigatingToPinnedAssets = false
    @State private var selectedWalletId: String?
    @State private var selectedAssetGroupId: String?
    @ScaledMetric(relativeTo: .caption2) private var torBadgeFontSize: CGFloat = 8
    private var selectedWallet: WalletView? {
        guard let selectedWalletId else { return nil }
        return store.wallet(for: selectedWalletId)
    }
    var body: some View {
        NavigationStack {
            ZStack {
                SpectraBackdrop().ignoresSafeArea()
                ScrollView(showsIndicators: false) {
                    VStack(spacing: SpectraLayout.sectionSpacing) {
                        // Before the first wallet a total, Send and Receive
                        // have nothing to act on, so the page is one card that
                        // leads to adding one.
                        if store.wallets.isEmpty {
                            DashboardWelcomeCard(store: store)
                        } else {
                            portfolioHeader
                            actionButtons
                            assetsOrWalletsCard
                        }
                    }.spectraScreenPadding()
                }.refreshable {
                    await store.performUserInitiatedRefresh()
                }.scrollBounceBehavior(.always)
            }
            .navigationTitle(AppLocalization.string("Spectra")).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    NavigationLink {
                        AppNoticesView(store: store)
                    } label: {
                        noticeToolbarLabel
                    }
                }
                // Tor is off by default and lives in Settings; the toolbar
                // reports it only while it is doing something.
                if store.tor.status != .stopped {
                    ToolbarItem(placement: .topBarLeading) {
                        NavigationLink {
                            TorSettingsView(store: store)
                        } label: {
                            torToolbarIndicator
                        }
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        spectraHaptic(.light)
                        store.isShowingAddWalletEntry = true
                    } label: {
                        Image(systemName: "plus")
                    }.accessibilityLabel(AppLocalization.string("Add Wallet"))
                }
            }.navigationDestination(isPresented: $store.isShowingAddWalletEntry) {
                AddWalletEntryView(store: store)
            }.navigationDestination(item: $selectedWalletId) { walletId in
                if let wallet = store.wallet(for: walletId) {
                    WalletDetailView(store: store, wallet: wallet)
                }
            }
            // A wallet deleted under its page — from Manage Wallet, which it
            // pushed — leaves nothing to show: the page and all it pushed
            // close, rather than standing blank until Back.
            .onChange(of: selectedWallet == nil) { _, isGone in
                if isGone, selectedWalletId != nil { selectedWalletId = nil }
            }.navigationDestination(item: $selectedAssetGroupId) { id in AssetGroupDetailView(store: store, assetGroupId: id) }
                .navigationDestination(isPresented: store.sendFlowBinding(on: .home)) {
                    SendView(store: store)
                }.navigationDestination(
                    isPresented: Bindable(store.receiveFlow).isPresented
                ) {
                    ReceiveView(store: store)
                }.navigationDestination(isPresented: $isNavigatingToPinnedAssets) {
                    PinnedAssetsView(store: store)
                }
        }
    }
    private var portfolioHeader: some View {
        DashboardPortfolioHeader(store: store)
    }
    private var noticeToolbarLabel: some View {
        let notices = activeNotices
        let count = notices.count
        let isEmpty = notices.isEmpty
        return ZStack(alignment: .topTrailing) {
            Image(systemName: isEmpty ? "tray" : "exclamationmark.bubble").font(.system(size: 18, weight: .semibold)).frame(
                width: 24, height: 24)
            if !isEmpty {
                // Red only when something failed; warnings take the warning colour.
                let tint = notices.contains { $0.severity == .error } ? Color.red : Color.spectraWarning
                Text(verbatim: count > 9 ? "9+" : "\(count)").font(.caption2.weight(.bold)).foregroundStyle(.white)
                    .padding(.horizontal, SpectraLayout.Space.xs).padding(.vertical, SpectraLayout.Space.xxs)
                    .background(Capsule().fill(tint)).offset(x: 6, y: -5)
            }
        }.frame(width: 32, height: 28, alignment: .center).foregroundStyle(Color.primary).accessibilityLabel(
            isEmpty
                ? AppLocalization.string("No active notices")
                : AppLocalization.format("%lld active notices", count: count, count)
        )
    }
    private var torToolbarIndicator: some View {
        let status = store.tor.status
        let color: Color
        let icon: String
        switch status {
        case .ready:
            color = .green; icon = "network.badge.shield.half.filled"
        case .bootstrapping:
            color = .spectraWarning; icon = "network.badge.shield.half.filled"
        case .error:
            color = .red; icon = "network.slash"
        case .stopped:
            color = Color(.systemGray3); icon = "network"
        }
        return ZStack(alignment: .bottomTrailing) {
            Image(systemName: icon)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(color)
                .frame(width: 24, height: 24)
            if case .bootstrapping(let pct) = status {
                Text("\(pct)%")
                    .font(.system(size: torBadgeFontSize, weight: .bold))
                    .monospacedDigit()
                    .foregroundStyle(.white)
                    .padding(.horizontal, SpectraLayout.Space.xxs)
                    .padding(.vertical, SpectraLayout.Space.xxs)
                    .background(Capsule().fill(Color.spectraWarning))
                    .offset(x: 6, y: 4)
            } else if case .error = status {
                Circle().fill(color).frame(width: 7, height: 7).offset(x: 3, y: 3)
            }
        }
        .frame(width: 30, height: 28, alignment: .center)
        .accessibilityLabel(torAccessibilityLabel(status))
    }
    private func torAccessibilityLabel(_ status: TorStatus) -> String {
        switch status {
        case .stopped:          return AppLocalization.string("Tor off")
        case .bootstrapping(let p): return AppLocalization.format("Tor connecting, %lld%%", Int(p))
        case .ready:            return AppLocalization.string("Tor on")
        case .error:            return AppLocalization.string("Tor error")
        }
    }
    private var actionButtons: some View {
        DashboardActionButtons(store: store)
    }
    /// The card's header is its page switch: a title per page, each with its
    /// count, and the selected page's own action at the end. A segmented
    /// control above the card said the same thing a second time, a row away
    /// from the list it switched, and its action lived in the toolbar, which
    /// changed shape whenever the page did.
    @ViewBuilder
    private var assetsOrWalletsCard: some View {
        switch dashboardPage {
        case .assets:
            let rows = shownAssetRows
            SpectraRowGroup(data: rows, header: { pageSwitch(assetCount: rows.count) }, footer: { assetsFooter(shown: rows) }) { row in
                Button { selectedAssetGroupId = row.id } label: {
                    DashboardAssetRowView(presentation: row).equatable().spectraRowPadding()
                }.buttonStyle(.plain)
            }
        case .wallets:
            SpectraRowGroup(data: store.wallets, header: { pageSwitch(assetCount: shownAssetRows.count) }, footer: { EmptyView() }) { wallet in
                Button { selectedWalletId = wallet.id } label: {
                    WalletCardView(presentation: .init(wallet: wallet, store: store))
                        .equatable().spectraRowPadding()
                }.buttonStyle(.plain)
            }
        }
    }
    private func pageSwitch(assetCount: Int) -> some View {
        HStack(spacing: SpectraLayout.Space.l) {
            dashboardPageTab(.assets, title: "Assets", count: assetCount)
            dashboardPageTab(.wallets, title: "Wallets", count: store.wallets.count)
            Spacer(minLength: 0)
            if dashboardPage == .assets { pinAssetsButton }
        }
        .padding(.horizontal, SpectraLayout.rowHorizontal)
        .sensoryFeedback(.selection, trigger: dashboardPage)
    }
    private func dashboardPageTab(_ page: DashboardPage, title: String, count: Int) -> some View {
        let isSelected = dashboardPage == page
        return Button {
            dashboardPage = page
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.xs) {
                Text(AppLocalization.string(title)).font(.headline)
                    .foregroundStyle(isSelected ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                Text("\(count)").font(.subheadline.weight(.semibold)).monospacedDigit()
                    .foregroundStyle(isSelected ? AnyShapeStyle(.secondary) : AnyShapeStyle(.tertiary))
            }
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }
    /// What sits under the asset rows: placeholders while the only wallets
    /// are still reading, a note while some are, the folded small balances,
    /// or why there is nothing to list.
    @ViewBuilder
    private func assetsFooter(shown: [DashboardAssetRowPresentation]) -> some View {
        let reading = !store.portfolioWalletsReadingBalances.isEmpty
        let smallCount = store.cachedDashboardAssetGroups.filter(\.isSmall).count
        if shown.isEmpty && reading {
            VStack(spacing: SpectraLayout.Space.m) {
                ForEach(0..<3, id: \.self) { _ in
                    HStack(spacing: SpectraLayout.Space.m) {
                        Circle().fill(SpectraLayout.insetFill).frame(width: 36, height: 36)
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                            SpectraShimmer(height: 14).frame(maxWidth: 140)
                            SpectraShimmer(height: 10).frame(maxWidth: 90)
                        }
                        Spacer(minLength: 0)
                        SpectraShimmer(height: 14).frame(width: 70)
                    }
                }
            }
            .padding(.horizontal, SpectraLayout.rowHorizontal).padding(.vertical, SpectraLayout.Space.m)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(AppLocalization.string("Reading balances…"))
        } else if shown.isEmpty && smallCount == 0 {
            SpectraEmptyStateContent(
                title: "No balances yet", message: "Pull down to read balances again.", systemImage: "chart.pie")
                .padding(.horizontal, SpectraLayout.rowHorizontal).padding(.vertical, SpectraLayout.Space.m)
        } else if reading {
            Divider().opacity(0.25)
            HStack(spacing: SpectraLayout.Space.s) {
                SpectraLoadingGlyph(size: 18)
                Text(AppLocalization.string("Reading balances…")).font(.caption).foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }.spectraRowPadding()
        }
        if smallCount > 0 {
            Divider().opacity(0.25)
            Button {
                spectraHaptic(.light)
                store.preferences.hideSmallBalances.toggle()
            } label: {
                HStack(spacing: SpectraLayout.Space.s) {
                    Text(store.preferences.hideSmallBalances
                        ? AppLocalization.format("Show %lld small balances", count: smallCount, smallCount)
                        : AppLocalization.string("Hide small balances"))
                        .font(.subheadline.weight(.semibold)).foregroundStyle(.tint)
                    Spacer(minLength: 0)
                    Image(systemName: store.preferences.hideSmallBalances ? "chevron.down" : "chevron.up")
                        .font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                }.spectraRowPadding()
            }.buttonStyle(.plain)
        }
    }
    /// The rows Home lists: core's groups, less the small ones while they
    /// are folded away.
    private var shownAssetRows: [DashboardAssetRowPresentation] {
        let hidesSmall = store.preferences.hideSmallBalances
        return store.cachedDashboardAssetGroups
            .filter { !(hidesSmall && $0.isSmall) }
            .map { DashboardAssetRowPresentation(assetGroup: $0, amounts: store.amounts, hidesBalance: store.preferences.hideBalances) }
    }
    private var activeNotices: [AppNoticeItem] { store.appNoticeItems }
    // Pinning is available only on the assets page.
    private var pinAssetsButton: some View {
        Button {
            spectraHaptic(.light)
            isNavigatingToPinnedAssets = true
        } label: {
            Image(systemName: "pin").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                .frame(width: 44, height: 44).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.trailing, -SpectraLayout.Space.m)
        .accessibilityLabel(AppLocalization.string("Pin Assets"))
    }
}
enum DashboardPage {
    case wallets
    case assets
}
enum AppNoticeSeverity {
    case warning
    case error
    var tint: Color {
        switch self {
        case .warning: return .spectraWarning
        case .error: return .red
        }
    }
    var label: String {
        switch self {
        case .warning: return AppLocalization.string("Warning")
        case .error: return AppLocalization.string("Error")
        }
    }
}
struct AppNoticeItem: Identifiable {
    let id = UUID()
    let title: String
    let message: String
    let severity: AppNoticeSeverity
    let systemImage: String
    var timestamp: Date? = nil
    /// What can be done about it from the notices page, if anything.
    var action: AppNoticeAction? = nil
}
/// A notice's way forward: ask again, go where it is fixed, or put it away.
enum AppNoticeAction {
    /// Run the read that failed again.
    case retry(@MainActor () async -> Void)
    /// Where a network's endpoints are chosen.
    case openEndpoints
    /// Seen, with nothing else to do.
    case dismiss(@MainActor () -> Void)
}
extension DashboardAssetGroup: Identifiable {
    /// Core supplies the asset's display identity from its primary holding,
    /// or from the catalog when a pinned asset is not held.
    var name: String { identity.name }
    var symbol: String { identity.symbol }
    var artworkName: String { identity.artworkName }
    var color: Color { identity.color }
}

extension DashboardPinOption: Identifiable {
    public var id: String { tokenId }
    var color: Color { AssetPresentationCatalog.color(deploymentId: deploymentId) }
}
/// One asset across every wallet: what it is worth, where it is held, how to
/// move it, and where it lives. Read from core's current groups by id, so a
/// refresh lands here as it lands on Home.
struct AssetGroupDetailView: View {
    let store: AppState
    let assetGroupId: String
    private var assetGroup: DashboardAssetGroup? {
        store.cachedDashboardAssetGroups.first { $0.id == assetGroupId }
    }
    /// Where the coin lives, from core's asset wiki — the same join the wiki
    /// screen renders, rather than a second dashboard-only cache of it.
    private var places: [AssetWikiPlace] {
        CoreReferenceTables.assetWikiEntry(tokenId: assetGroupId)?.livesOn ?? []
    }
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                if let assetGroup {
                    AssetDetailHeroCard(assetGroup: assetGroup, store: store)
                    AssetMoveButtons(store: store, assetGroup: assetGroup)
                    AssetChainBreakdownCard(assetGroup: assetGroup, store: store)
                    if !places.isEmpty { AssetPlacesCard(places: places, symbol: assetGroup.symbol) }
                } else {
                    SpectraEmptyStateCard(
                        title: "Asset not listed", message: "This asset is no longer in your portfolio.",
                        systemImage: "chart.pie")
                }
            }.spectraScreenPadding()
        }.background(SpectraBackdrop().ignoresSafeArea())
            .navigationTitle(assetGroup?.symbol ?? "").navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .refreshable { await store.performUserInitiatedRefresh() }
    }
}
/// Send and Receive for this asset, from the wallets that can: Send opens on
/// a wallet holding it, Receive on one whose network it lives on.
private struct AssetMoveButtons: View {
    let store: AppState
    let assetGroup: DashboardAssetGroup
    private var holdingKeys: Set<String> { Set(assetGroup.holdings.map(\.coin.holdingKey)) }
    private var sendTarget: (walletId: String, holdingKey: String)? {
        for wallet in store.sendEnabledWallets {
            if let coin = store.availableSendCoins(for: wallet.id).first(where: { holdingKeys.contains($0.holdingKey) }) {
                return (wallet.id, coin.holdingKey)
            }
        }
        return nil
    }
    private var receiveWalletId: String? {
        let chains = Set(assetGroup.holdings.map(\.coin.chainId) + [assetGroup.identity.chainId])
        return store.receiveEnabledWallets.first { chains.contains($0.chainId) }?.id
    }
    var body: some View {
        let send = sendTarget
        let receive = receiveWalletId
        GlassEffectContainer(spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                Button {
                    spectraHaptic(.medium)
                    if let send { store.beginSend(walletId: send.walletId, holdingKey: send.holdingKey) }
                } label: {
                    Label(AppLocalization.string("Send"), systemImage: "arrow.up.right")
                        .font(.body.weight(.semibold)).frame(maxWidth: .infinity).padding(.vertical, SpectraLayout.Space.m)
                }.buttonStyle(.glass).disabled(send == nil)
                Button {
                    spectraHaptic(.medium)
                    if let receive { store.beginReceive(walletId: receive) }
                } label: {
                    Label(AppLocalization.string("Receive"), systemImage: "arrow.down.left")
                        .font(.body.weight(.semibold)).frame(maxWidth: .infinity).padding(.vertical, SpectraLayout.Space.m)
                }.buttonStyle(.glass).tint(.accentColor).disabled(receive == nil)
            }
        }
    }
}
private struct AssetDetailHeroCard: View {
    let assetGroup: DashboardAssetGroup
    let store: AppState
    var body: some View {
        let hidden = store.preferences.hideBalances
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: assetGroup.artworkName, fallbackText: assetGroup.symbol,
                color: assetGroup.color, size: 60
            )
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                Text(assetGroup.name).font(.title2.weight(.bold)).foregroundStyle(Color.primary)
                    .fixedSize(horizontal: false, vertical: true)
                BalanceText(text: store.amounts.formattedFiat(assetGroup.totalValue), isHidden: hidden)
                    .font(.title3.weight(.semibold)).foregroundStyle(Color.primary)
                    .spectraNumericTextLayout(minimumScaleFactor: 0.7)
                BalanceText(
                    text: store.amounts.formattedAssetAmount(
                        assetGroup.totalAmount, symbol: assetGroup.symbol, deploymentId: assetGroup.identity.holdingKey),
                    isHidden: hidden
                ).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                    .spectraNumericTextLayout(minimumScaleFactor: 0.7)
                Text(AppLocalization.format("dashboard.asset.price", store.amounts.formattedFiat(assetGroup.price), assetGroup.symbol))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraElevatedFill()
    }
}
/// Every place the row's asset is held, largest first.
private struct AssetChainBreakdownCard: View {
    let assetGroup: DashboardAssetGroup
    let store: AppState

    /// Held nowhere, so there is no breakdown to draw.
    ///
    /// A pinned asset the user holds none of has no holdings at all — core
    /// names the row from its `identity` instead of synthesizing a place. The
    /// amount is checked too, because a real holding that has been emptied says
    /// the same thing to the reader and deserves the same sentence.
    private var holdsNothing: Bool { assetGroup.holdings.isEmpty || assetGroup.totalAmount == "0" }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("Chain Breakdown")).font(.headline).foregroundStyle(Color.primary)
            if holdsNothing {
                Text(AppLocalization.string("No balance on any chain."))
                    .font(.subheadline).foregroundStyle(.secondary)
            } else {
                ForEach(Array(assetGroup.holdings.enumerated()), id: \.offset) { index, holding in
                    AssetChainBreakdownRow(
                        chain: holding.coin.chain,
                        chainTitle: holding.coin.chainName,
                        tokenStandard: holding.coin.tokenStandard,
                        amountText: store.amounts.formattedAssetAmount(
                            holding.coin.amount, symbol: holding.coin.symbol,
                            deploymentId: holding.coin.holdingKey),
                        valueText: store.amounts.formattedFiat(holding.value),
                        hidesBalance: store.preferences.hideBalances,
                        fallbackColor: holding.coin.color
                    )
                    if index < assetGroup.holdings.count - 1 { Divider().opacity(0.4) }
                }
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraCardFill()
    }
}

/// One chain the asset is held on.
///
/// The badge is the **chain's**, which is what the row is about: USDC on
/// Ethereum and USDC on Solana must not look alike. `fallbackColor` is the
/// asset's, for a chain the registry has no artwork for.
private struct AssetChainBreakdownRow: View {
    let chain: Chain?
    let chainTitle: String
    let tokenStandard: String
    let amountText: String
    let valueText: String
    let hidesBalance: Bool
    let fallbackColor: Color
    var body: some View {
        let badge = AssetHolding.nativeChainBadge(for: chain) ?? (nil, fallbackColor)
        return HStack(alignment: .center, spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: badge.artworkName,
                fallbackText: chainTitle, color: badge.color, size: 30)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(chainTitle).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary).lineLimit(1)
                Text(tokenStandard).font(.caption2).foregroundStyle(.secondary)
            }
            Spacer(minLength: SpectraLayout.Space.m)
            VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                BalanceText(text: amountText, isHidden: hidesBalance)
                    .font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary).spectraNumericTextLayout()
                BalanceText(text: valueText, isHidden: hidesBalance)
                    .font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
            }
        }
    }
}
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
struct PortfolioWalletSelectionView: View {
    let store: AppState
    var body: some View {
        List {
            Section {
                ForEach(store.wallets) { wallet in
                    Toggle(isOn: binding(for: wallet.id)) {
                        PortfolioWalletToggleRowView(walletName: wallet.name, chainTitleText: wallet.networkTitle)
                            .equatable()
                    }
                }
            } header: {
                Text(AppLocalization.string("Included In Portfolio Total"))
            } footer: {
                Text(
                    AppLocalization.string(
                        "Only selected wallets contribute to the portfolio total and the aggregated asset list on the home page."))
            }
        }.navigationTitle(AppLocalization.string("Portfolio Wallets"))
    }
    private func binding(for walletId: String) -> Binding<Bool> {
        Binding(
            get: {
                store.wallet(for: walletId)?.includeInPortfolioTotal ?? true
            }, set: { isIncluded in store.setPortfolioInclusion(isIncluded, for: walletId) }
        )
    }
}
struct AppNoticesView: View {
    let store: AppState
    var body: some View {
        let notices = store.appNoticeItems
        return List {
            if notices.isEmpty {
                Section {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                        Text(AppLocalization.string("No active notices")).font(.headline)
                        Text(AppLocalization.string("Current wallet, pricing, and chain-state warnings will appear here.")).font(
                            .subheadline
                        ).foregroundStyle(.secondary)
                    }.padding(.vertical, SpectraLayout.Space.xs)
                }
            } else {
                Section(AppLocalization.string("Active Notices")) {
                    ForEach(notices) { notice in noticeRow(notice) }
                }
            }
        }.navigationTitle(AppLocalization.string("Notices"))
    }

    /// A notice with its way forward: a row that opens where it is fixed, or
    /// a button under it. Borderless, so the row does not fire it.
    @ViewBuilder
    private func noticeRow(_ notice: AppNoticeItem) -> some View {
        switch notice.action {
        case .openEndpoints:
            NavigationLink { EndpointCatalogSettingsView(store: store) } label: {
                DashboardNoticeCardView(notice: notice)
            }
        case .retry(let retry):
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                DashboardNoticeCardView(notice: notice)
                Button(AppLocalization.string("Retry"), systemImage: "arrow.clockwise") { Task { await retry() } }
                    .buttonStyle(.borderless).font(.subheadline.weight(.semibold))
            }
        case .dismiss(let dismiss):
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                DashboardNoticeCardView(notice: notice)
                Button(AppLocalization.string("Dismiss"), systemImage: "xmark") { dismiss() }
                    .buttonStyle(.borderless).font(.subheadline.weight(.semibold))
            }
        case nil:
            DashboardNoticeCardView(notice: notice)
        }
    }
}
struct DashboardAssetRowPresentation: Identifiable, Equatable {
    let assetGroup: DashboardAssetGroup
    let amountText: String
    let totalValueText: String
    /// A price is public: it shows with the balances hidden. `nil` for an
    /// asset with no price, whose "—" value already says so once.
    let priceText: String?
    let hidesBalance: Bool
    var id: String { assetGroup.id }

    @MainActor
    init(assetGroup: DashboardAssetGroup, amounts: AmountPresentation, hidesBalance: Bool) {
        self.assetGroup = assetGroup
        amountText = amounts.formattedAssetAmount(
            assetGroup.totalAmount, symbol: assetGroup.symbol, deploymentId: assetGroup.identity.holdingKey)
        totalValueText = amounts.formattedFiat(assetGroup.totalValue)
        priceText = amounts.formattedFiatIfAvailable(assetGroup.price)
        self.hidesBalance = hidesBalance
    }
}
struct DashboardAssetRowView: View, Equatable {
    let presentation: DashboardAssetRowPresentation
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.presentation == rhs.presentation }
    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: presentation.assetGroup.artworkName, fallbackText: presentation.assetGroup.symbol,
                color: presentation.assetGroup.color, size: 36
            )
            // Side by side while it fits; at accessibility sizes the figures
            // go under the name rather than squeeze it to a few letters.
            if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    nameLine
                    BalanceText(text: presentation.amountText, isHidden: presentation.hidesBalance)
                        .font(.caption).foregroundStyle(.secondary)
                    BalanceText(text: presentation.totalValueText, isHidden: presentation.hidesBalance)
                        .font(.headline).foregroundStyle(Color.primary)
                    if let priceText = presentation.priceText {
                        Text(priceText).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 0)
            } else {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    nameLine
                    BalanceText(text: presentation.amountText, isHidden: presentation.hidesBalance)
                        .font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
                }
                Spacer(minLength: SpectraLayout.Space.s)
                VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                    BalanceText(text: presentation.totalValueText, isHidden: presentation.hidesBalance)
                        .font(.headline).foregroundStyle(Color.primary).spectraNumericTextLayout()
                    if let priceText = presentation.priceText {
                        Text(priceText).font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
                    }
                }
            }
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
    }
    /// The asset's whole name, on two lines when one is not enough.
    private var nameLine: some View {
        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.xs) {
            if presentation.assetGroup.isPinned {
                Image(systemName: "pin.fill").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                    .accessibilityLabel(AppLocalization.string("Pinned"))
            }
            Text(presentation.assetGroup.name).font(.headline).foregroundStyle(Color.primary)
                .lineLimit(2).fixedSize(horizontal: false, vertical: true)
        }
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
struct PortfolioWalletToggleRowView: View, Equatable {
    let walletName: String
    let chainTitleText: String
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.walletName == rhs.walletName && lhs.chainTitleText == rhs.chainTitleText }
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
            Text(walletName)
            Text(chainTitleText).font(.caption).foregroundStyle(.secondary)
        }
    }
}
struct DashboardNoticeCardView: View {
    let notice: AppNoticeItem
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: notice.systemImage).foregroundStyle(notice.severity.tint)
                Text(notice.title).font(.headline)
                Spacer()
                Text(notice.severity.label).font(.caption.weight(.semibold)).foregroundStyle(notice.severity.tint)
            }
            Text(notice.message).font(.subheadline).foregroundStyle(.primary)
            if let timestamp = notice.timestamp {
                Text(
                    AppLocalization.format(
                        "Last known healthy sync: %@", timestamp.appFormatted(time: .shortened))
                ).font(.caption).foregroundStyle(.secondary)
            }
        }.padding(.vertical, SpectraLayout.Space.xs)
    }
}

// ── Dashboard top-level sections ────────────────────────────────────────
// Each section is a standalone `View` struct so its internal TupleView
// types don't cascade into `DashboardView.body`'s opaque return. This
// matches the SetupView refactor and Apple's preferred pattern of many
// focused `View` structs rather than long computed-var bodies.

private struct DashboardPortfolioHeader: View {
    @Bindable var store: AppState
    var body: some View {
        let hidden = store.preferences.hideBalances
        let reading = !store.portfolioWalletsReadingBalances.isEmpty
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack(spacing: SpectraLayout.Space.xs) {
                Text(AppLocalization.string("Portfolio")).font(.subheadline).foregroundStyle(.secondary)
                Spacer(minLength: 0)
                Button {
                    spectraHaptic(.light)
                    store.preferences.hideBalances.toggle()
                } label: {
                    Image(systemName: hidden ? "eye.slash" : "eye").font(.subheadline.weight(.semibold))
                        .frame(width: 44, height: 44).contentShape(Rectangle())
                }
                .buttonStyle(.plain).foregroundStyle(.secondary)
                .accessibilityLabel(AppLocalization.string(hidden ? "Show Balances" : "Hide Balances"))
                NavigationLink {
                    PortfolioWalletSelectionView(store: store)
                } label: {
                    Image(systemName: "slider.horizontal.3").font(.subheadline.weight(.semibold))
                        .frame(width: 44, height: 44).contentShape(Rectangle())
                }
                .buttonStyle(.plain).foregroundStyle(.secondary)
                .accessibilityLabel(AppLocalization.string("Portfolio Wallets"))
                .accessibilityHint(AppLocalization.string("Choose which wallets count in the total."))
            }
            .padding(.vertical, -SpectraLayout.Space.m)
            if store.portfolioTotalIsUnknown {
                SpectraShimmer(height: 34).frame(maxWidth: 200)
                    .accessibilityLabel(AppLocalization.string("Reading balances…"))
            } else {
                BalanceText(text: store.amounts.formattedQuotedTotal(store.portfolioQuotedTotal), isHidden: hidden)
                    .font(.title.weight(.bold)).foregroundStyle(Color.primary)
                    .lineLimit(1).minimumScaleFactor(0.5).allowsTightening(true)
            }
            if reading {
                HStack(spacing: SpectraLayout.Space.xs) {
                    SpectraLoadingGlyph(size: 14)
                    Text(AppLocalization.string("Reading balances…")).font(.caption).foregroundStyle(.secondary)
                }
            }
            // A total that leaves wallets or unpriced assets out says so,
            // under the figure rather than in it.
            let counted = store.wallets.filter(\.includeInPortfolioTotal).count
            if counted < store.wallets.count {
                Text(AppLocalization.format("portfolio.counted_format", count: store.wallets.count, counted, store.wallets.count))
                    .font(.caption).foregroundStyle(.secondary)
            }
            if !store.portfolioTotalIsUnknown, let total = store.portfolioQuotedTotal {
                if total.unpricedCount > 0 {
                    Text(AppLocalization.format(
                        "portfolio.unpriced_format", count: Int(total.unpricedCount), Int(total.unpricedCount)))
                        .font(.caption).foregroundStyle(.secondary)
                }
                if total.testNetworkCount > 0 {
                    Text(AppLocalization.string("Test network assets are not counted."))
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .padding(SpectraLayout.cardPadding).frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }
}

private struct DashboardWelcomeCard: View {
    @Bindable var store: AppState
    var body: some View {
        VStack(spacing: SpectraLayout.Space.l) {
            SpectraLogo(size: 72)
            VStack(spacing: SpectraLayout.Space.xs) {
                Text(AppLocalization.string("Welcome to Spectra")).font(.title2.weight(.bold))
                Text(AppLocalization.string("Create a new wallet or bring one you already have. Your keys stay on this device."))
                    .font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            Button {
                spectraHaptic(.medium)
                store.isShowingAddWalletEntry = true
            } label: {
                Text(AppLocalization.string("Add Wallet")).font(.body.weight(.semibold)).frame(maxWidth: .infinity)
            }.buttonStyle(.glassProminent).controlSize(.large)
        }
        .padding(.horizontal, SpectraLayout.cardPadding).padding(.vertical, SpectraLayout.Space.xl)
        .frame(maxWidth: .infinity)
        .spectraElevatedFill()
    }
}

private struct DashboardActionButtons: View {
    @Bindable var store: AppState
    var body: some View {
        let canSend = store.canBeginSend
        let canReceive = store.canBeginReceive
        return GlassEffectContainer(spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                Button { spectraHaptic(.medium); store.beginSend() } label: {
                    Label(AppLocalization.string("Send"), systemImage: "arrow.up.right")
                        .font(.body.weight(.semibold)).frame(maxWidth: .infinity).padding(.vertical, SpectraLayout.Space.m)
                }.buttonStyle(.glass)
                    .disabled(!canSend)
                Button { spectraHaptic(.medium); store.beginReceive() } label: {
                    Label(AppLocalization.string("Receive"), systemImage: "arrow.down.left")
                        .font(.body.weight(.semibold)).frame(maxWidth: .infinity).padding(.vertical, SpectraLayout.Space.m)
                }.buttonStyle(.glassProminent)
                    .disabled(!canReceive)
            }
        }
    }
}

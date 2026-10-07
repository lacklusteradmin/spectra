import SwiftUI
struct DashboardView: View {
    @Bindable var store: AppState
    @State private var dashboardPage: DashboardPage = .assets
    @State private var isNavigatingToPinnedAssets = false
    @State private var selectedWalletId: String?
    @State private var selectedAssetGroup: DashboardAssetGroup?
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
            }.navigationDestination(item: $selectedAssetGroup) { assetGroup in AssetGroupDetailView(store: store, assetGroup: assetGroup) }
                .navigationDestination(isPresented: Bindable(store.sendFlow).isPresented) {
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
                Text("\(min(count, 9))").font(.caption2.weight(.bold)).foregroundStyle(.white).padding(.horizontal, SpectraLayout.Space.xs).padding(.vertical, SpectraLayout.Space.xxs)
                    .background(Capsule().fill(Color.red)).offset(x: 6, y: -5)
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
    private var assetsOrWalletsCard: some View {
        VStack(spacing: 0) {
            HStack(spacing: SpectraLayout.Space.l) {
                dashboardPageTab(.assets, title: "Assets", count: visiblePortfolio.count)
                dashboardPageTab(.wallets, title: "Wallets", count: store.wallets.count)
                Spacer(minLength: 0)
                if dashboardPage == .assets { pinAssetsButton }
            }
            .padding(.horizontal, SpectraLayout.rowHorizontal)
            .sensoryFeedback(.selection, trigger: dashboardPage)
            Divider().opacity(0.25)
            VStack(spacing: 0) {
                switch dashboardPage {
                case .wallets: walletsCardRows(wallets: store.wallets)
                case .assets: assetsCardRows(portfolio: visiblePortfolio)
                }
            }.padding(.vertical, SpectraLayout.Space.xs)
        }.frame(maxWidth: .infinity).glassEffect(
            .regular.tint(SpectraLayout.GlassTint.content).interactive(), in: .rect(cornerRadius: SpectraLayout.Radius.card))
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
    private func assetCountText(_ count: Int) -> String {
        AppLocalization.format("%lld assets", count: count, count)
    }
    @ViewBuilder
    private func walletsCardRows(wallets: [WalletView]) -> some View {
        ForEach(Array(wallets.enumerated()), id: \.element.id) { index, wallet in
            let badge = AssetHolding.nativeChainBadge(for: wallet.family) ?? (nil, .mint)
            Button { selectedWalletId = wallet.id } label: {
                WalletCardView(
                    presentation: WalletCardView.Presentation(
                        walletName: wallet.name, chainTitleText: wallet.networkTitle,
                        totalValueText: store.preferences.hideBalances
                            ? "••••••"
                            : store.amounts.formattedWalletTotal(walletId: wallet.id),
                        assetCountText: assetCountText(wallet.shownHoldings.count),
                        isWatchOnly: wallet.signing.isWatchOnly, badgeArtworkName: badge.0,
                        badgeMark: wallet.familyName, badgeColor: badge.1
                    )
                ).equatable().padding(.horizontal, SpectraLayout.rowHorizontal).padding(.vertical, SpectraLayout.rowVertical)
            }.buttonStyle(.plain)
            if index < wallets.count - 1 { Divider().padding(.leading, SpectraLayout.rowDividerInset).opacity(0.25) }
        }
    }
    @ViewBuilder
    private func assetsCardRows(portfolio: [DashboardAssetGroup]) -> some View {
        if portfolio.isEmpty {
            emptyCardState(title: "No assets to display yet",
                           message: "Import a wallet or pull to refresh to load chain balances.",
                           systemImage: "chart.pie")
        } else {
            let presentations = visibleAssetPresentations(portfolio: portfolio)
            ForEach(Array(presentations.enumerated()), id: \.element.id) { index, presentation in
                Button { selectedAssetGroup = presentation.assetGroup } label: {
                    DashboardAssetRowView(presentation: presentation).equatable().padding(.horizontal, SpectraLayout.rowHorizontal).padding(
                        .vertical, SpectraLayout.rowVertical)
                }.buttonStyle(.plain)
                if index < presentations.count - 1 { Divider().padding(.leading, SpectraLayout.rowDividerInset).opacity(0.25) }
            }
        }
    }
    private func emptyCardState(title: String, message: String, systemImage: String) -> some View {
        SpectraEmptyStateContent(title: title, message: message, systemImage: systemImage)
            .padding(.horizontal, SpectraLayout.rowHorizontal)
            .padding(.vertical, SpectraLayout.Space.m)
    }
    private var visiblePortfolio: [DashboardAssetGroup] { store.cachedDashboardAssetGroups }
    private func visibleAssetPresentations(portfolio: [DashboardAssetGroup]) -> [DashboardAssetRowPresentation] {
        let hideBalances = store.preferences.hideBalances
        return portfolio.map { assetGroup in
            DashboardAssetRowPresentation(
                assetGroup: assetGroup,
                amountText: store.amounts.formattedAssetAmount(
                    assetGroup.totalAmount, symbol: assetGroup.symbol, deploymentId: assetGroup.identity.holdingKey
                ),
                totalValueText: hideBalances
                    ? "••••••"
                    : store.amounts.formattedFiat(assetGroup.totalValue),
                priceText: dashboardAssetPriceText(for: assetGroup, hideBalances: hideBalances)
            )
        }
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
    private func dashboardAssetPriceText(for assetGroup: DashboardAssetGroup, hideBalances: Bool) -> String {
        hideBalances ? "••••••" : store.amounts.formattedFiat(assetGroup.price)
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
struct AssetGroupDetailView: View {
    let store: AppState
    let assetGroup: DashboardAssetGroup
    /// Where the coin lives, from core's asset wiki — the same join the wiki
    /// screen renders, rather than a second dashboard-only cache of it.
    private var places: [AssetWikiPlace] {
        CoreReferenceTables.assetWikiEntry(tokenId: assetGroup.id)?.livesOn ?? []
    }
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                AssetDetailHeroCard(assetGroup: assetGroup, store: store)
                AssetChainBreakdownCard(assetGroup: assetGroup, store: store)
            }.spectraScreenPadding()
        }.background(SpectraBackdrop().ignoresSafeArea())
            .navigationTitle(assetGroup.symbol).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .toolbar {
                if !places.isEmpty {
                    ToolbarItem(placement: .topBarTrailing) {
                        NavigationLink(AppLocalization.string("Details")) {
                            AssetContractsDetailView(store: store, assetGroup: assetGroup)
                        }
                    }
                }
            }
    }
}
struct AssetContractsDetailView: View {
    let store: AppState
    let assetGroup: DashboardAssetGroup
    private var places: [AssetWikiPlace] {
        CoreReferenceTables.assetWikiEntry(tokenId: assetGroup.id)?.livesOn ?? []
    }
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                AssetDetailHeroCard(assetGroup: assetGroup, store: store, compact: true)
                AssetPlacesCard(places: places, symbol: assetGroup.symbol)
            }.spectraScreenPadding()
        }.background(SpectraBackdrop().ignoresSafeArea())
            .navigationTitle(AppLocalization.format("%@ Details", assetGroup.symbol)).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
    }
}
private struct AssetDetailHeroCard: View {
    let assetGroup: DashboardAssetGroup
    let store: AppState
    var compact: Bool = false
    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: assetGroup.artworkName, fallbackText: assetGroup.symbol,
                color: assetGroup.color, size: compact ? 48 : 60
            )
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                Text(assetGroup.name).font(compact ? .title3.weight(.bold) : .title2.weight(.bold))
                    .foregroundStyle(Color.primary).lineLimit(1).minimumScaleFactor(0.8)
                Text(assetGroup.symbol).font(.subheadline.weight(.semibold).monospaced())
                    .foregroundStyle(assetGroup.color)
                if !compact {
                    Text(store.amounts.formattedFiat(assetGroup.totalValue))
                        .font(.title3.weight(.semibold)).foregroundStyle(Color.primary)
                        .spectraNumericTextLayout(minimumScaleFactor: 0.7)
                    Text(
                        store.amounts.formattedAssetAmount(
                            assetGroup.totalAmount, symbol: assetGroup.symbol,
                            deploymentId: assetGroup.identity.holdingKey)
                    ).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                        .spectraNumericTextLayout(minimumScaleFactor: 0.7)
                }
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
                        fallbackColor: holding.coin.color
                    )
                    if index < assetGroup.holdings.count - 1 { Divider().opacity(0.3) }
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
                Text(amountText).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary).spectraNumericTextLayout()
                Text(valueText).font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
            }
        }
    }
}
struct PinnedAssetsView: View {
    let store: AppState
    @State private var searchText: String = ""
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
                    store.resetPinnedDashboardAssets()
                }
            }
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
                    ForEach(notices) { notice in DashboardNoticeCardView(notice: notice) }
                }
            }
        }.navigationTitle(AppLocalization.string("Notices"))
    }
}
struct DashboardAssetRowPresentation: Identifiable, Equatable {
    let assetGroup: DashboardAssetGroup
    let amountText: String
    let totalValueText: String
    let priceText: String
    var id: String { assetGroup.id }
}
struct DashboardAssetRowView: View, Equatable {
    let presentation: DashboardAssetRowPresentation
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.presentation == rhs.presentation }
    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: presentation.assetGroup.artworkName, fallbackText: presentation.assetGroup.symbol,
                color: presentation.assetGroup.color, size: 36
            )
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                HStack(spacing: SpectraLayout.Space.s) {
                    if presentation.assetGroup.isPinned {
                        Image(systemName: "pin.fill").font(.caption.weight(.semibold)).foregroundStyle(Color.red.opacity(0.82)).frame(
                            width: 24, height: 18
                        ).background(Color.red.opacity(0.1), in: Capsule()).clipped()
                    }
                    Text(presentation.assetGroup.name).font(.headline).foregroundStyle(Color.primary).lineLimit(1).truncationMode(.tail)
                }
                Text(presentation.amountText).font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
            }
            Spacer()
            VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                Text(presentation.totalValueText).font(.headline).foregroundStyle(Color.primary).spectraNumericTextLayout()
                Text(presentation.priceText).font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
            }
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }.contentShape(Rectangle())
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
                        "Last known healthy sync: %@", timestamp.formatted(date: .abbreviated, time: .shortened))
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
        NavigationLink {
            PortfolioWalletSelectionView(store: store)
        } label: {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                    Text(AppLocalization.string("Portfolio")).font(.subheadline).foregroundStyle(.secondary)
                    Text(store.preferences.hideBalances ? "••••••" : store.amounts.formattedQuotedTotal(store.portfolioQuotedTotal))
                        .font(.title.weight(.bold)).foregroundStyle(Color.primary).lineLimit(1).minimumScaleFactor(0.5).allowsTightening(true)
                }
                Spacer()
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }.padding(SpectraLayout.cardPadding).frame(maxWidth: .infinity, alignment: .leading)
                .spectraElevatedFill()
        }.buttonStyle(.plain)
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

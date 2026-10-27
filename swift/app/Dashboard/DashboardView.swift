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

extension DashboardAssetGroup: Identifiable {
    /// Core supplies the asset's display identity from its largest holding.
    var name: String { identity.name }
    var symbol: String { identity.symbol }
    var artworkName: String { identity.artworkName }
    var color: Color { identity.color }
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

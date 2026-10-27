import Foundation
import SwiftUI

struct WalletDetailView: View {
    let store: AppState
    let wallet: WalletView
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var isShowingManagePage: Bool = false
    @State private var isShowingHiddenHoldings = false
    /// Core's answer to what this wallet offers; read again when the wallet
    /// or the endpoints behind its notes change.
    @State private var actions: WalletActions?
    /// The action whose page is open.
    @State private var openedAction: WalletAction?
    /// The wallet's ENS primary name, where core finds one that resolves back.
    @State private var ensName: String?
    private struct ActionsKey: Equatable {
        let walletId: String
        let identityRevision: UInt64
        let settings: AppSettings
    }
    init(store: AppState, wallet: WalletView) {
        self.store = store
        self.wallet = wallet
    }
    private struct HoldingPresentation: Identifiable {
        let coin: AssetHolding
        let amountText: String
        let valueText: String
        var id: String { coin.id }
    }
    private struct DetailPresentation {
        let wallet: WalletView
        let walletAddress: String?
        let derivationPathsText: String?
        let walletBadge: (artworkName: String?, color: Color)
        let visibleHoldingPresentations: [HoldingPresentation]
        /// What the user hid, listed apart and counted in no total.
        let hiddenHoldingPresentations: [HoldingPresentation]
        let walletTotalValueText: String
    }
    private var isWatchOnly: Bool { displayedWallet.signing.isWatchOnly }
    private var isPrivateKeyWallet: Bool { displayedWallet.signing.isPrivateKey }
    private var displayedWallet: WalletView {
        store.wallet(for: wallet.id) ?? wallet
    }
    private var firstActivityDateText: String {
        guard let firstDate = store.cachedFirstActivityDateByWalletId[wallet.id] else {
            return AppLocalization.string("No activity yet")
        }
        return firstDate.appFormatted(time: .shortened)
    }
    private var detailPresentation: DetailPresentation {
        let wallet = displayedWallet
        // Core orders the holdings: most valuable first, unpriced last.
        let presentation = { (holding: AssetHolding) in
            HoldingPresentation(
                coin: holding,
                amountText: store.amounts.formattedAssetAmount(holding.amount, symbol: holding.symbol, deploymentId: holding.holdingKey),
                valueText: store.amounts.formattedFiat(store.amounts.holdingValue(walletId: wallet.id, coin: holding))
            )
        }
        let holdingPresentations = wallet.shownHoldings.map(presentation)
        let hiddenPresentations = wallet.holdings.filter(wallet.hides).map(presentation)
        return DetailPresentation(
            wallet: wallet,
            // A wallet is on one chain, so its address is that chain's.
            walletAddress: wallet.address(on: wallet.chain),
            derivationPathsText: derivationPathsText(for: wallet),
            walletBadge: AssetHolding.nativeChainBadge(for: wallet.family) ?? (nil, .mint),
            visibleHoldingPresentations: holdingPresentations,
            hiddenHoldingPresentations: hiddenPresentations,
            walletTotalValueText: store.amounts.formattedWalletTotal(walletId: wallet.id)
        )
    }
    /// The wallet's derivation path, named by its profile and account when
    /// core reads it as one.
    private func derivationPathsText(for wallet: WalletView) -> String? {
        guard !isWatchOnly, !isPrivateKeyWallet else { return nil }
        let chain = wallet.chainId
        guard let path = wallet.derivationPath, !path.isEmpty else { return nil }
        guard let choice = derivationProfileOfPath(chain: chain, path: path) else { return path }
        return AppLocalization.format(
            "wallet.detail.chainPath",
            AppLocalization.format("derivation.profile_account_format", choice.profile.title, Int(choice.account)), path)
    }
    private var watchOnlyBadge: some View {
        Label(AppLocalization.string("Watching"), systemImage: "eye").font(.caption.weight(.semibold)).foregroundStyle(.tint).padding(
            .horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs).background(Color.accentColor.opacity(0.15), in: Capsule())
    }
    /// Deleting the wallet from the advanced page removes it under both
    /// pages; leaving this one pops that one too.
    private func handleWalletPresenceChange(walletStillExists: Bool) {
        guard !walletStillExists else { return }
        dismiss()
    }
    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                walletHeroCard
                if let actions {
                    WalletEverydayActionBar(offers: actions.actions(in: .everyday), perform: perform)
                }
                walletHoldingsCard
                if let walletAddress = detailPresentation.walletAddress {
                    walletAddressCard(walletAddress: walletAddress)
                }
                if let actions {
                    let network = actions.actions(in: .network)
                    if !network.isEmpty { WalletNetworkActionsCard(offers: network, perform: perform) }
                    WalletNetworkNotesCard(summary: actions.summary)
                }
            }.spectraScreenPadding()
        }.background(SpectraBackdrop().ignoresSafeArea())
            // Waits for the read to finish, as Home's does, so the spinner
            // means something; this wallet's network is what it reads.
            .refreshable {
                _ = await store.performUserInitiatedRefresh(forChain: displayedWallet.chain)
            }.navigationTitle(AppLocalization.string("Wallet Details")).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                // Name, keys, other networks and deletion: managing the
                // wallet, not settings for experts.
                Button(AppLocalization.string("Manage"), systemImage: "slider.horizontal.3") {
                    isShowingManagePage = true
                }
            }
        }.navigationDestination(isPresented: $isShowingManagePage) {
            WalletAdvancedDetailsView(
                store: store, wallet: detailPresentation.wallet,
                manageOffers: actions?.actions(in: .manage) ?? [],
                derivationPathsText: detailPresentation.derivationPathsText,
                firstActivityDateText: firstActivityDateText
            )
        }.navigationDestination(item: $openedAction) { action in
            actionPage(action)
        }.task(id: ActionsKey(
            walletId: wallet.id, identityRevision: store.walletIdentityRevision, settings: store.appSettings
        )) {
            await loadActions()
        }.task(id: wallet.id) {
            ensName = try? await store.bridge.ready().walletEnsName(walletId: wallet.id)
        }.onChange(of: (store.wallet(for: wallet.id) != nil)) { _, walletStillExists in
            handleWalletPresenceChange(walletStillExists: walletStillExists)
        }
    }
    /// Hide a holding from the wallet's total and the portfolio, or show it
    /// again; core keeps the choice with the wallet.
    private func setHidden(_ holding: AssetHolding, _ hidden: Bool) {
        spectraHaptic(.light)
        Task {
            do {
                _ = try await store.stateCommands.apply(
                    .setHoldingHidden(walletId: wallet.id, deploymentId: holding.holdingKey, hidden: hidden))
            } catch {
                store.reportCommandError(error)
            }
        }
    }
    private func loadActions() async {
        do {
            let loaded = try await store.bridge.ready().walletActions(walletId: wallet.id)
            guard !Task.isCancelled else { return }
            actions = loaded
        } catch {
            guard !Task.isCancelled else { return }
            // A wallet deleted under the page has no actions; leaving pops it.
            actions = nil
        }
    }
    /// Open what `action` does: a flow the app presents, or its own page.
    private func perform(_ action: WalletAction) {
        switch action {
        case .send: store.beginSend(walletId: wallet.id)
        case .receive: store.beginReceive(walletId: wallet.id)
        case .openInExplorer:
            if let address = detailPresentation.walletAddress,
               let link = addressExplorerLink(chainId: displayedWallet.chain, address: address),
               let url = URL(string: link.url) {
                openURL(url)
            }
        case .getTestCoins:
            if let url = displayedWallet.chain.faucetURL {
                openURL(url)
            }
        case .addToNetwork, .rename, .revealPhrase, .exportKeys, .delete: isShowingManagePage = true
        case .history, .multisig, .addKeys, .stake, .scanBlocks, .coins, .tokenApprovals, .nfts, .shieldedFunds,
             .mwebFunds, .networkAccount, .accessKeys, .tokenStorage, .coinObjects, .tokenAccounts, .trustLines,
             .signMessage, .verifyMessage:
            openedAction = action
        }
    }
    @ViewBuilder
    private func actionPage(_ action: WalletAction) -> some View {
        switch action {
        case .history:
            HistoryListView(store: store, walletId: wallet.id)
        case .multisig:
            WalletMultisigView(store: store, wallet: displayedWallet)
        case .addKeys:
            WalletSetupMethodsView(
                store: store, chain: displayedWallet.chain,
                upgrading: (displayedWallet, actions?.actions.first { $0.action == .addKeys }?.note ?? ""))
        case .stake:
            WalletStakingView(store: store, wallet: displayedWallet)
        case .scanBlocks:
            WalletBlockScanView(store: store, wallet: displayedWallet)
        case .coins:
            WalletCoinsView(store: store, wallet: displayedWallet)
        case .tokenApprovals:
            WalletApprovalsView(store: store, wallet: displayedWallet)
        case .nfts:
            WalletNftsView(store: store, wallet: displayedWallet)
        case .shieldedFunds:
            WalletShieldedFundsView(store: store, wallet: displayedWallet)
        case .mwebFunds:
            WalletMwebFundsView(store: store, wallet: displayedWallet)
        case .networkAccount:
            WalletNetworkAccountView(store: store, wallet: displayedWallet)
        case .accessKeys:
            WalletAccessKeysView(store: store, wallet: displayedWallet)
        case .tokenStorage:
            WalletTokenStorageView(store: store, wallet: displayedWallet)
        case .coinObjects:
            WalletCoinObjectsView(store: store, wallet: displayedWallet)
        case .tokenAccounts:
            WalletTokenAccountsView(store: store, wallet: displayedWallet)
        case .trustLines:
            WalletTrustLinesView(store: store, wallet: displayedWallet)
        case .signMessage, .verifyMessage:
            WalletMessageView(store: store, wallet: displayedWallet, canSign: action == .signMessage)
        case .send, .receive, .openInExplorer, .getTestCoins, .addToNetwork, .rename, .revealPhrase, .exportKeys,
             .delete:
            EmptyView()
        }
    }
    @ViewBuilder
    private var walletHeroCard: some View {
        let presentation = detailPresentation
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: presentation.walletBadge.artworkName,
                fallbackText: presentation.wallet.familyName,
                color: presentation.walletBadge.color, size: 56
            )
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                    Text(presentation.wallet.name).font(.title2.weight(.bold)).foregroundStyle(Color.primary).lineLimit(1)
                        .minimumScaleFactor(0.7)
                    Spacer(minLength: 0)
                    if isWatchOnly { watchOnlyBadge }
                }
                Text(presentation.wallet.networkTitle).font(.subheadline.weight(.medium))
                    .foregroundStyle(presentation.walletBadge.color)
                if let ensName {
                    Text(verbatim: ensName).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                if presentation.wallet.balancesReadAt == nil {
                    SpectraShimmer(height: 20).frame(maxWidth: 140)
                        .accessibilityLabel(AppLocalization.string("Reading balances…"))
                } else {
                    BalanceText(text: presentation.walletTotalValueText, isHidden: store.preferences.hideBalances)
                        .font(.title3.weight(.semibold)).foregroundStyle(Color.primary)
                        .spectraNumericTextLayout(minimumScaleFactor: 0.7)
                }
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraElevatedFill()
    }
    private var walletHoldingsCard: some View {
        let presentation = detailPresentation
        let shown = presentation.visibleHoldingPresentations
        return SpectraRowGroup(
            title: AppLocalization.string("Holdings"), data: shown, header: { EmptyView() },
            footer: { holdingsFooter(shown: shown, hidden: presentation.hiddenHoldingPresentations) }
        ) { holding in
            holdingRow(holding).spectraRowPadding()
                .contextMenu {
                    Button {
                        setHidden(holding.coin, true)
                    } label: {
                        Label(AppLocalization.string("Hide From Totals"), systemImage: "eye.slash")
                    }
                }
        }
    }
    /// Under the rows: why there are none, or the hidden holdings, folded
    /// away until asked for, each with the way to show it again.
    @ViewBuilder
    private func holdingsFooter(shown: [HoldingPresentation], hidden: [HoldingPresentation]) -> some View {
        if shown.isEmpty && hidden.isEmpty {
            SpectraEmptyStateContent(
                title: "No assets loaded",
                message: "No assets loaded for this wallet yet.",
                systemImage: "tray"
            )
            .padding(.horizontal, SpectraLayout.rowHorizontal).padding(.vertical, SpectraLayout.Space.m)
        }
        if !hidden.isEmpty {
            if !shown.isEmpty { Divider().opacity(0.25) }
            Button {
                spectraHaptic(.light)
                isShowingHiddenHoldings.toggle()
            } label: {
                HStack(spacing: SpectraLayout.Space.s) {
                    Text(AppLocalization.format("Hidden Assets (%lld)", hidden.count))
                        .font(.subheadline.weight(.semibold)).foregroundStyle(.tint)
                    Spacer(minLength: 0)
                    Image(systemName: isShowingHiddenHoldings ? "chevron.up" : "chevron.down")
                        .font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                }.spectraRowPadding()
            }
            .buttonStyle(.plain)
            .accessibilityValue(AppLocalization.string(isShowingHiddenHoldings ? "Expanded" : "Collapsed"))
            if isShowingHiddenHoldings {
                ForEach(hidden) { holding in
                    Divider().padding(.leading, SpectraLayout.rowDividerInset).opacity(0.25)
                    HStack(spacing: SpectraLayout.Space.m) {
                        holdingRow(holding)
                        Button(AppLocalization.string("Show")) { setHidden(holding.coin, false) }
                            .buttonStyle(.glass).font(.caption.weight(.semibold))
                    }.spectraRowPadding()
                }
            }
        }
    }
    @ViewBuilder
    private func walletAddressCard(walletAddress: String) -> some View {
        let chain = displayedWallet.chain
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack(spacing: SpectraLayout.Space.s) {
                Text(AppLocalization.string("Wallet Address")).font(.headline).foregroundStyle(Color.primary)
                Spacer()
                CopyButton(value: displayAddress(chain: chain, address: walletAddress), title: "Copy")
            }
            // Grouped and in its checksummed case, so it is compared a group
            // at a time; never hyphenated where it wraps.
            Text(readableAddress(walletAddress, chain: chain)).font(.body.monospaced())
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, SpectraLayout.Space.m).padding(.vertical, SpectraLayout.Space.s)
                .frame(maxWidth: .infinity, alignment: .leading).spectraInsetFill()
                .accessibilityLabel(Text(verbatim: walletAddress))
            // An ICP account is a hash of its principal; each names the
            // wallet to a different kind of sender.
            if let principal = displayedWallet.icpPrincipal {
                Text(AppLocalization.string("Principal")).font(.subheadline.weight(.semibold))
                Text(principal).font(.footnote.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                    .padding(.horizontal, SpectraLayout.Space.m).padding(.vertical, SpectraLayout.Space.s)
                    .frame(maxWidth: .infinity, alignment: .leading).spectraInsetFill()
                Text(AppLocalization.string("Exchanges and ICP ledger transfers pay the account ID above. ICRC tokens and the NNS address the principal."))
                    .font(.caption).foregroundStyle(.secondary)
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraCardFill()
    }
    /// A holding's badge, name and balance; its row pads it.
    private func holdingRow(_ holding: HoldingPresentation) -> some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: holding.coin.artworkName, fallbackText: holding.coin.symbol, color: holding.coin.color, size: 36
            )
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(holding.coin.name).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
                Text(verbatim: "\(holding.coin.symbol) • \(holding.coin.standardLabel)").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                BalanceText(text: holding.amountText, isHidden: store.preferences.hideBalances)
                    .font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary).spectraNumericTextLayout()
                BalanceText(text: holding.valueText, isHidden: store.preferences.hideBalances)
                    .font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
            }
        }
    }
}

import Foundation
import SwiftUI
import CoreImage
import CoreImage.CIFilterBuiltins
import UIKit
import Vision
import VisionKit
struct TransactionStatusBadge: View {
    let status: TransactionStatus
    private var statusText: String { status.localizedTitle }
    private var statusColor: Color { Color.spectraTransactionStatusColor(status) }
    private var badgeScale: CGFloat {
        switch status {
        case .pending: return 1.0
        case .confirmed: return 1.05
        case .failed: return 0.97
        }
    }
    var body: some View {
        Text(statusText.uppercased()).font(.caption2.bold()).tracking(0.6).frame(minWidth: 86).padding(.horizontal, SpectraLayout.Space.s).padding(
            .vertical, SpectraLayout.Space.xs).background(statusColor.opacity(0.16), in: Capsule()).foregroundStyle(statusColor).scaleEffect(badgeScale).animation(
            .spring(response: 0.35, dampingFraction: 0.74), value: status)
    }
}
struct SendQRScannerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let onScan: (String) -> Void
    var body: some View {
        NavigationStack {
            QRCodeScannerView { payload in
                onScan(payload)
                dismiss()
            }.ignoresSafeArea(edges: .bottom).navigationTitle(AppLocalization.string("Scan QR Code")).navigationBarTitleDisplayMode(
                .inline
            ).toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(AppLocalization.string("Cancel")) {
                        dismiss()
                    }
                }
            }
        }
    }
}
struct QRCodeScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(onScan: onScan) }
    func makeUIViewController(context: Context) -> DataScannerViewController {
        let controller = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false, isPinchToZoomEnabled: true, isGuidanceEnabled: true, isHighlightingEnabled: true
        )
        controller.delegate = context.coordinator
        try? controller.startScanning()
        return controller
    }
    func updateUIViewController(_ uiViewController: DataScannerViewController, context: Context) {}
    static func dismantleUIViewController(_ uiViewController: DataScannerViewController, coordinator: Coordinator) {
        uiViewController.stopScanning()
    }
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        private let onScan: (String) -> Void
        private var hasResolvedPayload = false
        init(onScan: @escaping (String) -> Void) {
            self.onScan = onScan
        }
        func dataScanner(_ dataScanner: DataScannerViewController, didTapOn item: RecognizedItem) { resolve(item, from: dataScanner) }
        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            guard let firstItem = addedItems.first else { return }
            resolve(firstItem, from: dataScanner)
        }
        private func resolve(_ item: RecognizedItem, from dataScanner: DataScannerViewController) {
            guard !hasResolvedPayload else { return }
            guard case .barcode(let barcode) = item,
                let payload = barcode.payloadStringValue?.trimmingCharacters(in: .whitespacesAndNewlines), !payload.isEmpty
            else { return }
            hasResolvedPayload = true
            dataScanner.stopScanning()
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            onScan(payload)
        }
    }
}
struct WalletCardView: View, Equatable {
    struct Presentation: Equatable {
        let walletName: String
        let chainTitleText: String
        let totalValueText: String
        let assetCountText: String
        let isWatchOnly: Bool
        let badgeArtworkName: String?
        let badgeMark: String
        let badgeColor: Color
    }
    let presentation: Presentation
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.presentation == rhs.presentation }
    private var watchOnlyBadge: some View {
        Image(systemName: "eye").font(.caption.weight(.semibold)).foregroundStyle(.tint).padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs)
            .background(Color.accentColor.opacity(0.15), in: Capsule())
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(
                    artworkName: presentation.badgeArtworkName, fallbackText: presentation.badgeMark,
                    color: presentation.badgeColor, size: 36)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    HStack(spacing: SpectraLayout.Space.xs) {
                        if presentation.isWatchOnly { watchOnlyBadge }
                        Text(presentation.walletName).font(.headline).foregroundStyle(Color.primary)
                    }
                    Text(presentation.chainTitleText).font(.caption2).foregroundStyle(.secondary).lineLimit(2)
                }
                Spacer()
                VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                    Text(presentation.totalValueText).font(.headline).foregroundStyle(Color.primary).spectraNumericTextLayout()
                    Text(presentation.assetCountText).font(.caption2).foregroundStyle(.secondary)
                }
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
        }.contentShape(Rectangle())
    }
}
struct QRCodeRenderer {
    static func makeImage(from string: String, scale: CGFloat = 12) -> UIImage? {
        let context = CIContext()
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(string.utf8)
        filter.correctionLevel = "M"
        guard let outputImage = filter.outputImage else { return nil }
        let scaledImage = outputImage.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
        guard let cgImage = context.createCGImage(scaledImage, from: scaledImage.extent) else { return nil }
        return UIImage(cgImage: cgImage)
    }
}
struct ActivityItemSheet: UIViewControllerRepresentable {
    let activityItems: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: activityItems, applicationActivities: nil)
    }
    func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}
@MainActor
final class PhotoLibraryImageSaver: NSObject {
    private let completion: (Result<Void, Error>) -> Void
    init(completion: @escaping (Result<Void, Error>) -> Void) {
        self.completion = completion
    }
    func save(_ image: UIImage) {
        UIImageWriteToSavedPhotosAlbum(image, self, #selector(saveCompleted(_:didFinishSavingWithError:contextInfo:)), nil)
    }
    @objc
    private func saveCompleted(_ image: UIImage, didFinishSavingWithError error: Error?, contextInfo: UnsafeRawPointer?) {
        if let error { completion(.failure(error)) } else { completion(.success(())) }
    }
}
struct QRCodeImage: View {
    let address: String
    var body: some View {
        if let image = qrUIImage {
            Image(uiImage: image).interpolation(.none).resizable().scaledToFit()
        } else {
            Image(systemName: "qrcode").resizable().scaledToFit().padding(SpectraLayout.Space.xl).foregroundStyle(.black)
        }
    }
    private var qrUIImage: UIImage? { QRCodeRenderer.makeImage(from: address) }
}
struct WalletDetailView: View {
    let store: AppState
    let wallet: WalletView
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var didCopyWalletAddress: Bool = false
    @State private var isShowingAdvancedPage: Bool = false
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
    private static let firstActivityFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter
    }()
    private var isWatchOnly: Bool { displayedWallet.signing.isWatchOnly }
    private var isPrivateKeyWallet: Bool { displayedWallet.signing.isPrivateKey }
    private var displayedWallet: WalletView {
        store.wallet(for: wallet.id) ?? wallet
    }
    private var firstActivityDateText: String {
        guard let firstDate = store.cachedFirstActivityDateByWalletId[wallet.id] else {
            return AppLocalization.string("No activity yet")
        }
        return Self.firstActivityFormatter.string(from: firstDate)
    }
    private var detailPresentation: DetailPresentation {
        let wallet = displayedWallet
        // Core orders the holdings: most valuable first, unpriced last.
        let presentation = { (holding: AssetHolding) in
            HoldingPresentation(
                coin: holding,
                amountText: store.amounts.formattedAssetAmount(holding.amount, symbol: holding.symbol, deploymentId: holding.holdingKey),
                valueText: store.preferences.hideBalances
                    ? "••••••" : store.amounts.formattedFiat(store.amounts.holdingValue(walletId: wallet.id, coin: holding))
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
            walletTotalValueText: store.preferences.hideBalances
                ? "••••••" : store.amounts.formattedWalletTotal(walletId: wallet.id)
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
            .refreshable {
                await store.refreshBalances()
            }.navigationTitle(AppLocalization.string("Wallet Details")).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Advanced")) {
                    isShowingAdvancedPage = true
                }
            }
        }.navigationDestination(isPresented: $isShowingAdvancedPage) {
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
        }.onChange(of: wallet.id) { _, _ in
            didCopyWalletAddress = false
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
            if let link = chainFaucetUrl(chain: displayedWallet.chain), let url = URL(string: link) {
                openURL(url)
            }
        case .addToNetwork, .rename, .revealPhrase, .exportKeys, .delete: isShowingAdvancedPage = true
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
                Text(presentation.walletTotalValueText).font(.title3.weight(.semibold)).foregroundStyle(Color.primary)
                    .spectraNumericTextLayout(minimumScaleFactor: 0.7)
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraElevatedFill()
    }
    @ViewBuilder
    private var walletHoldingsCard: some View {
        let presentation = detailPresentation
        let holdings = presentation.visibleHoldingPresentations
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("Holdings")).font(.headline).foregroundStyle(Color.primary)
            if holdings.isEmpty {
                SpectraEmptyStateContent(
                    title: "No assets loaded",
                    message: "No assets loaded for this wallet yet.",
                    systemImage: "tray"
                )
            } else {
                ForEach(holdings) { holding in
                    holdingRow(holding)
                        .contextMenu {
                            Button {
                                setHidden(holding.coin, true)
                            } label: {
                                Label(AppLocalization.string("Hide From Totals"), systemImage: "eye.slash")
                            }
                        }
                    if holding.id != holdings.last?.id {
                        Divider().opacity(0.25)
                    }
                }
            }
            let hidden = presentation.hiddenHoldingPresentations
            if !hidden.isEmpty {
                DisclosureGroup {
                    ForEach(hidden) { holding in
                        HStack {
                            holdingRow(holding)
                            Button(AppLocalization.string("Show")) { setHidden(holding.coin, false) }
                                .buttonStyle(.glass).font(.caption.weight(.semibold))
                        }
                    }
                } label: {
                    Text(AppLocalization.format("Hidden Assets (%lld)", hidden.count))
                        .font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                }
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraCardFill()
    }
    @ViewBuilder
    private func walletAddressCard(walletAddress: String) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: "qrcode").font(.subheadline.weight(.semibold)).foregroundStyle(.tint)
                Text(AppLocalization.string("Wallet Address")).font(.headline).foregroundStyle(Color.primary)
                Spacer()
                Button {
                    UIPasteboard.general.string = walletAddress
                    didCopyWalletAddress = true
                    spectraHaptic(.light)
                } label: {
                    Label(
                        didCopyWalletAddress ? AppLocalization.string("Copied") : AppLocalization.string("Copy"),
                        systemImage: didCopyWalletAddress ? "checkmark" : "doc.on.doc"
                    ).font(.caption.weight(.semibold))
                }.buttonStyle(.glass).tint(.accentColor)
            }
            Text(walletAddress).font(.footnote.monospaced()).foregroundStyle(.secondary).textSelection(
                .enabled
            ).padding(.horizontal, SpectraLayout.Space.m).padding(.vertical, SpectraLayout.Space.s).frame(maxWidth: .infinity, alignment: .leading).spectraInsetFill()
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
    @ViewBuilder
    private func holdingRow(_ holding: HoldingPresentation) -> some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: holding.coin.artworkName, fallbackText: holding.coin.symbol, color: holding.coin.color, size: 34
            )
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(holding.coin.name).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
                Text("\(holding.coin.symbol) • \(holding.coin.tokenStandard)").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                Text(holding.amountText).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary).spectraNumericTextLayout()
                Text(holding.valueText).font(.caption).foregroundStyle(.secondary).spectraNumericTextLayout()
            }
        }.padding(.vertical, SpectraLayout.Space.xs)
    }
}
/// The wallet's name, recovery phrase, identifiers and deletion.
///
/// Revealing the phrase and deleting the wallet are rare, and one is sensitive
/// and the other irreversible, so each is a row a level down, and the details
/// page is only what the wallet holds.
private struct WalletAdvancedDetailsView: View {
    let store: AppState
    let wallet: WalletView
    /// What core offers for managing the wallet, in its order.
    let manageOffers: [WalletActionOffer]
    let derivationPathsText: String?
    let firstActivityDateText: String
    @Environment(\.scenePhase) private var scenePhase
    @State private var seedReveal = SeedPhraseRevealState()
    @State private var didCopySeedPhrase = false
    @State private var isShowingDeleteWalletAlert: Bool = false
    private var displayedWallet: WalletView {
        store.wallet(for: wallet.id) ?? wallet
    }
    private func offers(_ action: WalletAction) -> Bool { manageOffers.contains { $0.action == action } }
    private func note(_ action: WalletAction) -> String {
        AppLocalization.string(manageOffers.first { $0.action == action }?.note ?? "")
    }
    private var isWatchOnly: Bool { displayedWallet.signing.isWatchOnly }
    private var isPrivateKeyWallet: Bool { displayedWallet.signing.isPrivateKey }
    private var requiresSeedPhrasePassword: Bool { displayedWallet.signing.requiresPassword }
    private var deleteWalletMessage: String {
        if isWatchOnly {
            return AppLocalization.string("You can't recover this wallet after deletion unless you still have this address.")
        }
        if isPrivateKeyWallet {
            return AppLocalization.string("Please keep this private key because you can't recover this wallet after deletion.")
        }
        return AppLocalization.string("Please take note of your seed phrase because you can't recover this wallet after deletion.")
    }
    var body: some View {
        Form {
            if offers(.rename) {
                Section {
                    Button {
                        spectraHaptic(.light)
                        store.beginEditingWallet(displayedWallet)
                    } label: {
                        HStack(spacing: SpectraLayout.Space.s) {
                            Text(AppLocalization.string("Name")).foregroundStyle(Color.primary)
                            Spacer(minLength: SpectraLayout.Space.s)
                            // Concrete label colours: a button's hierarchical
                            // styles derive from its tint, which drew the name orange.
                            Text(displayedWallet.name).foregroundStyle(Color(.secondaryLabel)).lineLimit(1)
                            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(Color(.tertiaryLabel))
                        }
                    }
                }
            }
            if offers(.revealPhrase) {
                Section(AppLocalization.string("Security")) {
                    Button {
                        spectraHaptic(.medium)
                        if requiresSeedPhrasePassword {
                            seedReveal.passwordInput = ""
                            seedReveal.isShowingPasswordPrompt = true
                        } else {
                            Task {
                                await revealSeedPhrase()
                            }
                        }
                    } label: {
                        Label(
                            seedReveal.isRevealing
                                ? AppLocalization.string("Checking Face ID...")
                                : (requiresSeedPhrasePassword
                                    ? AppLocalization.string("Show Seed Phrase (Password)")
                                    : AppLocalization.string("Show Seed Phrase")),
                            systemImage: requiresSeedPhrasePassword ? "lock.shield" : "faceid"
                        )
                    }.disabled(seedReveal.isRevealing || !displayedWallet.signing.hasSeedPhrase)
                }
            }
            if offers(.exportKeys) {
                Section {
                    NavigationLink {
                        WalletKeyExportView(store: store, wallet: displayedWallet)
                    } label: {
                        Label(WalletAction.exportKeys.title, systemImage: WalletAction.exportKeys.systemImage)
                    }
                } footer: {
                    Text(note(.exportKeys))
                }
            }
            if offers(.addToNetwork) {
                Section {
                    NavigationLink {
                        WalletCopyView(store: store, wallet: displayedWallet)
                    } label: {
                        Label(WalletAction.addToNetwork.title, systemImage: WalletAction.addToNetwork.systemImage)
                    }
                } footer: {
                    Text(note(.addToNetwork))
                }
            }
            Section(AppLocalization.string("Details")) {
                WalletDetailRow(label: "Wallet ID", value: wallet.id)
                if let derivationPathsText { WalletDetailRow(label: "Derivation Path", value: derivationPathsText) }
                WalletDetailRow(label: "First Activity", value: firstActivityDateText)
            }
            if offers(.delete) {
                Section {
                    Button(role: .destructive) {
                        spectraHaptic(.medium)
                        isShowingDeleteWalletAlert = true
                    } label: {
                        Label(AppLocalization.string("Delete Wallet"), systemImage: "trash").foregroundStyle(.red)
                    }
                }
            }
        }.navigationTitle(AppLocalization.string("Advanced")).navigationBarTitleDisplayMode(.inline)
        .navigationDestination(
            isPresented: Binding(
                get: { store.walletImport.isPresented && store.walletImport.editingWalletId == wallet.id },
                set: { isPresented in
                    if !isPresented { store.walletImport.isPresented = false }
                }
            )
        ) {
            SetupView(store: store, draft: store.walletImport.draft)
        }.alert(AppLocalization.string("Delete Wallet?"), isPresented: $isShowingDeleteWalletAlert) {
            Button(AppLocalization.string("Delete"), role: .destructive) {
                Task { await store.deleteWallet(wallet) }
            }
            Button(AppLocalization.string("Cancel"), role: .cancel) {
                isShowingDeleteWalletAlert = false
            }
        } message: {
            Text(deleteWalletMessage)
        }.alert(
            AppLocalization.string("Cannot Reveal Seed Phrase"),
            isPresented: .isPresent($seedReveal.errorMessage)
        ) {
            Button(AppLocalization.string("OK"), role: .cancel) {}
        } message: {
            Text(seedReveal.errorMessage ?? "Unknown error")
        }.onChange(of: (store.wallet(for: wallet.id) != nil)) { _, walletStillExists in
            if !walletStillExists {
                isShowingDeleteWalletAlert = false
                seedReveal.invalidate()
            }
        }.onChange(of: scenePhase) { _, newPhase in
            seedReveal.setSceneIsActive(newPhase == .active)
            // Native authentication may briefly make the scene inactive.
            // Leaving the app invalidates the request as well as its presentation.
            if newPhase == .background { seedReveal.invalidate() }
        }
        .onAppear { seedReveal.activate(sceneIsActive: scenePhase == .active) }
        .onDisappear { seedReveal.deactivate() }
        .sheet(
            isPresented: $seedReveal.isShowingPasswordPrompt,
            onDismiss: {
                seedReveal.passwordInput = ""
            }
        ) {
            NavigationStack {
                ZStack {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                        Text(
                            AppLocalization.string(
                                "This wallet has an optional seed phrase password. Enter it after Face ID to reveal the recovery phrase.")
                        ).font(.subheadline).foregroundStyle(.secondary)
                        SecureField(AppLocalization.string("Wallet Password"), text: $seedReveal.passwordInput)
                            .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive().padding(SpectraLayout.Space.m)
                            .spectraInputFieldStyle().foregroundStyle(Color.primary)
                        Button {
                            spectraHaptic(.medium)
                            let password = seedReveal.passwordInput
                            seedReveal.isShowingPasswordPrompt = false
                            seedReveal.passwordInput = ""
                            Task {
                                await revealSeedPhrase(password: password)
                            }
                        } label: {
                            Text(AppLocalization.string("Reveal Seed Phrase")).font(.headline).frame(maxWidth: .infinity)
                        }.buttonStyle(.glassProminent).disabled(
                            seedReveal.passwordInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        Spacer()
                    }.padding(SpectraLayout.Space.l)
                }.navigationTitle(AppLocalization.string("Wallet Password")).navigationBarTitleDisplayMode(.inline).toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(AppLocalization.string("Cancel")) {
                            seedReveal.isShowingPasswordPrompt = false
                        }
                    }
                }
            }
        }.sheet(
            isPresented: $seedReveal.isShowingPhraseSheet,
            onDismiss: {
                seedReveal.clearPhrase()
                didCopySeedPhrase = false
            }
        ) {
            NavigationStack {
                ZStack {
                    ScrollView(showsIndicators: false) {
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                            Text(
                                AppLocalization.string(
                                    "Write this down and keep it offline. Anyone with this phrase can control your funds.")
                            ).font(.subheadline).foregroundStyle(.secondary)
                            Text(seedReveal.phrase).font(.body.monospaced()).foregroundStyle(Color.primary).privacySensitive().padding(SpectraLayout.Space.m)
                                .frame(maxWidth: .infinity, alignment: .leading).spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
                            // Copying is how one phrase reaches a second
                            // network's import: each wallet is on one network.
                            Button {
                                copySecretToPasteboard(seedReveal.phrase)
                                didCopySeedPhrase = true
                            } label: {
                                Label(
                                    AppLocalization.string(didCopySeedPhrase ? "Copied" : "Copy"),
                                    systemImage: didCopySeedPhrase ? "checkmark" : "doc.on.doc"
                                ).font(.subheadline.weight(.semibold))
                            }.buttonStyle(.glass).tint(.accentColor).disabled(seedReveal.phrase.isEmpty)
                            Text(AppLocalization.string("A copied phrase stays on this device and leaves the clipboard after a minute."))
                                .font(.caption).foregroundStyle(.secondary)
                        }.padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
                            .padding(SpectraLayout.Space.l)
                    }
                }.navigationTitle(AppLocalization.string("Seed Phrase")).navigationBarTitleDisplayMode(.inline).toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(AppLocalization.string("Done")) {
                            seedReveal.isShowingPhraseSheet = false
                        }
                    }
                }
            }
        }
    }
    private func revealSeedPhrase(password: String? = nil) async {
        let result = await seedReveal.reveal(canPresent: {
            store.wallet(for: wallet.id) != nil
        }, operation: {
            try await store.revealSeedPhrase(for: wallet, password: password)
        })
        if let result { spectraNotificationHaptic(result ? .success : .error) }
    }
}
private struct WalletDetailRow: View {
    let label: String
    let value: String
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(AppLocalization.string(label)).font(.caption).foregroundStyle(.secondary)
            Text(value).font(.subheadline).foregroundStyle(Color.primary).textSelection(.enabled)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}
struct SeedPathSlotEditor: View {
    let title: String
    @Binding var path: String
    let defaultPath: String
    private var segments: [DerivationPathSegment] {
        parseDerivationPath(rawPath: path) ?? parseDerivationPath(rawPath: defaultPath) ?? []
    }
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack {
                Text(AppLocalization.string(title)).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
                Spacer()
                Button(AppLocalization.string("Reset")) {
                    path = defaultPath
                }.font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: SpectraLayout.Space.s) {
                    Text("m").font(.caption.monospaced().weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(segments.indices, id: \.self) { index in
                        let segment = segments[index]
                        HStack(spacing: SpectraLayout.Space.xs) {
                            Text(verbatim: "/").font(.caption.monospaced()).foregroundStyle(.secondary)
                            TextField(
                                "0",
                                text: Binding(
                                    get: { String(segment.value) }, set: { updateSegment(at: index, value: $0) }
                                )
                            ).keyboardType(.numberPad).font(.caption.monospaced()).foregroundStyle(Color.primary).padding(.horizontal, SpectraLayout.Space.s)
                                .padding(.vertical, SpectraLayout.Space.s).spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
                            if segment.isHardened {
                                Text(verbatim: "'").font(.caption.monospaced().weight(.bold)).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
}
    }
    private func updateSegment(at index: Int, value: String) {
        guard var resolvedSegments = parseDerivationPath(rawPath: path) ?? parseDerivationPath(rawPath: defaultPath),
            resolvedSegments.indices.contains(index), let numericValue = UInt32(value.filter(\.isNumber))
        else { return }
        resolvedSegments[index].value = numericValue
        path = formatDerivationPath(segments: resolvedSegments)
    }
}

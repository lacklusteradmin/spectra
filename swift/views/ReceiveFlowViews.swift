import Foundation
import SwiftUI
import UIKit

/// Receive starts on the wallet list; choosing a wallet pushes its address,
/// so the navigation bar's back button and swipe return to the list.
struct ReceiveView: View {
    @Bindable var store: AppState
    @State private var isShowingAddress = false

    var body: some View {
        // One wallet leaves nothing to choose: `beginReceive` selected it.
        if store.receiveEnabledWallets.count == 1 {
            ReceiveAddressView(store: store)
        } else {
            walletList
                .navigationDestination(isPresented: $isShowingAddress) {
                    ReceiveAddressView(store: store)
                }
        }
    }

    /// The question, then every wallet that can receive as Home lists it —
    /// its network, what it holds and whether Spectra only watches it — so
    /// the one to receive into is told apart without opening each.
    private var walletList: some View {
        ReceiveScreen(title: "Receive") {
            FlowPageHeading(
                title: "Choose a wallet to receive into",
                subtitle: "Each wallet has its own address on its network.")
            if store.receiveEnabledWallets.isEmpty {
                SpectraEmptyStateCard(
                    title: "No receive wallets",
                    message: "Import a wallet to generate receive addresses.",
                    systemImage: "wallet.bifold"
                )
            } else {
                SpectraRowGroup(data: store.receiveEnabledWallets) { wallet in
                    // A link, not a choice: every row opens its wallet, so
                    // none is marked.
                    Button { select(wallet) } label: {
                        WalletCardView(presentation: .init(wallet: wallet, store: store))
                            .equatable().spectraRowPadding()
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    private func select(_ wallet: WalletView) {
        spectraHaptic(.light)
        if store.receiveFlow.walletId != wallet.id {
            store.receiveFlow.walletId = wallet.id
            store.syncReceiveAssetSelection()
        }
        isShowingAddress = true
    }
}

private struct ReceiveAddressView: View {
    @Bindable var store: AppState
    @State private var didCopy: Bool = false
    @State private var isShowingShareSheet: Bool = false
    /// What an account on a reserve network must first receive to exist,
    /// read from the network while the wallet holds nothing.
    @State private var reserve: AccountReserve?
    /// An amount, and a memo where the network takes one, to ask the sender
    /// for. Empty asks for nothing and the code is the address alone.
    @State private var requestedAmount = ""
    @State private var requestMemoKind: PaymentMemoKind?
    @State private var requestMemo = ""
    @State private var isRequestingAmount = false

    private var selectedWallet: WalletView? {
        store.receiveEnabledWallets.first(where: { $0.id == store.receiveFlow.walletId })
    }

    private var selectedCoin: AssetHolding? {
        store.selectedReceiveCoin(for: store.receiveFlow.walletId)
    }

    /// The address as it is shown, copied, shared and encoded: core's display
    /// form, an EVM address in its checksummed case.
    private var address: String? {
        let resolved = store.receiveFlow.resolvedAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !resolved.isEmpty, !store.receiveFlow.isResolving, let chain = selectedCoin?.chain ?? selectedWallet?.chain
        else { return nil }
        return displayAddress(chain: chain, address: resolved)
    }

    /// Core's payment code for the amount asked, or why it cannot be one.
    /// `nil` while nothing is asked: the code is then the address.
    private var paymentRequest: Result<String, Error>? {
        guard isRequestingAmount, let address, let chain = selectedCoin?.chain else { return nil }
        let amount = AmountPresentation.canonicalDecimalInput(requestedAmount)
        let memoText = requestMemo.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !amount.isEmpty || !memoText.isEmpty else { return nil }
        let kinds = paymentMemoKinds(chain: chain)
        let memo = memoText.isEmpty || kinds.isEmpty
            ? nil : PaymentMemo(kind: requestMemoKind.flatMap { kinds.contains($0) ? $0 : nil } ?? kinds[0], value: memoText)
        return Result {
            try paymentRequestUri(chain: chain, address: address, amount: amount.isEmpty ? nil : amount, memo: memo)
        }
    }

    var body: some View {
        let address = address
        let request = try? paymentRequest?.get()
        // What the code holds: the request when one is asked, else the address.
        let encoded = request ?? address
        // Rendered once per pass, for the code on screen and for sharing.
        let qrImage = encoded.flatMap { QRCodeRenderer.makeImage(from: $0) }
        ReceiveScreen(title: "Receive") {
            if selectedWallet?.signing.isWatchOnly == true { watchOnlyNotice }
            receiveAddressHero(address: address, qrImage: qrImage, isRequest: request != nil)
            if let chain = selectedCoin?.chain, selectedCoin?.isNativeCoin == true, paymentRequestsSupported(chain: chain) {
                requestAmountCard(chain: chain)
            }
            receiveActionCard(address: address)
        }
        .sheet(isPresented: $isShowingShareSheet) {
            if let encoded {
                // The text first, so a message or a note carries the address
                // — or the whole request — itself; the image goes with it
                // where it can.
                ActivityItemSheet(activityItems: [encoded] + (qrImage.map { [$0] } ?? []))
            }
        }
        .onChange(of: store.receiveFlow.holdingKey) {
            requestedAmount = ""
            requestMemo = ""
            isRequestingAmount = false
        }
        .task(id: "\(store.receiveFlow.walletId)|\(store.receiveFlow.holdingKey)") {
            await store.refreshReceiveAddress()
        }
        .task(id: store.receiveFlow.walletId) {
            reserve = nil
            guard let wallet = selectedWallet, wallet.chainId.requiresAccountReserve,
                !wallet.holdings.contains(where: { $0.isNativeCoin && $0.hasBalance })
            else { return }
            reserve = try? await store.bridge.ready().accountReserve(chain: wallet.chainId)
        }
    }

    /// Spectra holds no key for a watched address. Funds sent to it are only
    /// the user's if another wallet holds that key.
    private var watchOnlyNotice: some View {
        Label(
            AppLocalization.string("receive.watchOnly.warning"),
            systemImage: "eye.trianglebadge.exclamationmark"
        )
        .font(.subheadline)
        .foregroundStyle(.spectraWarning)
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassEffect(.regular.tint(.spectraWarning.opacity(0.08)), in: .rect(cornerRadius: SpectraLayout.Radius.card))
    }

    /// Asking for an amount: the network's coin only — a request's amount is
    /// in it — and a memo where the network's payments carry one.
    private func requestAmountCard(chain: Chain) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Toggle(isOn: $isRequestingAmount.animation()) {
                Label(AppLocalization.string("Request an Amount"), systemImage: "qrcode")
                    .font(.subheadline.weight(.semibold))
            }
            if isRequestingAmount {
                HStack(spacing: SpectraLayout.Space.s) {
                    TextField(AppLocalization.string("Amount"), text: $requestedAmount)
                        .keyboardType(.decimalPad).monospacedDigit()
                    Text(verbatim: chain.gasTokenSymbol).foregroundStyle(.secondary)
                }
                .padding(SpectraLayout.Space.m)
                .spectraInputFieldStyle()
                if case let kinds = paymentMemoKinds(chain: chain), !kinds.isEmpty {
                    SendPaymentMemoField(kinds: kinds, kind: $requestMemoKind, text: $requestMemo)
                }
                if case .failure(let error) = paymentRequest {
                    Label(userErrorMessage(error), systemImage: "exclamationmark.triangle.fill")
                        .font(.caption).foregroundStyle(.red)
                } else {
                    Text(AppLocalization.string("receive.request.hint"))
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    private func receiveAddressHero(address: String?, qrImage: UIImage?, isRequest: Bool) -> some View {
        let wallet = selectedWallet
        let coin = selectedCoin
        return VStack(spacing: SpectraLayout.Space.m) {
            // The network is named here and on the wallet line under the
            // code; a third mark above this sentence said it again.
            if let coin {
                Label(
                    AppLocalization.format("Receive only %@ assets on this network. Check the sender's network before transferring.", coin.chainName),
                    systemImage: "exclamationmark.triangle.fill"
                )
                .font(.subheadline)
                .foregroundStyle(.spectraWarning)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let qrImage {
                Image(uiImage: qrImage).interpolation(.none).resizable().scaledToFit()
                    .frame(width: 184, height: 184)
                    .padding(SpectraLayout.Space.l)
                    .background(Color.white, in: RoundedRectangle(cornerRadius: SpectraLayout.Radius.inner, style: .continuous))
                    .accessibilityLabel(AppLocalization.string(isRequest ? "QR code of the payment request" : "QR code of the address"))
            } else if let error = store.receiveFlow.error {
                receiveError(error)
            } else {
                receiveQRCodePlaceholder(size: 216)
                    .accessibilityLabel(AppLocalization.string("Loading receive address…"))
            }

            // The address takes any asset on the chain, so the line names the
            // chain and draws its mark, not the gas token's.
            HStack(spacing: SpectraLayout.Space.m) {
                if let coin {
                    let badge = AssetHolding.nativeChainBadge(for: coin.chain) ?? (nil, coin.color)
                    CoinBadge(
                        artworkName: badge.artworkName,
                        fallbackText: coin.chainName,
                        color: badge.color,
                        size: 36
                    )
                }
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(wallet?.name ?? AppLocalization.string("Wallet"))
                        .font(.headline)
                    Text(coin?.chainName ?? AppLocalization.string("Select a chain"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if let reserve {
                Label(
                    AppLocalization.format(
                        "This account exists once it receives at least %@ %@; a smaller first payment fails.",
                        reserve.amount, reserve.symbol),
                    systemImage: "exclamationmark.circle"
                )
                .font(.footnote).foregroundStyle(.spectraWarning).multilineTextAlignment(.center)
            }
            if let address {
                // Grouped, so a sender compares it a group at a time.
                Text(groupedAddress(address))
                    .font(.body.monospaced())
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityLabel(Text(verbatim: address))
            }
        }
        .frame(maxWidth: .infinity)
        .padding(SpectraLayout.Space.l)
        .spectraElevatedFill()
    }

    /// The address could not be read: why, and the way to ask again.
    private func receiveError(_ error: String) -> some View {
        VStack(spacing: SpectraLayout.Space.m) {
            Image(systemName: "exclamationmark.triangle.fill").font(.largeTitle).foregroundStyle(.red)
            Text(error).font(.subheadline).foregroundStyle(.red).multilineTextAlignment(.center)
            Button {
                Task { await store.refreshReceiveAddress() }
            } label: {
                Label(AppLocalization.string("Retry"), systemImage: "arrow.clockwise").font(.subheadline.weight(.semibold))
            }
            .buttonStyle(.glass)
        }
        .frame(width: 216, height: 216)
    }

    private func receiveActionCard(address: String?) -> some View {
        GlassEffectContainer(spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                Button {
                    guard let address else { return }
                    UIPasteboard.general.string = address
                    didCopy = true
                    spectraHaptic(.light)
                } label: {
                    Label(
                        AppLocalization.string(didCopy ? "Copied" : "Copy Address"),
                        systemImage: didCopy ? "checkmark" : "doc.on.doc"
                    )
                    .font(.headline)
                    .frame(maxWidth: .infinity)
                    .frame(minHeight: 46)
                    .contentTransition(.symbolEffect(.replace))
                }
                .buttonStyle(.glassProminent)
                .disabled(address == nil)

                Button {
                    isShowingShareSheet = true
                } label: {
                    Label(AppLocalization.string("Share"), systemImage: "square.and.arrow.up")
                        .font(.headline)
                        .frame(maxWidth: .infinity)
                        .frame(minHeight: 46)
                }
                .buttonStyle(.glass)
                .disabled(address == nil)
            }
        }
        .task(id: didCopy) {
            guard didCopy else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            didCopy = false
        }
    }
}

/// The backdrop and scrolling column both receive pages share. The pages are
/// pushed, so the navigation bar's back button is their one way out.
private struct ReceiveScreen<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()

            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) { content }
                    .spectraScreenPadding()
            }
        }
        .navigationTitle(AppLocalization.string(title))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar(.hidden, for: .tabBar)
    }
}

/// `SpectraLoadingGlyph` and `SpectraShimmer` carry `@State`, so their
/// memberwise initializers are main-actor isolated; a nonisolated free
/// function cannot call them.
@MainActor private func receiveQRCodePlaceholder(size: CGFloat) -> some View {
    ZStack {
        RoundedRectangle(cornerRadius: SpectraLayout.Radius.card, style: .continuous)
            .fill(Color.white.opacity(0.82))
        VStack(spacing: SpectraLayout.Space.m) {
            SpectraLoadingGlyph(size: 42, tint: .accentColor)
            VStack(spacing: SpectraLayout.Space.s) {
                SpectraShimmer(height: 14)
                    .frame(width: size * 0.58)
                SpectraShimmer(height: 14)
                    .frame(width: size * 0.42)
            }
        }
    }
    .frame(width: size, height: size)
}

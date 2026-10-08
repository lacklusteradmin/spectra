import Foundation
import SwiftUI
import UIKit
import VisionKit

@MainActor
fileprivate struct SendComposerPresentation {
    let sendWallets: [WalletView]
    let selectedWallet: WalletView?
    let availableSendCoins: [AssetHolding]
    let selectedCoin: AssetHolding?
    let selectedCoinAmountText: String?
    let selectedCoinApproximateFiatText: String?
    let addressBookEntries: [AddressBookEntry]

    init(store: AppState) {
        sendWallets = store.sendEnabledWallets
        selectedWallet = sendWallets.first(where: { $0.id == store.sendFlow.walletId })
        availableSendCoins = store.availableSendCoins(for: store.sendFlow.walletId)
        selectedCoin = availableSendCoins.first(where: { $0.holdingKey == store.sendFlow.holdingKey })
        selectedCoinAmountText = selectedCoin.map { store.amounts.formattedAssetAmount($0.amount, symbol: $0.symbol, deploymentId: $0.holdingKey) }
        selectedCoinApproximateFiatText = store.amounts.formattedFiatIfAvailable(store.sendQuoteForEnteredAmount?.amountValue)
        addressBookEntries = store.sendAddressBookEntries
    }
}

@MainActor
struct SendFromPage: View {
    @Bindable var store: AppState
    private static let assetBadgeSize: CGFloat = 40
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SendFlowPageHeading(title: "Choose an asset to send", subtitle: "Choose an asset from your wallet.")
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                sendComposerSectionLabel("Sending wallet")
                walletRow
            }
            .padding(SpectraLayout.cardPadding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .spectraElevatedFill()

            if !presentation.availableSendCoins.isEmpty {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                    sendComposerSectionLabel("Available assets")
                    VStack(spacing: 0) {
                        ForEach(Array(presentation.availableSendCoins.enumerated()), id: \.element.holdingKey) { index, coin in
                            if index > 0 { Divider().padding(.leading, Self.assetBadgeSize + SpectraLayout.Space.m).opacity(0.3) }
                            assetRow(coin: coin, isSelected: coin.holdingKey == store.sendFlow.holdingKey)
                        }
                    }
                }
                .padding(SpectraLayout.cardPadding)
                .frame(maxWidth: .infinity, alignment: .leading)
                .spectraCardFill()
            }
        }
    }

    private var walletRow: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            if let selectedWallet = presentation.selectedWallet {
                let badge = AssetHolding.nativeChainBadge(for: selectedWallet.family) ?? (nil, Color.mint)
                CoinBadge(
                    artworkName: badge.artworkName,
                    fallbackText: selectedWallet.familyName,
                    color: badge.color,
                    size: 36
                )
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(selectedWallet.name).font(.headline).lineLimit(1)
                    // The network, not the family: a Sepolia wallet read
                    // "Ethereum" and looked like a mainnet one.
                    Text(selectedWallet.networkTitle).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
            if presentation.sendWallets.count > 1 {
                Menu {
                    Picker(AppLocalization.string("Wallet"), selection: Bindable(store.sendFlow).walletId) {
                        ForEach(presentation.sendWallets) { wallet in Text(wallet.name).tag(wallet.id) }
                    }
                } label: {
                    Text(AppLocalization.string("Change"))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.tint)
                }
                .onChange(of: store.sendFlow.walletId) { _, _ in store.syncSendAssetSelection() }
            }
        }
    }

    /// One row per asset, full width.
    private func assetRow(coin: AssetHolding, isSelected: Bool) -> some View {
        Button {
            guard !isSelected else { return }
            store.sendFlow.holdingKey = coin.holdingKey
            spectraHaptic(.light)
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(artworkName: coin.artworkName, fallbackText: coin.symbol, color: coin.color, size: Self.assetBadgeSize)
                (dynamicTypeSize.isAccessibilitySize
                    ? AnyLayout(VStackLayout(alignment: .leading, spacing: SpectraLayout.Space.s))
                    : AnyLayout(HStackLayout(spacing: SpectraLayout.Space.m))) {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        Text(coin.symbol).font(.headline)
                        Text(coin.name).font(.caption).foregroundStyle(.secondary)
                    }
                    if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: SpectraLayout.Space.s) }
                    Text(store.amounts.formattedAssetAmount(coin.amount, symbol: coin.symbol, deploymentId: coin.holdingKey))
                        .font(.subheadline.weight(.medium))
                        .spectraNumericTextLayout()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(isSelected ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
            }
            .padding(.vertical, SpectraLayout.Space.m)
            .frame(minHeight: 76)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

@MainActor
struct SendFlowPageHeading: View {
    let title: String
    var subtitle: String?

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string(title)).font(.title.weight(.bold))
            if let subtitle {
                Text(AppLocalization.string(subtitle)).font(.subheadline).foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, SpectraLayout.Space.s)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The current asset and exact wallet network remain visible while composing.
@MainActor
struct SendAssetContextView: View {
    let store: AppState

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        if let coin = presentation.selectedCoin {
            HStack(spacing: SpectraLayout.Space.s) {
                CoinBadge(artworkName: coin.artworkName, fallbackText: coin.symbol, color: coin.color, size: 24)
                Text(coin.symbol).font(.subheadline.weight(.semibold))
                Text(presentation.selectedWallet?.networkTitle ?? coin.chainName)
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(SpectraLayout.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .spectraInsetFill()
            .accessibilityElement(children: .combine)
        }
    }
}

/// A local core lookup supplies identity; a UI menu selection never does.
@MainActor
private struct SendRecipientIdentityView: View {
    let store: AppState
    let chain: Chain?
    let address: String
    var compact = false
    @State private var holder: EndpointHolder?

    private struct LookupIdentity: Hashable {
        let walletId: String
        let chain: Chain?
        let address: String
        let addressBook: [AddressBookEntry]
    }

    var body: some View {
        // Keep the task on a stable container even before core has an identity.
        VStack(alignment: .leading, spacing: 0) {
            if compact {
                HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.xs) {
                    Label(AppLocalization.string("Send to"), systemImage: "arrow.up.right")
                        .foregroundStyle(.secondary)
                    if let holder {
                        EndpointHolderLabel(holder: holder)
                    } else {
                        Text(verbatim: address).font(.subheadline.monospaced())
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .font(.subheadline)
            } else if holder != nil {
                HStack(spacing: SpectraLayout.Space.m) {
                    Image(systemName: holderSystemImage)
                        .font(.headline).foregroundStyle(.tint)
                        .frame(width: 36, height: 36)
                        .spectraInsetFill()
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        Text(verbatim: holderName).font(.headline)
                        Text(AppLocalization.string(holderSubtitle)).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .task(id: LookupIdentity(walletId: store.sendFlow.walletId, chain: chain, address: address, addressBook: store.addressBook.entries)) {
            holder = nil
            let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, let chain else { return }
            let answer = try? await store.bridge.ready().addressHolder(
                walletId: store.sendFlow.walletId, chainId: chain, address: trimmed)
            guard !Task.isCancelled else { return }
            holder = answer
        }
    }

    private var holderName: String {
        switch holder {
        case .wallet(let name), .contact(let name): name
        case nil: ""
        }
    }

    private var holderSystemImage: String {
        switch holder {
        case .wallet: "wallet.pass"
        case .contact, nil: "person.crop.circle"
        }
    }

    private var holderSubtitle: String {
        switch holder {
        case .wallet: "Your wallet"
        case .contact: "Saved contact"
        case nil: ""
        }
    }
}

/// The small uppercase label over a group inside a send card.
@MainActor
func sendComposerSectionLabel(_ title: String) -> some View {
    Text(AppLocalization.string(title))
        .font(.caption.weight(.semibold))
        .foregroundStyle(.secondary)
        .textCase(.uppercase)
}

@MainActor
struct SendRecipientPage: View {
    @Bindable var store: AppState
    @Binding var isShowingQRScanner: Bool
    @Binding var qrScannerErrorMessage: String?
    let validationError: String?
    let isValidating: Bool
    /// Core's resolution of the recipient as typed, once it has one.
    let validatedResolution: SendDestinationResolution?
    let retryValidation: () -> Void
    @State private var addressFocused = false
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.scenePhase) private var scenePhase
    @State private var hasPasteableText = false

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SendAssetContextView(store: store)
            SendFlowPageHeading(title: "Who are you sending to?", subtitle: "Enter an address or choose a saved contact.")
            recipientActions
            toCard
            Label(AppLocalization.string("Check the full address. A valid address does not verify the recipient's identity."), systemImage: "info.circle")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .onAppear { updatePasteAvailability() }
        .onReceive(NotificationCenter.default.publisher(for: UIPasteboard.changedNotification)) { _ in
            updatePasteAvailability()
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { updatePasteAvailability() }
        }
    }

    private func updatePasteAvailability() {
        // Check the type only; the system PasteButton reads contents on a tap.
        hasPasteableText = UIPasteboard.general.hasStrings
    }

    private var recipientActions: some View {
        GlassEffectContainer(spacing: SpectraLayout.Space.s) {
            (dynamicTypeSize.isAccessibilitySize
                ? AnyLayout(VStackLayout(spacing: SpectraLayout.Space.s))
                : AnyLayout(HStackLayout(spacing: SpectraLayout.Space.s))) {
                // The system control reads the clipboard only on the user's tap.
                Group {
                    if hasPasteableText {
                        PasteButton(payloadType: String.self) { pasted in
                            guard let text = pasted.first else { return }
                            store.sendFlow.address = text.trimmingCharacters(in: .whitespacesAndNewlines)
                            addressFocused = false
                        }
                        .labelStyle(.titleAndIcon)
                    } else {
                        Button {} label: {
                            Label(AppLocalization.string("Paste Address"), systemImage: "doc.on.clipboard")
                                .frame(maxWidth: .infinity, minHeight: 44)
                        }
                        .disabled(true)
                    }
                }
                .buttonStyle(.glass)
                .tint(.accentColor)
                .frame(maxWidth: .infinity, minHeight: 44)
                .accessibilityLabel(AppLocalization.string("Paste Address"))

                Button(action: scanRecipient) {
                    Label(AppLocalization.string("Scan"), systemImage: "qrcode.viewfinder")
                        .frame(maxWidth: .infinity, minHeight: 44)
                }
                .buttonStyle(.glass)
                .accessibilityLabel(AppLocalization.string("Scan QR Code"))

                Menu {
                    ForEach(presentation.addressBookEntries) { entry in
                        Button {
                            store.sendFlow.address = entry.address
                            addressFocused = false
                        } label: {
                            Text("\(entry.name) · \(entry.chainName)")
                        }
                    }
                } label: {
                    Label(AppLocalization.string("Contacts"), systemImage: "person.crop.circle")
                        .frame(maxWidth: .infinity, minHeight: 44)
                }
                .buttonStyle(.glass)
                .disabled(presentation.addressBookEntries.isEmpty)
            }
            .font(.subheadline.weight(.semibold))
        }
    }

    private var toCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SendRecipientIdentityView(
                store: store, chain: presentation.selectedCoin?.chainId,
                address: validatedResolution?.address ?? store.sendFlow.address)
            sendComposerSectionLabel("Recipient address")

            // No maximum line count: a long address remains visible at every
            // Dynamic Type size instead of scrolling inside a small field.
            SendAddressInput(
                text: Bindable(store.sendFlow).address, isFocused: $addressFocused,
                prompt: AppLocalization.string("Recipient address"))
                .onChange(of: store.sendFlow.address) { _, address in
                    // A multi-line field takes Return as a newline; here it
                    // means done, and no address holds one.
                    guard address.contains(where: \.isNewline) else { return }
                    store.sendFlow.address = address.filter { !$0.isNewline }
                    addressFocused = false
                }
                .padding(.horizontal, SpectraLayout.Space.m)
                .padding(.vertical, SpectraLayout.Space.m)
                .frame(minHeight: 44)
                .spectraInsetFill(cornerRadius: SpectraLayout.Radius.inner)

            if let chain = presentation.selectedCoin?.chainId,
               case let kinds = paymentMemoKinds(chain: chain), !kinds.isEmpty {
                SendPaymentMemoField(
                    kinds: kinds, kind: Bindable(store.sendFlow).memoKind,
                    text: Bindable(store.sendFlow).memoText)
            }

            if isValidating {
                SpectraLoadingRow(title: "Checking recipient...")
            } else if let validationError {
                HStack(spacing: SpectraLayout.Space.s) {
                    Label(validationError, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption).foregroundStyle(.red)
                    Spacer(minLength: 0)
                    Button(AppLocalization.string("Retry"), action: retryValidation)
                        .font(.caption.weight(.semibold))
                }
            } else if let validatedResolution {
                Label(
                    validatedResolution.usedEns
                        ? AppLocalization.format(
                            "Resolved ENS %@ to %@.",
                            store.sendFlow.address.trimmingCharacters(in: .whitespacesAndNewlines),
                            validatedResolution.address)
                        : AppLocalization.string("Valid address for this network"),
                    systemImage: "checkmark.circle.fill"
                )
                .font(.caption).foregroundStyle(.green)
            }
            recipientMessages
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }

    private func scanRecipient() {
        guard DataScannerViewController.isSupported else {
            qrScannerErrorMessage = AppLocalization.string("QR scanning is not supported on this device.")
            return
        }
        guard DataScannerViewController.isAvailable else {
            qrScannerErrorMessage = AppLocalization.string(
                "QR scanning is unavailable right now. Check camera permission and try again.")
            return
        }
        addressFocused = false
        isShowingQRScanner = true
    }

    @ViewBuilder
    private var recipientMessages: some View {
        if let qrScannerErrorMessage {
            Label(qrScannerErrorMessage, systemImage: "exclamationmark.triangle.fill")
                .font(.caption).foregroundStyle(.spectraWarning)
        }

        if store.sendFlow.isCheckingDestination {
            SpectraLoadingRow(title: "Checking destination on-chain balance...")
        }

        if let warning = store.sendFlow.destinationRiskWarning {
            Label(warning, systemImage: "exclamationmark.triangle.fill")
                .font(.caption).foregroundStyle(.spectraWarning)
        }

        if let info = store.sendFlow.destinationInfoMessage {
            Label(info, systemImage: "info.circle").font(.caption).foregroundStyle(.secondary)
        }
    }
}

@MainActor
struct SendAmountPage: View {
    @Bindable var store: AppState
    let quoteIsCurrent: Bool
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .largeTitle) private var amountFontSize: CGFloat = 56

    /// Core's shortcuts, in order. A quote prices each; until then they show
    /// disabled rather than appearing once it arrives.
    private static let shortcutPercentages = sendAmountShortcutPercentages()

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SendAssetContextView(store: store)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                SendFlowPageHeading(title: "How much are you sending?")
                SendRecipientIdentityView(
                    store: store, chain: presentation.selectedCoin?.chainId,
                    address: store.sendFlow.address, compact: true)
            }

            VStack(spacing: SpectraLayout.Space.l) {
                if let coin = presentation.selectedCoin {
                    CoinBadge(artworkName: coin.artworkName, fallbackText: coin.symbol, color: coin.color, size: 44)
                }
                amountField

                VStack(spacing: SpectraLayout.Space.xs) {
                    Text(AppLocalization.string("Available to send")).font(.caption).foregroundStyle(.secondary)
                    Text(maximumText).font(.headline).spectraNumericTextLayout()
                }
                .padding(.top, SpectraLayout.Space.s)

                if presentation.selectedCoin?.hasBalance == true {
                    GlassEffectContainer(spacing: SpectraLayout.Space.s) {
                        if dynamicTypeSize.isAccessibilitySize {
                            LazyVGrid(columns: [
                                GridItem(.flexible(), spacing: SpectraLayout.Space.s),
                                GridItem(.flexible(), spacing: SpectraLayout.Space.s)
                            ], spacing: SpectraLayout.Space.s) {
                                ForEach(Self.shortcutPercentages, id: \.self) { percentage in
                                    percentButton(percentage: percentage)
                                }
                            }
                        } else {
                            HStack(spacing: SpectraLayout.Space.s) {
                                ForEach(Self.shortcutPercentages, id: \.self) { percentage in
                                    percentButton(percentage: percentage)
                                }
                            }
                        }
                    }
                }

                if !store.sendFlow.amount.isEmpty && !store.sendAmountIsValid {
                    Text(AppLocalization.string("Enter a positive decimal amount within this asset's precision."))
                        .font(.caption).foregroundStyle(.red)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(.vertical, SpectraLayout.Space.xl)
            .padding(.horizontal, SpectraLayout.cardPadding)
            .frame(maxWidth: .infinity)
            .spectraElevatedFill()

            if let amountText = presentation.selectedCoinAmountText {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                    amountRow("Wallet balance", value: amountText)
                    Text(AppLocalization.string("Maximum amount accounts for estimated network fees."))
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(SpectraLayout.cardPadding)
                .frame(maxWidth: .infinity, alignment: .leading)
                .spectraCardFill()
            }
        }
    }

    private var amountField: some View {
        VStack(spacing: SpectraLayout.Space.s) {
            (dynamicTypeSize.isAccessibilitySize
                ? AnyLayout(VStackLayout(spacing: SpectraLayout.Space.s))
                : AnyLayout(HStackLayout(spacing: SpectraLayout.Space.s))) {
                // An amount must show every entered digit, including at large
                // text sizes; wrapping is preferable to an ellipsis.
                TextField("0", text: Bindable(store.sendFlow).amount, axis: .vertical)
                    .lineLimit(1...)
                    .keyboardType(.decimalPad)
                    .font(.system(size: amountFontSize, weight: .semibold))
                    .accessibilityLabel(AppLocalization.string("Amount"))
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity)

                if let selectedCoin = presentation.selectedCoin {
                    Text(selectedCoin.symbol)
                        .font(.title3.weight(.medium))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: true, vertical: false)
                        .layoutPriority(1)
                }
            }
            .frame(maxWidth: .infinity)
            // Always present once there is an amount: "—" says the asset has
            // no price, where a missing line said nothing at all.
            if !store.sendFlow.amount.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text("≈ \(presentation.selectedCoinApproximateFiatText ?? "—")")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .spectraNumericTextLayout()
            }
        }
    }

    /// The fee-adjusted maximum at the asset's display precision; the exact
    /// figure is what MAX fills in.
    private var maximumText: String {
        if store.sendFlow.isPreparingPreview { return AppLocalization.string("Estimating…") }
        guard quoteIsCurrent, let maximum = store.sendShortcutAmount(percentage: 100),
              let coin = presentation.selectedCoin else { return "—" }
        return store.amounts.formattedAssetAmount(maximum, symbol: coin.symbol, deploymentId: coin.holdingKey)
    }

    private func amountRow(_ label: String, value: String) -> some View {
        HStack {
            Text(AppLocalization.string(label)).font(.subheadline).foregroundStyle(.secondary)
            Spacer(minLength: SpectraLayout.Space.s)
            Text(value).font(.subheadline.weight(.semibold)).spectraNumericTextLayout()
        }
    }

    private func percentButton(percentage: UInt32) -> some View {
        let amount = quoteIsCurrent ? store.sendShortcutAmount(percentage: percentage) : nil
        return Button {
            guard let amount else { return }
            store.sendFlow.amount = amount
            spectraHaptic(.light)
        } label: {
            Text(percentage == 100 ? AppLocalization.string("Max") : "\(percentage)%")
                .font(.subheadline.weight(.semibold))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity, minHeight: 36)
        }
        .buttonStyle(.glass)
        .disabled(amount == nil)
        .accessibilityLabel(percentage == 100
            ? AppLocalization.string("Maximum after estimated fees")
            : AppLocalization.format("%lld percent of estimated maximum", Int(percentage)))
    }
}

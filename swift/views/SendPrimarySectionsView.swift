import Foundation
import SwiftUI
import VisionKit

@MainActor
fileprivate struct SendComposerPresentation {
    let sendWallets: [WalletView]
    let selectedWallet: WalletView?
    let availableSendCoins: [Coin]
    let selectedCoin: Coin?
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
    private static let assetBadgeSize: CGFloat = 28

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            walletRow
            if !presentation.availableSendCoins.isEmpty {
                Divider().opacity(0.3)
                sendComposerSectionLabel("Asset")
                VStack(spacing: 0) {
                    ForEach(Array(presentation.availableSendCoins.enumerated()), id: \.element.holdingKey) { index, coin in
                        if index > 0 { Divider().padding(.leading, Self.assetBadgeSize + SpectraLayout.Space.m).opacity(0.3) }
                        assetRow(coin: coin, isSelected: coin.holdingKey == store.sendFlow.holdingKey)
                    }
                }
            }
        }
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }

    private var walletRow: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            if let selectedWallet = presentation.selectedWallet {
                let badge = Coin.nativeChainBadge(for: selectedWallet.family) ?? (nil, Color.mint)
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
                Picker(AppLocalization.string("Wallet"), selection: Bindable(store.sendFlow).walletId) {
                    ForEach(presentation.sendWallets) { wallet in Text(wallet.name).tag(wallet.id) }
                }
                .pickerStyle(.menu)
                .labelsHidden()
                .onChange(of: store.sendFlow.walletId) { _, _ in store.syncSendAssetSelection() }
            }
        }
    }

    /// One row per asset, full width.
    private func assetRow(coin: Coin, isSelected: Bool) -> some View {
        Button {
            guard !isSelected else { return }
            store.sendFlow.holdingKey = coin.holdingKey
            spectraHaptic(.light)
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(artworkName: coin.artworkName, fallbackText: coin.symbol, color: coin.color, size: Self.assetBadgeSize)
                Text(coin.symbol).font(.subheadline.weight(.semibold))
                Spacer(minLength: SpectraLayout.Space.s)
                Text(store.amounts.formattedAssetAmount(coin.amount, symbol: coin.symbol, deploymentId: coin.holdingKey))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .spectraNumericTextLayout()
                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                    .font(.body)
                    .foregroundStyle(isSelected ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
            }
            .padding(.vertical, SpectraLayout.Space.s)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
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
    @Binding var selectedAddressBookEntryId: String
    @Binding var isShowingQRScanner: Bool
    @Binding var qrScannerErrorMessage: String?
    let validationError: String?
    let isValidating: Bool
    /// Core's resolution of the recipient as typed, once it has one.
    let validatedResolution: SendDestinationResolution?
    let retryValidation: () -> Void
    @FocusState private var addressFocused: Bool

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        toCard
    }

    private var toCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            sendComposerSectionLabel("To")

            HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
                // Wraps rather than scrolls, so the whole address is on screen
                // to compare against the one it was copied from.
                TextField(AppLocalization.string("Recipient address"), text: Bindable(store.sendFlow).address, axis: .vertical)
                    .lineLimit(1...3)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.subheadline.monospaced())
                    .focused($addressFocused)
                    .submitLabel(.done)
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

                Button {
                    guard DataScannerViewController.isSupported else {
                        qrScannerErrorMessage = AppLocalization.string("QR scanning is not supported on this device.")
                        return
                    }
                    guard DataScannerViewController.isAvailable else {
                        qrScannerErrorMessage = AppLocalization.string(
                            "QR scanning is unavailable right now. Check camera permission and try again.")
                        return
                    }
                    isShowingQRScanner = true
                } label: {
                    Image(systemName: "qrcode.viewfinder")
                        .font(.title3.weight(.semibold))
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(.glass)
                .accessibilityLabel(AppLocalization.string("Scan QR Code"))
            }

            if !presentation.addressBookEntries.isEmpty {
                Picker(AppLocalization.string("Saved Recipient"), selection: $selectedAddressBookEntryId) {
                    Text(AppLocalization.string("None")).tag("")
                    ForEach(presentation.addressBookEntries) { entry in
                        Text("\(entry.name) · \(entry.chainName)").tag(entry.id)
                    }
                }
                .pickerStyle(.menu)
                .font(.subheadline)
                .onChange(of: selectedAddressBookEntryId) { _, newValue in
                    guard let entry = presentation.addressBookEntries.first(where: { $0.id == newValue }) else { return }
                    store.sendFlow.address = entry.address
                }
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

    /// Core's shortcuts, in order. A quote prices each; until then they show
    /// disabled rather than appearing once it arrives.
    private static let shortcutPercentages = sendAmountShortcutPercentages()

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }

    var body: some View {
        VStack(spacing: SpectraLayout.Space.m) {
            amountField

            Divider().opacity(0.3)

            VStack(spacing: SpectraLayout.Space.s) {
                if let amountText = presentation.selectedCoinAmountText {
                    amountRow("Balance", value: amountText)
                }
                amountRow("Available to send", value: maximumText)
            }

            if presentation.selectedCoin?.hasBalance == true {
                HStack(spacing: SpectraLayout.Space.s) {
                    ForEach(Self.shortcutPercentages, id: \.self) { percentage in
                        percentButton(percentage: percentage)
                    }
                }
            }

            if !store.sendFlow.amount.isEmpty && !store.sendAmountIsValid {
                Text(AppLocalization.string("Enter a positive decimal amount within this asset's precision."))
                    .font(.caption).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity)
        .spectraElevatedFill()
    }

    private var amountField: some View {
        VStack(alignment: .trailing, spacing: SpectraLayout.Space.xs) {
            (dynamicTypeSize.isAccessibilitySize
                ? AnyLayout(VStackLayout(alignment: .trailing, spacing: SpectraLayout.Space.s))
                : AnyLayout(HStackLayout(spacing: SpectraLayout.Space.s))) {
                TextField("0", text: Bindable(store.sendFlow).amount)
                    .keyboardType(.decimalPad)
                    .font(.largeTitle.weight(.semibold))
                    .accessibilityLabel(AppLocalization.string("Amount"))
                    .multilineTextAlignment(.trailing)
                    .spectraNumericTextLayout()
                    .frame(maxWidth: .infinity)

                if let selectedCoin = presentation.selectedCoin {
                    Text(selectedCoin.symbol)
                        .font(.headline)
                        .foregroundStyle(.secondary)
                }
            }
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
                .frame(maxWidth: .infinity, minHeight: 36)
        }
        .buttonStyle(.glass)
        .disabled(amount == nil)
        .accessibilityLabel(percentage == 100
            ? AppLocalization.string("Maximum after estimated fees")
            : AppLocalization.format("%lld percent of estimated maximum", Int(percentage)))
    }
}

import SwiftUI

/// The composer's amount: typed, or a share of what the quote says can be sent.
@MainActor
struct SendAmountPage: View {
    @Bindable var store: AppState
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .largeTitle) private var amountFontSize: CGFloat = 56
    @FocusState private var isAmountFocused: Bool

    /// Core's shortcuts, in order. A quote prices each; until then they show
    /// disabled rather than appearing once it arrives.
    private static let shortcutPercentages = sendAmountShortcutPercentages()

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }
    private var quoteIsCurrent: Bool { store.sendFlow.quoteIsCurrent }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SendAssetContextView(store: store)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                FlowPageHeading(title: "How much are you sending?")
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
                    // Why the maximum is not the balance: a coin pays its own
                    // fee. A token's maximum is its balance.
                    if let coin = presentation.selectedCoin, coin.isNativeCoin,
                       let balance = presentation.selectedCoinAmountText {
                        Text(AppLocalization.format("Wallet balance %@, less the network fee", balance))
                            .font(.caption).foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
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

                amountProblem
                    .font(.caption)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.vertical, SpectraLayout.Space.xl)
            .padding(.horizontal, SpectraLayout.cardPadding)
            .frame(maxWidth: .infinity)
            .spectraElevatedFill()
            .keyboardAnchor()
        }
    }

    /// The amount with its unit beside it, centred together. The field is
    /// as wide as its digits and their size steps down as they grow, so a
    /// long amount stays on one line beside its unit; at accessibility sizes
    /// it wraps instead, with the unit under it, and never cuts a digit.
    private var amountField: some View {
        VStack(spacing: SpectraLayout.Space.s) {
            if dynamicTypeSize.isAccessibilitySize {
                VStack(spacing: SpectraLayout.Space.s) {
                    TextField("0", text: Bindable(store.sendFlow).amount, axis: .vertical)
                        .lineLimit(1...)
                        .keyboardType(.decimalPad)
                        .font(.system(size: amountFontSize, weight: .semibold))
                        .accessibilityLabel(AppLocalization.string("Amount"))
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                    unitLabel
                }
            } else {
                HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                    TextField("0", text: Bindable(store.sendFlow).amount)
                        .keyboardType(.decimalPad)
                        .font(.system(size: fittedAmountFontSize, weight: .semibold))
                        .accessibilityLabel(AppLocalization.string("Amount"))
                        .focused($isAmountFocused)
                        .fixedSize()
                    unitLabel
                }
                .frame(maxWidth: .infinity)
                // The field is as wide as its digits; the whole row takes
                // the tap, as the full-width field it replaced did.
                .contentShape(Rectangle())
                .onTapGesture { isAmountFocused = true }
            }
            approximateValue
        }
    }

    /// Eight digits at the full size, then smaller as more are typed, down
    /// to a size that still fits an asset's full precision on the line.
    private var fittedAmountFontSize: CGFloat {
        let digits = max(store.sendFlow.amount.count, 1)
        return amountFontSize * min(1, max(0.35, 8 / CGFloat(digits)))
    }

    @ViewBuilder
    private var unitLabel: some View {
        if let selectedCoin = presentation.selectedCoin {
            Text(selectedCoin.symbol)
                .font(.title3.weight(.medium))
                .foregroundStyle(.secondary)
                .fixedSize()
        }
    }

    /// The amount in the display currency once core has priced it. An asset
    /// with no price says so in words; a quote still on its way keeps the
    /// line's room without a value, so nothing below it moves.
    @ViewBuilder
    private var approximateValue: some View {
        if !store.sendFlow.amount.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            let quoted = quoteIsCurrent && store.sendQuoteForEnteredAmount != nil
            Text(presentation.selectedCoinApproximateFiatText.map { "≈ \($0)" }
                ?? AppLocalization.string("No price available"))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .spectraNumericTextLayout()
                .opacity(quoted || presentation.selectedCoinApproximateFiatText != nil ? 1 : 0)
        }
    }

    /// What stands between this amount and Review, under the field it is
    /// about: a malformed amount, then core's refusal of it, then a quote that
    /// did not come back.
    @ViewBuilder
    private var amountProblem: some View {
        if !store.sendFlow.amount.isEmpty && !store.sendAmountIsValid {
            Label(AppLocalization.string("Enter a positive decimal amount within this asset's precision."),
                  systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
        } else if let refusal = store.sendAmountRefusal {
            Label(refusal, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red)
        } else if let error = store.sendFlow.previewError {
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red)
                Spacer(minLength: 0)
                Button(AppLocalization.string("Retry"), action: store.sendFlow.schedulePreview).fontWeight(.semibold)
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

    private func percentButton(percentage: UInt32) -> some View {
        let amount = quoteIsCurrent ? store.sendShortcutAmount(percentage: percentage) : nil
        return Button {
            guard let amount else { return }
            store.sendFlow.amount = AmountPresentation.decimalFieldText(amount)
            spectraHaptic(.light)
        } label: {
            Text(percentage == 100
                ? AppLocalization.string("Max")
                : (Double(percentage) / 100).formatted(.percent.locale(AppLocalization.locale)))
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

import SwiftUI

/// The send flow's network page: fee estimates and the per-chain options that
/// go with them.
///
/// Nothing here reads the flow's own state — the step, the scanner, the
/// address-book selection — so it is its own view, a diffing boundary: a step
/// change does not re-evaluate every fee row.
struct SendNetworkStep: View {
    @Bindable var store: AppState

    /// The quote for the selected holding; nil while none is current.
    private var quote: OwnedSendPreview? { store.sendQuote }

    private func hasNetworkSendSections(for coin: AssetHolding?) -> Bool {
        coin?.chain.hasSendPreview ?? false
    }

    /// The quoted fee, with its display-currency value when core had one.
    private func networkFeeText(_ quote: OwnedSendPreview, chain: Chain) -> String? {
        guard let fee = quote.networkFee else { return nil }
        return AppLocalization.format(
            "Estimated Network Fee: %@", store.amounts.compactNetworkFee(fee, value: quote.networkFeeValue, chain: chain))
    }

    var body: some View {
        networkStep(selectedCoin: selectedCoin)
    }

    private var selectedCoin: AssetHolding? {
        store.availableSendCoins(for: store.sendFlow.walletId).first(where: { $0.holdingKey == store.sendFlow.holdingKey })
    }

    @ViewBuilder
    private func networkStep(selectedCoin: AssetHolding?) -> some View {
        // Shown inside the review's disclosure, which already names it: no
        // page header of its own.
        if hasNetworkSendSections(for: selectedCoin) {
            networkCard(selectedCoin: selectedCoin)
        } else {
            noNetworkPreviewCard(selectedCoin: selectedCoin)
        }
    }

    private func noNetworkPreviewCard(selectedCoin: AssetHolding?) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            networkSectionHeader("Network")
            if let selectedCoin {
                Text(AppLocalization.format("Spectra will prepare the %@ transfer with the default %@ network policy.", selectedCoin.symbol, selectedCoin.chainName))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            } else {
                Text(AppLocalization.string("Select an asset to load network details."))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    // MARK: - Network fee card

    @ViewBuilder
    private func networkCard(selectedCoin: AssetHolding?) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            networkCardContent(selectedCoin: selectedCoin)
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }

    /// One branch per preview shape. The chain decides which, through the
    /// registry; nothing here names one.
    @ViewBuilder
    private func networkCardContent(selectedCoin: AssetHolding?) -> some View {
        if let selectedCoin {
            let chain = selectedCoin.chain
            if chain.isEVM {
                evmNetworkContent(selectedCoin: selectedCoin)
            } else {
                simpleFeeContent(selectedCoin: selectedCoin, chain: chain)
            }
            sendPreviewDetailsContent(for: selectedCoin)
        }
    }

    // MARK: — Network sub-sections

    /// A typed value that keeps its name and unit beside it once filled: a
    /// placeholder alone left Max Fee and Priority Fee looking the same. The
    /// placeholder is the quote's value, not the name beside it again.
    private func labeledField(
        _ title: String, unit: String?, estimate: String?, text: Binding<String>, keyboard: UIKeyboardType
    ) -> some View {
        HStack(spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string(title)).font(.subheadline).foregroundStyle(.secondary)
            TextField(text: text, prompt: Text(verbatim: estimate ?? "")) { Text(AppLocalization.string(title)) }
                .keyboardType(keyboard).multilineTextAlignment(.trailing).monospacedDigit()
            if let unit { Text(verbatim: unit).font(.subheadline).foregroundStyle(.secondary) }
        }
        .padding(.horizontal, SpectraLayout.Space.m).padding(.vertical, SpectraLayout.Space.s)
        .frame(minHeight: 44)
        .spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
    }

    @ViewBuilder
    private func evmNetworkContent(selectedCoin: AssetHolding) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            networkSectionHeader(AppLocalization.format("%@ Network", selectedCoin.chainName))
            let terms = store.sendFlow.quotedEvmTerms
            Toggle(AppLocalization.string("Use Custom Fees"), isOn: Bindable(store.sendFlow).useCustomEvmFees)
            if store.sendFlow.useCustomEvmFees {
                labeledField(
                    "Max Fee", unit: "gwei", estimate: terms.map { AmountPresentation.decimalFieldText($0.maxFeePerGasGwei) },
                    text: Bindable(store.sendFlow).customEvmMaxFeeGwei, keyboard: .decimalPad)
                labeledField(
                    "Priority Fee", unit: "gwei",
                    estimate: terms.map { AmountPresentation.decimalFieldText($0.maxPriorityFeePerGasGwei) },
                    text: Bindable(store.sendFlow).customEvmPriorityFeeGwei, keyboard: .decimalPad)
                if let customEvmFeeValidationError = store.sendFlow.customEvmFeeValidationError {
                    Text(customEvmFeeValidationError).font(.caption).foregroundStyle(.red)
                } else {
                    Text(AppLocalization.string("Custom EIP-1559 fees are applied to this send and preview."))
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Toggle(AppLocalization.string("Manual Nonce"), isOn: Bindable(store.sendFlow).evmManualNonceEnabled)
            if store.sendFlow.evmManualNonceEnabled {
                labeledField(
                    "Nonce", unit: nil, estimate: terms.map { String($0.nonce) },
                    text: Bindable(store.sendFlow).evmManualNonce, keyboard: .numberPad)
                if let evmNonceValidationError = store.sendFlow.evmNonceValidationError {
                    Text(evmNonceValidationError).font(.caption).foregroundStyle(.red)
                }
            }
            if store.sendFlow.isPreparingPreview {
                SpectraLoadingRow(title: "Loading nonce and fee estimate...")
            } else if let quote, case .ethereum(let evmSendPreview) = quote.preview {
                Divider().opacity(0.3)
                valueRow("Nonce", "\(evmSendPreview.nonce)")
                valueRow("Gas Limit", evmSendPreview.gasLimit.formatted(.number.locale(AppLocalization.locale)))
                valueRow("Max Fee", store.amounts.compactGasPrice(gwei: evmSendPreview.maxFeePerGasGwei))
                valueRow("Priority Fee", store.amounts.compactGasPrice(gwei: evmSendPreview.maxPriorityFeePerGasGwei))
            } else if let error = store.sendFlow.previewError {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.caption).foregroundStyle(.red)
            } else {
                // The amount and recipient are in by this page; a missing
                // quote is one still on its way.
                SpectraLoadingRow(title: "Loading nonce and fee estimate...")
            }
        }
    }

    /// The fee card for every chain but the EVM family, whose card carries
    /// fee and nonce overrides. A UTXO preview's rate, size, inputs and change
    /// are the detail rows below it.
    @ViewBuilder
    private func simpleFeeContent(selectedCoin: AssetHolding, chain: Chain) -> some View {
        let chainName = selectedCoin.chainName
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            networkSectionHeader(AppLocalization.format("%@ Network", chainName))
            if store.sendFlow.isPreparingPreview {
                SpectraLoadingRow(title: AppLocalization.format("Loading %@ fee estimate...", chainName))
            } else if let quote {
                if let fee = networkFeeText(quote, chain: chain) {
                    Text(fee).font(.subheadline.weight(.semibold))
                }
                ForEach(previewDetailLines(quote.preview), id: \.self) { Text($0) }
                if !selectedCoin.isNativeCoin {
                    Text(AppLocalization.format("Token transfers on %@ pay network fees in %@. Keep a %@ balance for fees.", chainName, chain.gasTokenSymbol, chain.gasTokenSymbol))
                        .font(.caption).foregroundStyle(.secondary)
                }
            } else if let error = store.sendFlow.previewError {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.caption).foregroundStyle(.red)
            } else {
                SpectraLoadingRow(title: AppLocalization.format("Loading %@ fee estimate...", chainName))
            }
            Text(AppLocalization.format("Spectra signs and broadcasts %@ transfers in-app.", chain.displayName))
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    /// The chain-specific fields a preview carries beside its fee.
    ///
    /// Exhaustive over core's enum on purpose: a new preview variant is a
    /// compile error here rather than a card that quietly shows only the fee.
    private func previewDetailLines(_ preview: SendPreview) -> [String] {
        switch preview {
        case .monero(let p):
            return [AppLocalization.format("Priority: %@", p.priorityLabel)]
        case .sui(let p):
            return [
                AppLocalization.format("Gas Budget: %llu MIST", p.gasBudgetMist),
                AppLocalization.format("Reference Gas Price: %llu", p.referenceGasPrice),
            ]
        case .aptos(let p):
            return [
                AppLocalization.format("Max Gas Amount: %llu", p.maxGasAmount),
                AppLocalization.format("Gas Unit Price: %llu octas", p.gasUnitPriceOctas),
            ]
        case .utxo, .ethereum, .tron, .solana, .xrp, .stellar, .cardano, .ton, .icp, .near,
             .polkadot, .bittensor:
            return []
        }
    }

    /// What the fee section above has not already said. The EVM rows state
    /// the fee rate, and the amount page states the balance and the maximum,
    /// so those are not repeated here — the repeat of the rate rounded the
    /// priority fee the rows gave as 0.001 gwei to "0.00".
    @ViewBuilder
    private func sendPreviewDetailsContent(for selectedCoin: AssetHolding) -> some View {
        let isEVM = selectedCoin.chain.isEVM
        if let details = store.sendPreviewDetails(for: selectedCoin), details.hasDetailRows(isEVM: isEVM) {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                Divider().opacity(0.3).padding(.vertical, SpectraLayout.Space.xs)
                if let feeRateDescription = details.feeRateDescription, !isEVM {
                    valueRow("Fee Rate", feeRateDescription)
                }
                if let estimatedTransactionBytes = details.estimatedTransactionBytes {
                    valueRow("Estimated Size", AppLocalization.format("%lld bytes", count: Int(estimatedTransactionBytes), estimatedTransactionBytes))
                }
                if let selectedInputCount = details.selectedInputCount { valueRow("Selected Inputs", "\(selectedInputCount)") }
                if let usesChangeOutput = details.usesChangeOutput {
                    valueRow("Change Output", AppLocalization.string(usesChangeOutput ? "Yes" : "No"))
                }
            }
        }
    }

    private func valueRow(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(AppLocalization.string(label)).font(.subheadline).foregroundStyle(.secondary)
            Spacer(minLength: SpectraLayout.Space.s)
            Text(value).font(.subheadline.weight(.semibold)).multilineTextAlignment(.trailing)
        }
    }
}

/// Shared by the network card and the no-preview card that replaces it.
@ViewBuilder
private func networkSectionHeader(_ title: String) -> some View {
    Text(AppLocalization.string(title))
        .font(.caption.weight(.semibold))
        .foregroundStyle(.secondary)
        .textCase(.uppercase)
        .padding(.bottom, SpectraLayout.Space.s)
}

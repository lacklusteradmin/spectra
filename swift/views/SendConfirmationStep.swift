import SwiftUI

/// The pre-build review form.
struct SendConfirmationStep: View {
    @Bindable var store: AppState
    let quoteIsCurrent: Bool
    let recipientAddress: String
    let retryQuote: () -> Void
    private var selectedCoin: AssetHolding? {
        store.availableSendCoins(for: store.sendFlow.walletId).first(where: { $0.holdingKey == store.sendFlow.holdingKey })
    }

    var body: some View {
        confirmStep(selectedCoin: selectedCoin)
    }

    private func confirmStep(selectedCoin: AssetHolding?) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            amountCard(selectedCoin: selectedCoin)
            if let selectedCoin {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    SendTransferPartiesView(
                        store: store, walletId: store.sendFlow.walletId, chain: selectedCoin.chain,
                        sender: store.selectedWalletForSend()?.address(on: selectedCoin.chain),
                        recipient: recipientAddress)
                    Divider().opacity(0.4)
                    SendCostRows(fee: networkFeeText, total: totalText)
                    if let error = store.sendFlow.previewError {
                        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                            Label(error, systemImage: "exclamationmark.triangle.fill")
                                .font(.caption).foregroundStyle(.red)
                            Spacer(minLength: 0)
                            Button(AppLocalization.string("Retry"), action: retryQuote)
                                .font(.caption.weight(.semibold))
                        }
                    } else if let refusal = store.sendAmountRefusal {
                        Label(refusal, systemImage: "exclamationmark.triangle.fill")
                            .font(.caption).foregroundStyle(.red)
                    }
                }
                .padding(SpectraLayout.cardPadding)
                .spectraCardFill()
            }
            // Under the parties they are about, before anything else on the
            // page, as the built transaction's review puts its warnings.
            ForEach(store.sendFlow.destinationWarnings, id: \.self) { warning in
                Label(warning, systemImage: "exclamationmark.triangle.fill")
                    .font(.subheadline)
                    .foregroundStyle(.spectraWarning)
            }
            DisclosureGroup(AppLocalization.string("Fee and advanced settings")) {
                SendNetworkStep(store: store)
                    .keyboardAnchor()
            }
            .font(.subheadline.weight(.semibold))
            .disclosureGroupStyle(.spectra)

            Label(AppLocalization.string("Check the full address and amount. You can review the built transaction again before signing."), systemImage: "checkmark.shield")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if store.sendFlow.session.isBusy {
                SpectraLoadingRow(
                    title: "Preparing transaction...",
                    subtitle: "Keep this screen open while Spectra prepares the transfer."
                )
            }
        }
    }

    @ViewBuilder
    private func amountCard(selectedCoin: AssetHolding?) -> some View {
        if let selectedCoin {
            SendAmountSummaryView(
                artworkName: selectedCoin.artworkName, symbol: selectedCoin.symbol,
                chain: selectedCoin.chain, amount: store.sendFlow.amountInput,
                fiatText: store.amounts.formattedFiatIfAvailable(confirmedQuote?.amountValue).map { "≈ \($0)" })
        }
    }

    private var confirmedQuote: OwnedSendPreview? { quoteIsCurrent ? store.sendQuoteForEnteredAmount : nil }

    /// The fee as far as it is known: being quoted, quoted, unavailable, or —
    /// on a network whose fee only the built transaction fixes — set at build.
    /// A quote that is not yet asked for (typing pauses first) is on its way,
    /// not absent; an override still being typed is answered beside its field.
    private var networkFeeText: String {
        if store.sendFlow.evmNonceValidationError != nil || store.sendFlow.customEvmFeeValidationError != nil {
            return AppLocalization.string("Unavailable")
        }
        if store.sendFlow.previewError != nil { return AppLocalization.string("Unavailable") }
        if store.sendFlow.isPreparingPreview || !quoteIsCurrent { return AppLocalization.string("Estimating…") }
        guard let quote = confirmedQuote, let fee = quote.networkFee else {
            return AppLocalization.string("Set when the transaction is built")
        }
        return "≈ " + store.amounts.compactNetworkFee(fee, value: quote.networkFeeValue, chain: quote.chainId)
    }

    /// The amount and the quoted fee together, when the coin pays its own
    /// fee, at the coin's display precision as every amount is. The row
    /// stays while the fee is being quoted, so the card does not jump.
    private var totalText: String? {
        guard let coin = selectedCoin, coin.isNativeCoin else { return nil }
        guard let total = confirmedQuote?.total, !store.sendFlow.isPreparingPreview,
              store.sendFlow.previewError == nil, quoteIsCurrent else { return networkFeeText }
        return "≈ " + store.amounts.formattedAssetAmount(total, symbol: coin.symbol, deploymentId: coin.holdingKey)
    }
}

/// Transient errors and core-derived confirmation notices.
struct SendStatusCards: View {
    let store: AppState

    var body: some View {
        sendStatusCards
    }

    @ViewBuilder
    private var sendStatusCards: some View {
        if let sendError = store.sendFlow.session.error {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
                Text(sendError).font(.subheadline).foregroundStyle(.red)
            }
            .padding(SpectraLayout.Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassEffect(.regular.tint(.red.opacity(0.06)), in: .rect(cornerRadius: SpectraLayout.Radius.card))
        }

        if let sendVerificationNotice = store.sendFlow.verificationNotice {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: "exclamationmark.circle.fill")
                    .foregroundStyle(store.sendFlow.verificationNoticeIsWarning ? .red : .spectraWarning)
                Text(sendVerificationNotice).font(.subheadline)
                    .foregroundStyle(store.sendFlow.verificationNoticeIsWarning ? .red : .spectraWarning)
            }
            .padding(SpectraLayout.Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassEffect(.regular.tint(.spectraWarning.opacity(0.06)), in: .rect(cornerRadius: SpectraLayout.Radius.card))
        }


    }

}

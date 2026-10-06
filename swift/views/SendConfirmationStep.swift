import SwiftUI

/// The pre-build review form.
struct SendConfirmationStep: View {
    @Bindable var store: AppState
    let quoteIsCurrent: Bool
    let recipientAddress: String

    private var isSendBusy: Bool { store.sendFlow.session.isBusy || store.sendFlow.isPreparingPreview }
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
                    Divider().opacity(0.35)
                    confirmationRow(label: "Network Fee", value: networkFeeText ?? AppLocalization.string("Estimating…"))
                }
                .padding(SpectraLayout.cardPadding)
                .spectraCardFill()
            }
            DisclosureGroup(AppLocalization.string("Fee and advanced settings")) {
                SendNetworkStep(store: store)
                    .padding(.top, SpectraLayout.Space.s)
            }
            .font(.subheadline.weight(.semibold))
            .padding(.horizontal, SpectraLayout.Space.xs)

            Label(AppLocalization.string("Check the full address and amount. You can review the built transaction again before signing."), systemImage: "viewfinder")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if store.sendFlow.isCheckingDestination || isSendBusy {
                SpectraLoadingRow(
                    title: isSendBusy ? "Preparing transaction..." : "Checking recipient...",
                    subtitle: isSendBusy ? "Keep this screen open while Spectra prepares the transfer." : nil
                )
            }
            if let warning = store.sendFlow.destinationRiskWarning {
                Label(warning, systemImage: "exclamationmark.triangle.fill")
                    .font(.subheadline)
                    .foregroundStyle(.spectraWarning)
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

    private func confirmationRow(label: String, value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string(label)).font(.subheadline).foregroundStyle(.secondary)
            Spacer(minLength: SpectraLayout.Space.s)
            Text(value)
                .font(.subheadline.weight(.semibold))
                .multilineTextAlignment(.trailing)
                .spectraNumericTextLayout(minimumScaleFactor: 0.8)
        }
    }

    private var confirmedQuote: OwnedSendPreview? { quoteIsCurrent ? store.sendQuoteForEnteredAmount : nil }

    private var networkFeeText: String? {
        guard let quote = confirmedQuote, let fee = quote.networkFee else { return nil }
        let chain = quote.chainId
        return store.amounts.compactNetworkFee(fee, value: quote.networkFeeValue, chain: chain)
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

import SwiftUI
import UIKit

/// A projection of core's durable artifact and history record. Node acceptance
/// describes submission only; the history record alone supplies confirmation.
struct SendStagesView: View {
    @Bindable var store: AppState
    let artifact: SendArtifact
    var transaction: TransactionRecord? = nil
    var transactionError: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            if !hasFinalHistoryStatus { stageProgress }
            amountSummary
            if let terms = artifact.review.transferTerms { transferTerms(terms) }

            if showsImmediateNodeChoice { broadcastDestinations }

            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                SendTransferPartiesView(
                    store: store, walletId: artifact.walletId, chain: artifact.chainId,
                    sender: artifact.sender, recipient: artifact.recipient, memo: artifact.memo,
                    // Offered once the send is on record, named for what it sent;
                    // the sheet asks before anything is saved.
                    suggestedContactName: displayedTransaction.map { _ in
                        AppLocalization.format("%@ Recipient", artifact.symbol)
                    })
                // What the quote the send was built from estimated, as the
                // review put it. A receipt states the fee paid instead.
                if !hasFinalHistoryStatus, let fee = artifact.review.networkFee {
                    Divider().opacity(0.4)
                    SendCostRows(
                        fee: "≈ " + store.amounts.compactNetworkFee(
                            fee, value: artifact.review.networkFeeValue, chain: artifact.chainId),
                        total: artifact.review.total.map {
                            "≈ " + store.amounts.formattedAssetAmount(
                                $0, symbol: artifact.symbol, deploymentId: artifact.chainId.entry?.nativeDeploymentId)
                        })
                }
            }
            .padding(SpectraLayout.cardPadding)
            .spectraCardFill()

            if artifact.attempts.isEmpty { reviewWarnings }

            if hasFinalHistoryStatus {
                transactionReceipt
                if !artifact.attempts.isEmpty {
                    DisclosureGroup(AppLocalization.string("View submission results")) {
                        submissionResults
                    }
                    .font(.subheadline.weight(.semibold))
                    .disclosureGroupStyle(.spectra)
                }
            } else if !artifact.attempts.isEmpty {
                submissionResults
            }

            technicalDetails
            stageNotice

            if showsRetryNodeChoice {
                DisclosureGroup(AppLocalization.string("Retry submission")) {
                    broadcastDestinations
                }
                .font(.subheadline.weight(.semibold))
                .disclosureGroupStyle(.spectra)
            }

            if let transactionError {
                Label(transactionError, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }

    private var displayedTransaction: TransactionRecord? {
        guard transaction?.id == artifact.id else { return nil }
        return transaction
    }

    private var hasFinalHistoryStatus: Bool {
        displayedTransaction?.status == .confirmed || displayedTransaction?.status == .failed
    }

    private var hasAcceptedSubmission: Bool {
        artifact.attempts.contains { $0.outcome == .accepted }
    }

    private var showsImmediateNodeChoice: Bool {
        let action = SendExecutionAction(artifact: artifact, transaction: displayedTransaction)
        return action == .broadcast
            || (action == .retry && SendExecutionAction.canRetry(artifact: artifact, transaction: displayedTransaction))
    }

    private var showsRetryNodeChoice: Bool {
        displayedTransaction?.status != .confirmed && !showsImmediateNodeChoice
            && SendExecutionAction.canRetry(artifact: artifact, transaction: displayedTransaction)
    }

    /// Where a stage stands. Each has its own symbol, as the composer's step
    /// bar does, so the current stage is not told from a waiting one by colour.
    private enum StageState { case done, current, waiting }

    private var stageProgress: some View {
        let signed = artifact.stage != .prepared
        let submitted = !artifact.attempts.isEmpty
        return HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
            progressLabel("Built", state: .done)
            progressLabel(signed ? "Signed" : "Awaiting signing", state: signed ? .done : .current)
            progressLabel(
                submitted ? "Submitted" : "Awaiting submission",
                state: submitted ? .done : (signed ? .current : .waiting))
        }
        .padding(.vertical, SpectraLayout.Space.s)
        .accessibilityIdentifier("send.review.stages")
    }

    private func progressLabel(_ title: String, state: StageState) -> some View {
        VStack(spacing: SpectraLayout.Space.xs) {
            Image(systemName: state == .done ? "checkmark.circle.fill" : (state == .current ? "circle.inset.filled" : "circle"))
                .font(.subheadline.weight(.semibold))
                .accessibilityHidden(true)
            Text(AppLocalization.string(title))
                .font(.caption.weight(state == .current ? .semibold : .regular))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity)
        // As the composer's step bar: what is done and what is current in
        // the accent, what is still to come in grey.
        .foregroundStyle(state == .waiting ? Color.secondary : Color.accentColor)
        .accessibilityElement(children: .combine)
        .accessibilityValue(state == .current ? AppLocalization.string("Current") : "")
    }

    private var amountSummary: some View {
        SendAmountSummaryView(
            artworkName: AssetHolding.nativeChainBadge(for: artifact.chainId)?.artworkName,
            symbol: artifact.symbol, chain: artifact.chainId, amount: artifact.amount,
            statusTitle: statusTitle, statusSystemImage: statusSystemImage,
            statusColor: statusColor, isReceipt: displayedTransaction?.status == .confirmed)
    }

    /// What the token's own rules do between the sender and the recipient.
    private func transferTerms(_ terms: AssetTransferTerms) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            if terms.fee != "0" {
                LabeledContent(AppLocalization.string("Token Fee"),
                               value: "\(AmountPresentation.localizedDecimal(terms.fee)) \(artifact.symbol)")
                Divider().opacity(0.4)
                LabeledContent(AppLocalization.string("Recipient Receives"),
                               value: "\(AmountPresentation.localizedDecimal(terms.received)) \(artifact.symbol)")
            }
            if let carried = terms.carriedNative {
                if terms.fee != "0" { Divider().opacity(0.4) }
                LabeledContent(AppLocalization.string("Sent With It"),
                               value: "\(AmountPresentation.localizedDecimal(carried)) \(artifact.chainId.gasTokenSymbol)")
            }
            if let registration = terms.recipientRegistration {
                if terms.fee != "0" || terms.carriedNative != nil { Divider().opacity(0.4) }
                LabeledContent(AppLocalization.string("Recipient Registration"),
                               value: "\(AmountPresentation.localizedDecimal(registration)) \(artifact.chainId.gasTokenSymbol)")
            }
            if let program = terms.hookProgram {
                if terms.fee != "0" || terms.carriedNative != nil || terms.recipientRegistration != nil {
                    Divider().opacity(0.4)
                }
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                    Label(AppLocalization.string("This token runs a program on every transfer, which can refuse it."),
                          systemImage: "gearshape.2")
                        .foregroundStyle(.spectraWarning)
                    Text(verbatim: program)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                }
            }
        }
        .font(.subheadline)
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    private var statusTitle: String {
        if let record = displayedTransaction {
            switch record.status {
            case .confirmed: return "Confirmed on-chain"
            case .failed: return "Transaction failed"
            case .pending: break
            }
        }
        if artifact.stage == .prepared { return "Built, awaiting signing" }
        if artifact.attempts.isEmpty { return "Signed, not submitted" }
        if hasAcceptedSubmission { return "Awaiting confirmation" }
        return artifact.attempts.contains { $0.outcome == .uncertain }
            ? "Submission uncertain" : "Submission rejected"
    }

    private var statusSystemImage: String {
        if let record = displayedTransaction {
            switch record.status {
            case .confirmed: return "checkmark"
            case .failed: return "exclamationmark.triangle"
            case .pending: break
            }
        }
        if artifact.stage == .prepared { return "doc.text" }
        if artifact.attempts.isEmpty { return "signature" }
        if hasAcceptedSubmission { return "clock" }
        return "exclamationmark.triangle"
    }

    private var statusColor: Color {
        if displayedTransaction?.status == .confirmed { return .green }
        if displayedTransaction?.status == .failed { return .red }
        if !artifact.attempts.isEmpty, !hasAcceptedSubmission,
           !artifact.attempts.contains(where: { $0.outcome == .uncertain }) { return .red }
        return .spectraWarning
    }

    private var broadcastDestinations: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("Select submission nodes")).font(.headline)
            Text(AppLocalization.string("Signed and saved. Choose which nodes receive this transaction."))
                .font(.caption)
                .foregroundStyle(.secondary)
            VStack(spacing: 0) {
                ForEach(Array(store.sendFlow.session.endpoints.enumerated()), id: \.element) { index, endpoint in
                    if index > 0 { Divider().opacity(0.4) }
                    Toggle(isOn: Binding(
                        get: { store.sendFlow.session.selectedEndpoints.contains(endpoint) },
                        set: { selected in
                            if selected { store.sendFlow.session.selectedEndpoints.insert(endpoint) }
                            else { store.sendFlow.session.selectedEndpoints.remove(endpoint) }
                        }
                    )) {
                        Text(verbatim: endpoint)
                            .font(.caption.monospaced())
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                    .padding(.vertical, SpectraLayout.Space.s)
                    .disabled(store.sendFlow.session.isBusy)
                }
            }
            if store.sendFlow.session.selectedEndpoints.isEmpty {
                Label(AppLocalization.string("Select at least one node to submit this transaction."), systemImage: "info.circle")
                    .font(.caption)
                    .foregroundStyle(.spectraWarning)
            }
            Text(AppLocalization.string("Only selected nodes receive this submission from Spectra. Those nodes may propagate the transaction to other nodes."))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(SpectraLayout.cardPadding)
        .spectraCardFill()
    }

    private var submissionResults: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("Submission results")).font(.headline)
            ForEach(Array(artifact.attempts.enumerated()), id: \.offset) { index, attempt in
                if index > 0 { Divider().opacity(0.4) }
                VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                    Text(verbatim: attempt.endpoint)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                    Label(attempt.outcome.title, systemImage: attempt.outcome.systemImage)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(attempt.outcome.color)
                    DisclosureGroup(AppLocalization.string("Submission details")) {
                        Text(verbatim: attempt.detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        if let hash = attempt.transactionHash {
                            transactionHash(hash)
                        }
                    }
                    .font(.caption)
                    .disclosureGroupStyle(.spectra)
                }
            }
            if !hasFinalHistoryStatus, let hash = displayedTransaction?.transactionHash ?? artifact.transactionHash {
                Divider().opacity(0.4)
                transactionHash(hash)
            }
        }
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    @ViewBuilder
    private var transactionReceipt: some View {
        if let record = displayedTransaction {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                LabeledContent(
                    AppLocalization.string(record.status == .confirmed ? "On-chain status" : "Status"),
                    value: record.status.localizedTitle)
                    .font(.subheadline)
                if let reason = record.localizedFailureReason {
                    Text(reason).font(.subheadline).foregroundStyle(.red)
                }
                if let fee = store.amounts.receiptNetworkFeeText(for: record)
                    ?? store.amounts.confirmedNetworkFeeText(for: record) {
                    Divider().opacity(0.4)
                    LabeledContent(AppLocalization.string("Network Fee"), value: fee)
                        .font(.subheadline)
                }
                if let hash = record.transactionHash ?? artifact.transactionHash {
                    Divider().opacity(0.4)
                    transactionHash(hash)
                }
                if let explorer = record.explorerLink {
                    Link(destination: explorer.url) {
                        Label(explorer.label, systemImage: "safari")
                            .font(.subheadline.weight(.semibold))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, SpectraLayout.Space.s)
                    }
                    .buttonStyle(.glass)
                }
            }
            .padding(SpectraLayout.cardPadding)
            .spectraCardFill()
        }
    }

    private func transactionHash(_ hash: String) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string("Transaction Hash"))
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            // Copied from a button in view, as an address is; a long press
            // was the only way, and said nothing when it worked.
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                Text(groupedAddress(hash))
                    .font(.caption.monospaced())
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityLabel(Text(verbatim: hash))
                CopyButton(value: hash)
            }
        }
    }

    private var technicalDetails: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            DisclosureGroup(AppLocalization.string("Transaction details")) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    preparedFieldRows
                    technicalValue("Review digest", value: artifact.reviewDigest)
                    if !artifact.signingPayloadHex.isEmpty {
                        technicalValue("Signing payload", value: artifact.signingPayloadHex)
                    }
                    if !artifact.attempts.isEmpty, store.sendFlow.pendingHighRiskReasons.count > 1 {
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                            Text(AppLocalization.string("Review warnings"))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            reviewWarnings
                        }
                    }
                }
            }
            if let payload = artifact.signedPayload {
                DisclosureGroup(AppLocalization.string("Signed payload")) {
                    technicalText(payload)
                }
            }
        }
        .font(.subheadline.weight(.semibold))
        .disclosureGroupStyle(.spectra)
    }

    /// The prepared transaction field by field, exactly as it will be
    /// signed: core's names and base units, never re-rounded here.
    private var preparedFieldRows: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(AppLocalization.string("Prepared transaction")).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            VStack(spacing: 0) {
                let fields = preparedFields(preparedDetails: artifact.preparedDetails)
                ForEach(Array(fields.enumerated()), id: \.offset) { index, field in
                    if index > 0 { Divider().opacity(0.3) }
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.m) {
                            fieldName(field.name)
                            Spacer(minLength: SpectraLayout.Space.s)
                            fieldValue(field.value).fixedSize()
                        }
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                            fieldName(field.name)
                            fieldValue(field.value).fixedSize(horizontal: false, vertical: true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(.vertical, SpectraLayout.Space.s)
                    .accessibilityElement(children: .combine)
                }
            }
            .padding(.horizontal, SpectraLayout.Space.m)
            .spectraInsetFill()
        }
    }

    @ViewBuilder
    private func fieldName(_ name: String) -> some View {
        if !name.isEmpty {
            Text(verbatim: name).font(.caption.monospaced()).foregroundStyle(.secondary)
        }
    }

    private func fieldValue(_ value: String) -> some View {
        Text(verbatim: breakableAnywhere(value)).font(.caption.monospaced().weight(.regular))
            .accessibilityLabel(Text(verbatim: value))
    }

    private func technicalValue(_ title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(AppLocalization.string(title)).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            technicalText(value)
        }
    }

    /// A digest or payload: wrapped anywhere, so no hyphen appears in it, and
    /// copied whole from its button, since the wrapped text is not the value.
    private func technicalText(_ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
            Text(verbatim: breakableAnywhere(value))
                .font(.caption.monospaced().weight(.regular))
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityLabel(Text(verbatim: value))
            CopyButton(value: value)
        }
    }

    private var reviewWarnings: some View {
        ForEach(Array(store.sendFlow.pendingHighRiskReasons.dropFirst().enumerated()), id: \.offset) { _, reason in
            Label(reason, systemImage: "exclamationmark.triangle")
                .font(.subheadline)
                .foregroundStyle(.spectraWarning)
        }
    }

    @ViewBuilder
    private var stageNotice: some View {
        if artifact.stage == .prepared {
            Label(AppLocalization.string("Review the complete addresses. Signing and broadcasting remain separate actions."), systemImage: "checkmark.shield")
                .font(.caption)
                .foregroundStyle(.secondary)
        } else if !hasFinalHistoryStatus, !artifact.attempts.isEmpty {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                if hasAcceptedSubmission {
                    Label(AppLocalization.string("Node acceptance is not on-chain confirmation."), systemImage: "clock")
                        .foregroundStyle(.spectraWarning)
                }
                Label(AppLocalization.string("Transaction saved. You can close this screen and check its status in transaction history."), systemImage: "info.circle")
                    .foregroundStyle(.secondary)
                if let pendingText = store.pendingTransactionRefreshStatusText {
                    Text(pendingText).foregroundStyle(.secondary)
                }
            }
            .font(.caption)
        }
    }

}

/// What a node said to a submission, as every send and staking page shows
/// it: a word, a symbol and a colour, never the colour alone.
extension SubmissionOutcome {
    var title: String {
        switch self {
        case .accepted: AppLocalization.string("Node accepted")
        case .rejected: AppLocalization.string("Node rejected")
        case .uncertain: AppLocalization.string("Submission uncertain")
        }
    }

    var systemImage: String {
        switch self {
        case .accepted: "checkmark.circle"
        case .rejected: "xmark.circle"
        case .uncertain: "questionmark.circle"
        }
    }

    var color: Color {
        switch self {
        case .accepted: .green
        case .rejected: .red
        case .uncertain: .spectraWarning
        }
    }
}

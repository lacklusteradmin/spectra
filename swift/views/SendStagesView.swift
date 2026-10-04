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

            if showsImmediateNodeChoice { broadcastDestinations }

            SendTransferPartiesView(
                store: store, walletId: artifact.walletId, chain: artifact.chainId,
                sender: artifact.sender, recipient: artifact.recipient,
                saveRecipient: displayedTransaction.map { record in
                    { store.saveRecipientToAddressBook(record) }
                })
                .padding(SpectraLayout.cardPadding)
                .spectraCardFill()

            if artifact.attempts.isEmpty { reviewWarnings }

            if hasFinalHistoryStatus {
                transactionReceipt
                if !artifact.attempts.isEmpty {
                    DisclosureGroup(AppLocalization.string("View submission results")) {
                        submissionResults
                            .padding(.top, SpectraLayout.Space.s)
                    }
                    .font(.subheadline.weight(.semibold))
                }
            } else if !artifact.attempts.isEmpty {
                submissionResults
            }

            technicalDetails
            stageNotice

            if showsRetryNodeChoice {
                DisclosureGroup(AppLocalization.string("Retry submission")) {
                    broadcastDestinations
                        .padding(.top, SpectraLayout.Space.s)
                }
                .font(.subheadline.weight(.semibold))
            }

            if let transactionError {
                Text(transactionError)
                    .font(.caption)
                    .foregroundStyle(.secondary)
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

    private var stageProgress: some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
            progressLabel("Built", systemImage: "checkmark.circle", isCurrent: false)
            progressLabel(
                artifact.stage == .prepared ? "Awaiting signing" : "Signed",
                systemImage: artifact.stage == .prepared ? "circle" : "checkmark.circle",
                isCurrent: artifact.stage == .prepared)
            progressLabel(
                artifact.attempts.isEmpty ? "Awaiting submission" : "Submitted",
                systemImage: artifact.attempts.isEmpty ? "circle" : "antenna.radiowaves.left.and.right",
                isCurrent: artifact.stage == .signed && artifact.attempts.isEmpty)
        }
        .padding(.vertical, SpectraLayout.Space.s)
        .accessibilityIdentifier("send.review.stages")
    }

    private func progressLabel(_ title: String, systemImage: String, isCurrent: Bool) -> some View {
        VStack(spacing: SpectraLayout.Space.xs) {
            Image(systemName: systemImage)
                .font(.subheadline.weight(.semibold))
                .accessibilityHidden(true)
            Text(AppLocalization.string(title))
                .font(.caption.weight(.semibold))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity)
        .foregroundStyle(isCurrent ? Color.accentColor : Color.primary)
        .accessibilityElement(children: .combine)
    }

    private var amountSummary: some View {
        SendAmountSummaryView(
            artworkName: Coin.nativeChainBadge(for: artifact.chainId)?.artworkName,
            symbol: artifact.symbol, chain: artifact.chainId, amount: artifact.amount,
            statusTitle: statusTitle, statusSystemImage: statusSystemImage,
            statusColor: statusColor, isReceipt: displayedTransaction?.status == .confirmed)
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
                    Label(AppLocalization.string(outcomeText(attempt.outcome)), systemImage: outcomeSystemImage(attempt.outcome))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(outcomeColor(attempt.outcome))
                    DisclosureGroup(AppLocalization.string("Submission details")) {
                        Text(verbatim: attempt.detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        if let hash = attempt.transactionHash {
                            transactionHash(hash)
                        }
                    }
                    .font(.caption)
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
                    .buttonStyle(.glassProminent)
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
            Text(groupedAddress(hash))
                .font(.caption.monospaced())
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityLabel(Text(verbatim: hash))
                .contextMenu {
                    Button {
                        UIPasteboard.general.string = hash
                    } label: {
                        Label(AppLocalization.string("Copy"), systemImage: "doc.on.doc")
                    }
                }
        }
    }

    private var technicalDetails: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            DisclosureGroup(AppLocalization.string("Transaction details")) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    technicalValue("Prepared transaction", value: artifact.preparedDetails)
                    technicalValue("Review digest", value: artifact.reviewDigest)
                    if !artifact.signingPayloadHex.isEmpty {
                        technicalValue("Signing payload", value: artifact.signingPayloadHex)
                    }
                    if !artifact.attempts.isEmpty, store.pendingHighRiskSendReasons.count > 1 {
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                            Text(AppLocalization.string("Review warnings"))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            reviewWarnings
                        }
                    }
                }
                .padding(.top, SpectraLayout.Space.s)
            }
            if let payload = artifact.signedPayload {
                DisclosureGroup(AppLocalization.string("Signed payload")) {
                    Text(verbatim: payload)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                        .padding(.top, SpectraLayout.Space.s)
                }
            }
        }
        .font(.subheadline.weight(.semibold))
        .padding(.horizontal, SpectraLayout.Space.xs)
    }

    private func technicalValue(_ title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(AppLocalization.string(title)).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            Text(verbatim: value).font(.caption.monospaced()).textSelection(.enabled)
        }
    }

    private var reviewWarnings: some View {
        ForEach(Array(store.pendingHighRiskSendReasons.dropFirst().enumerated()), id: \.offset) { _, reason in
            Label(reason, systemImage: "exclamationmark.triangle")
                .font(.subheadline)
                .foregroundStyle(.spectraWarning)
        }
    }

    @ViewBuilder
    private var stageNotice: some View {
        if artifact.stage == .prepared {
            Label(AppLocalization.string("Review the complete addresses. Signing and broadcasting remain separate actions."), systemImage: "viewfinder")
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

    private func outcomeText(_ outcome: SubmissionOutcome) -> String {
        switch outcome {
        case .accepted: "Node accepted"
        case .rejected: "Node rejected"
        case .uncertain: "Submission uncertain"
        }
    }

    private func outcomeSystemImage(_ outcome: SubmissionOutcome) -> String {
        switch outcome {
        case .accepted: "checkmark.circle"
        case .rejected: "xmark.circle"
        case .uncertain: "questionmark.circle"
        }
    }

    private func outcomeColor(_ outcome: SubmissionOutcome) -> Color {
        switch outcome {
        case .accepted: .green
        case .rejected: .red
        case .uncertain: .spectraWarning
        }
    }
}

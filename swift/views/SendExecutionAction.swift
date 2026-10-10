import Foundation

/// Native action labels over core's build, submission and chain-status projections.
/// An acceptance receipt is never evidence that a transaction has confirmed.
enum SendExecutionAction: Equatable {
    case build, sign, broadcast, retry, done

    init(artifact: SendArtifact?, transaction: TransactionRecord?) {
        guard let artifact else { self = .build; return }
        let transaction = transaction?.id == artifact.id ? transaction : nil
        if let transaction, transaction.status != .pending { self = .done; return }
        if artifact.stage == .prepared { self = .sign; return }
        // Once a node has accepted it the send is out of the flow's hands:
        // the receipt follows it, and its explorer link is there to follow.
        if artifact.attempts.contains(where: { $0.outcome == .accepted }) {
            self = .done
        } else {
            self = artifact.attempts.isEmpty ? .broadcast : .retry
        }
    }

    static func canRetry(artifact: SendArtifact, transaction: TransactionRecord?) -> Bool {
        guard artifact.stage == .signed, !artifact.attempts.isEmpty else { return false }
        guard let transaction, transaction.id == artifact.id else { return false }
        return transaction.actions.rebroadcastUnavailableReason == nil
    }

    var title: String {
        switch self {
        case .build: "Build Transaction"
        case .sign: "Sign Transaction"
        case .broadcast: "Broadcast Transaction"
        case .retry: "Retry Same Transaction"
        case .done: "Done"
        }
    }

    var systemImage: String {
        switch self {
        case .build: "hammer.fill"
        case .sign: "signature"
        case .broadcast: "antenna.radiowaves.left.and.right"
        case .retry: "arrow.clockwise"
        case .done: "checkmark"
        }
    }
}

@MainActor
func sendSigningConfirmationMessage(artifact: SendArtifact, amounts: AmountPresentation) -> String {
    let amount = AmountPresentation.localizedDecimal(artifact.amount)
    var lines = ["\(amount) \(artifact.symbol) · \(artifact.chainId.displayName)"]
    if let fee = artifact.review.networkFee {
        lines.append(AppLocalization.format(
            "Network Fee: %@",
            "≈ " + amounts.compactNetworkFee(fee, value: artifact.review.networkFeeValue, chain: artifact.chainId)))
    }
    lines += [
        AppLocalization.string("Recipient"),
        artifact.recipient,
        "",
        AppLocalization.string("Signing authorizes this transaction. You will choose nodes and broadcast it separately.")
    ]
    var warnings = highRiskSendMessages(artifact.review.warnings)
        + evmRecipientMessages(artifact.review.recipientWarnings)
    if artifact.review.requiresSelfSendConfirmation {
        warnings.append(AppLocalization.string("This destination belongs to your wallet. Confirm intentional self-send."))
    }
    if !warnings.isEmpty { lines.append("\n• " + warnings.joined(separator: "\n• ")) }
    return lines.joined(separator: "\n")
}

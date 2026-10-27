import Foundation

/// The labels of core's next step for a staged transaction. Core decides the
/// step; an acceptance receipt is never evidence that a transaction confirmed.
extension SendNextStep {
    var title: String {
        switch self {
        case .sign: "Sign Transaction"
        case .broadcast: "Broadcast Transaction"
        case .rebroadcast: "Retry Same Transaction"
        case .done: "Done"
        }
    }

    var systemImage: String {
        switch self {
        case .sign: "signature"
        case .broadcast: "antenna.radiowaves.left.and.right"
        case .rebroadcast: "arrow.clockwise"
        case .done: "checkmark"
        }
    }
}

@MainActor
func sendSigningConfirmationMessage(artifact: SendArtifact, amounts: AmountPresentation) -> String {
    let amount = AmountPresentation.localizedDecimal(artifact.amount)
    var lines = ["\(amount) \(artifact.symbol) · \(artifact.chainId.displayName)"]
    if let fee = artifact.review.networkFee {
        lines.append(AppLocalization.format("Network Fee: %@", amounts.reviewedNetworkFee(fee, chain: artifact.chainId)))
    }
    lines += [
        AppLocalization.string("Recipient"),
        artifact.recipient,
        "",
        AppLocalization.string("Signing authorizes this transaction. You will choose nodes and broadcast it separately.")
    ]
    let warnings = artifact.reviewWarningMessages
    if !warnings.isEmpty { lines.append("\n• " + warnings.joined(separator: "\n• ")) }
    return lines.joined(separator: "\n")
}

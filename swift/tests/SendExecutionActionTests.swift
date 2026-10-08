import Foundation
import Testing
@testable import Spectra

@MainActor
struct SendExecutionActionTests {
    private func artifact(stage: SendStage = .signed, outcome: SubmissionOutcome? = nil) -> SendArtifact {
        SendArtifact(id: "send-action", revision: 1, stage: stage, walletId: "wallet", chainId: .ethereum,
            sender: "0x1111111111111111111111111111111111111111",
            recipient: "0x2222222222222222222222222222222222222222", memo: nil,
            amount: "1.000000000000000001", asset: "token-contract", symbol: "USDC", staking: nil, operation: nil, createdAt: 0,
            reviewDigest: "digest", review: SendArtifactReview(warnings: [], recipientWarnings: [], requiresSelfSendConfirmation: false, staking: nil, transferTerms: nil),
            preparedDetails: "", signingPayloadHex: "02", signedPayload: stage == .signed ? "02" : nil,
            transactionHash: nil, attempts: outcome.map {
                [BroadcastAttempt(endpoint: "https://node.example", attemptedAt: 0, outcome: $0, transactionHash: nil, detail: "")]
            } ?? [], selectedEndpoints: [])
    }

    private func transaction(id: String = "send-action", status: TransactionStatus,
                             failureReason: TransactionFailure? = nil) -> TransactionRecord {
        var record = TransactionRecord(id: id, kind: .send, status: status, walletName: "Wallet", assetDisplayName: "USD Coin",
            symbol: "USDC", chainId: .ethereum, amount: "1.000000000000000001", address: "recipient",
            transactionHash: "0x1234", failureReason: failureReason)
        record.actions = TransactionActions(recheckUnavailableReason: nil,
            rebroadcastUnavailableReason: status == .confirmed ? "Transaction is confirmed." : nil)
        return record
    }

    @Test func buildSignAndBroadcastRemainSeparateActions() {
        #expect(SendExecutionAction(artifact: nil, transaction: nil) == .build)
        #expect(SendExecutionAction(artifact: artifact(stage: .prepared), transaction: nil) == .sign)
        #expect(SendExecutionAction(artifact: artifact(), transaction: nil) == .broadcast)
    }

    @Test func confirmedReceiptsFinishInsteadOfRebroadcasting() {
        let receipt = transaction(status: .confirmed)
        #expect(SendExecutionAction(artifact: artifact(outcome: .accepted), transaction: receipt) == .done)
        #expect(!SendExecutionAction.canRetry(artifact: artifact(outcome: .accepted), transaction: receipt))
    }

    @Test func retryAvailabilityComesFromCore() {
        var unresolved = transaction(status: .pending)
        #expect(SendExecutionAction(artifact: artifact(outcome: .accepted), transaction: unresolved) == .viewTransaction)
        #expect(SendExecutionAction.canRetry(artifact: artifact(outcome: .accepted), transaction: unresolved))
        unresolved.actions.rebroadcastUnavailableReason = "Unavailable"
        #expect(!SendExecutionAction.canRetry(artifact: artifact(outcome: .accepted), transaction: unresolved))
    }

    @Test func acceptedSubmissionCanBeInspectedWhileUncertainSubmissionCanBeRetried() {
        let pending = transaction(status: .pending)
        #expect(SendExecutionAction(artifact: artifact(outcome: .accepted), transaction: pending) == .viewTransaction)
        #expect(SendExecutionAction.canRetry(artifact: artifact(outcome: .accepted), transaction: pending))
        #expect(SendExecutionAction(artifact: artifact(outcome: .uncertain), transaction: pending) == .retry)
        #expect(SendExecutionAction(artifact: artifact(outcome: .rejected), transaction: nil) == .retry)
    }

    @Test func aReceiptFromAnotherSendCannotFinishThisSend() {
        let other = transaction(id: "other-send", status: .confirmed)
        #expect(SendExecutionAction(artifact: artifact(), transaction: other) == .broadcast)
        #expect(!SendExecutionAction.canRetry(artifact: artifact(outcome: .accepted), transaction: other))
    }

    @Test func retriesWaitForTheMatchingCoreCapabilities() {
        let submitted = artifact(outcome: .uncertain)
        #expect(!SendExecutionAction.canRetry(artifact: submitted, transaction: nil))
        #expect(!SendExecutionAction.canRetry(
            artifact: submitted, transaction: transaction(id: "other-send", status: .pending)))
        #expect(SendExecutionAction.canRetry(artifact: submitted, transaction: transaction(status: .pending)))
    }

    @Test func signingSummaryUsesExactImmutableAmountAndCompleteRecipient() {
        var built = artifact(stage: .prepared)
        let message = sendSigningConfirmationMessage(artifact: built)
        #expect(message.contains(AmountPresentation.localizedDecimal(built.amount)))
        #expect(message.contains(built.symbol))
        #expect(message.contains(built.recipient))
        #expect(message.contains(built.chainId.displayName))
        #expect(!message.contains(built.asset))
        built.review.requiresSelfSendConfirmation = true
        #expect(sendSigningConfirmationMessage(artifact: built).contains(
            AppLocalization.string("This destination belongs to your wallet. Confirm intentional self-send.")))
    }
}

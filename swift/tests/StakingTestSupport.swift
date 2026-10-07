import Foundation

@testable import Spectra

@MainActor
enum StakingTestSupport {
    static func artifact(
        _ id: String = "staking-review", stage: SendStage = .prepared,
        submitted: Bool = false, chain: Chain = .solana
    ) -> SendArtifact {
        SendArtifact(
            id: id, revision: 1, stage: stage, walletId: "wallet", chainId: chain,
            sender: "owner", recipient: "validator", amount: "1.234567891", asset: chain.gasTokenSymbol,
            symbol: chain.gasTokenSymbol,
            staking: StakingRequest(
                walletId: "wallet", chainId: chain, action: .stake,
                validatorId: "validator", positionId: nil, amount: "1.234567891", lockupSeconds: nil),
            operation: nil, createdAt: 0, reviewDigest: "immutable-review",
            review: SendArtifactReview(
                warnings: [], recipientWarnings: [], requiresSelfSendConfirmation: false,
                staking: StakingReview(
                    networkFee: "0.000005", feeIsUpperBound: false,
                    feeIsDeductedFromAmount: false,
                    fundingAlreadyCompleted: false,
                    refundableDeposit: "0.00228288", lockupSeconds: nil, rewardPayoutIsDelayed: false)),
            preparedDetails: "reviewed content", signingPayloadHex: "0102",
            signedPayload: stage == .signed ? "signed-content" : nil,
            transactionHash: submitted ? "exact-hash" : nil,
            attempts: submitted
                ? [
                    BroadcastAttempt(
                        endpoint: "node-b", attemptedAt: 0, outcome: .accepted,
                        transactionHash: "exact-hash", detail: "Accepted")
                ] : [], selectedEndpoints: submitted ? ["node-b"] : [])
    }

    static func position(_ id: String = "position") -> StakingPosition {
        StakingPosition(
            id: id, owner: "owner", validatorIdentifier: "validator", status: .active,
            stakedAmountSmallestUnit: "1234567891", unbondingAmountSmallestUnit: "0",
            withdrawableAmountSmallestUnit: "0", claimableRewardsSmallestUnit: nil,
            pendingRewardsSmallestUnit: nil, rewardsUnlockTimeUnix: nil,
            unlockEpoch: nil, unlockTimeUnix: nil, availableActions: [.unstake])
    }

    static func transaction(_ id: String = "staking-review", status: TransactionStatus = .pending) -> TransactionRecord
    {
        TransactionRecord(
            id: id, walletId: "wallet", kind: .stake, status: status,
            walletName: "Wallet", assetDisplayName: "Solana", symbol: "SOL", chainId: .solana,
            amount: "1.234567891", address: "validator", transactionHash: "exact-hash")
    }

    static func operations() -> StakingOperations {
        StakingOperations(
            validators: { _ in [] }, positions: { _, _, _, _ in [] }, saved: { [] },
            build: { _, _ in artifact() }, endpoints: { _ in ["node-a", "node-b"] },
            sign: { _, _, _ in artifact(stage: .signed) },
            broadcast: { _, _ in artifact(stage: .signed, submitted: true) },
            inspect: { id in artifact(id) }, transaction: { id in transaction(id) },
            recheck: { id, _ in artifact(id, stage: .signed, submitted: true) },
            repair: { id, _ in
                var reviewed = artifact(id, chain: .icp)
                reviewed.review.staking?.fundingAlreadyCompleted = true
                return reviewed
            })
    }
}

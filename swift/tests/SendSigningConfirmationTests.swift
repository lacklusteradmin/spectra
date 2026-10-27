import Foundation
import Testing
@testable import Spectra

/// What the signing prompt says. Which step comes next is core's, tested in
/// core; this is the prompt's wording of the artifact core built.
@MainActor
struct SendSigningConfirmationTests {
    private func artifact() -> SendArtifact {
        SendArtifact(id: "send-action", revision: 1, stage: .prepared, walletId: "wallet", chainId: .ethereum,
            sender: "0x1111111111111111111111111111111111111111",
            recipient: "0x2222222222222222222222222222222222222222", memo: nil,
            amount: "1.000000000000000001", asset: "token-contract", symbol: "USDC", staking: nil, operation: nil, createdAt: 0,
            reviewDigest: "digest", review: SendArtifactReview(warnings: [], staking: nil, transferTerms: nil),
            preparedDetails: "", signingPayloadHex: "02", signedPayload: nil,
            transactionHash: nil, attempts: [], selectedEndpoints: [])
    }

    private let amounts = AmountPresentation(assetPrecision: nil, valuation: nil, selectedFiatCurrency: .usd)

    @Test func signingSummaryUsesExactImmutableAmountAndCompleteRecipient() {
        var built = artifact()
        let message = sendSigningConfirmationMessage(artifact: built, amounts: amounts)
        #expect(message.contains(AmountPresentation.localizedDecimal(built.amount)))
        #expect(message.contains(built.symbol))
        #expect(message.contains(built.recipient))
        #expect(message.contains(built.chainId.displayName))
        #expect(!message.contains(built.asset))
        #expect(!message.contains(AppLocalization.format("Network Fee: %@", "")))
        built.review.warnings = [.ownAddress]
        #expect(sendSigningConfirmationMessage(artifact: built, amounts: amounts).contains(
            AppLocalization.string("This destination belongs to your wallet. Confirm intentional self-send.")))
    }

    /// The prompt says what the review says, in core's order, transfer terms
    /// included.
    @Test func signingSummaryCarriesEveryReviewWarning() {
        var built = artifact()
        built.review.warnings = [
            .risk(warning: .newAddress), .tokenFee(fee: "0.01", received: "0.99"), .transferHook(program: "Hook111"),
        ]
        let message = sendSigningConfirmationMessage(artifact: built, amounts: amounts)
        for warning in built.reviewWarningMessages { #expect(message.contains(warning)) }
        #expect(built.reviewWarningMessages.count == 3)
        #expect(message.contains("Hook111"))
    }

    @Test func signingSummaryStatesTheReviewedNetworkFeeAndItsBound() {
        var built = artifact()
        built.review.networkFee = NetworkFee(amount: "0.00042", bound: .estimate, value: nil)
        let message = sendSigningConfirmationMessage(artifact: built, amounts: amounts)
        #expect(message.contains(AppLocalization.format("Network Fee: %@", "")))
        #expect(message.contains("≈ " + AmountPresentation.localizedDecimal("0.00042")))
        #expect(message.contains(Chain.ethereum.gasTokenSymbol))
        built.review.networkFee = NetworkFee(amount: "0.00042", bound: .atMost, value: nil)
        #expect(sendSigningConfirmationMessage(artifact: built, amounts: amounts)
            .contains("≤ " + AmountPresentation.localizedDecimal("0.00042")))
        built.review.networkFee = NetworkFee(amount: "0.00042", bound: .exact, value: nil)
        let exact = sendSigningConfirmationMessage(artifact: built, amounts: amounts)
        #expect(!exact.contains("≈") && !exact.contains("≤"))
    }
}

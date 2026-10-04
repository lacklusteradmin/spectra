import Foundation
import Testing

@testable import Spectra

/// What the send Live Activity says, for each status core can report.
///
/// The activity's lifecycle needs a device to observe, but the content is a
/// pure function of the record and the phase, which is the part that decides
/// whether a glance at the lock screen tells the truth.
struct SendLiveActivityContentTests {
    private func record(
        status: TransactionStatus = .pending,
        transactionHash: String? = nil,
        failureReason: TransactionFailure? = nil
    ) -> TransactionRecord {
        TransactionRecord(
            id: UUID().uuidString,
            kind: .send, status: status, walletName: "Main", assetDisplayName: "Ether",
            symbol: "ETH", chainId: Chain.ethereum, amount: "1.5",
            address: "0x1234567890abcdef1234567890abcdef12345678",
            transactionHash: transactionHash, failureReason: failureReason)
    }

    @Test func eachPhaseNamesItselfAndTheChain() {
        let sending = sendLiveActivityContentState(
            for: record(), phase: .sending, amountText: "1.5")
        #expect(sending.statusText == "Sending")
        #expect(sending.detailText.contains("Ethereum"), "the wait names the chain being waited on")

        let complete = sendLiveActivityContentState(
            for: record(status: .confirmed), phase: .complete, amountText: "1.5")
        #expect(complete.statusText == "Sent")
        #expect(complete.detailText.contains("Ethereum"))

        let failed = sendLiveActivityContentState(
            for: record(status: .failed), phase: .failed, amountText: "1.5")
        #expect(failed.statusText == "Send failed")
    }

    /// A failure the chain explained is more use than the generic sentence.
    @Test func aFailureReasonBeatsTheGenericLine() {
        let explained = record(status: .failed, failureReason: .executionFailed)
        let generic = record(status: .failed)
        let withReason = sendLiveActivityContentState(
            for: explained, phase: .failed, amountText: "1.5")
        let withoutReason = sendLiveActivityContentState(
            for: generic, phase: .failed, amountText: "1.5")
        #expect(withReason.detailText == explained.localizedFailureReason)
        #expect(withReason.detailText != withoutReason.detailText)
    }

    /// The symbol has its own label in the widget, so the amount arrives alone.
    @Test func theAmountCarriesNoSymbolAndTheSymbolIsItsOwnField() {
        let state = sendLiveActivityContentState(for: record(), phase: .sending, amountText: "1.5")
        #expect(state.amountText == "1.5")
        #expect(state.symbol == "ETH")
    }

    @Test func longIdentifiersKeepBothEndsAndShortOnesAreLeftAlone() {
        #expect(sendLiveActivityPreview("0xabc", keepingEachEnd: 6) == "0xabc")
        #expect(sendLiveActivityPreview("0x1234567890abcdef1234567890abcdef12345678", keepingEachEnd: 6) == "0x1234…345678")
    }

    @Test func theHashAppearsOnlyOnceThereIsOne() {
        #expect(sendLiveActivityContentState(for: record(), phase: .sending, amountText: "1.5")
                .transactionHashPreview == nil)
        let broadcast = record(transactionHash: "0xfeedfacefeedfacefeedfacefeedfacefeedface")
        #expect(sendLiveActivityContentState(for: broadcast, phase: .complete, amountText: "1.5")
                .transactionHashPreview == "0xfeedfa…feedface")
    }

    /// The lock screen runs a timer off `startedAt`, so it has to be the send's
    /// own clock rather than the moment the phase last changed.
    @Test func theTimerRunsFromTheTransactionsOwnStart() {
        let transaction = record()
        for phase in [
            SendTransactionLiveActivityAttributes.ContentState.Phase.sending, .complete, .failed,
        ] {
            #expect(sendLiveActivityContentState(for: transaction, phase: phase, amountText: "1.5")
                    .startedAt == transaction.createdDate)
        }
    }
}

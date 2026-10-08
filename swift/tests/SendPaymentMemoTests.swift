import Testing
@testable import Spectra

@MainActor
@Suite(.isolatedAppState)
struct SendPaymentMemoTests: IsolatedAppStateSuite {
    /// The composer offers the kinds core names for each network, sends a
    /// memo only once something is typed, and forgets it on another holding.
    @Test func memoFollowsCoreKindsAndTheSelectedHolding() {
        #expect(paymentMemoKinds(chain: .xrp) == [.destinationTag])
        #expect(paymentMemoKinds(chain: .stellar) == [.memoText, .memoId])
        #expect(paymentMemoKinds(chain: .ethereum).isEmpty)

        let flow = makeState().sendFlow
        flow.holdingKey = "xrp:native"
        flow.memoKind = .destinationTag
        #expect(flow.paymentMemo == nil)
        flow.memoText = "0"
        #expect(flow.paymentMemo == PaymentMemo(kind: .destinationTag, value: "0"))

        flow.holdingKey = "stellar:native"
        #expect(flow.memoKind == nil && flow.memoText.isEmpty && flow.paymentMemo == nil)

        flow.memoKind = .memoId
        flow.memoText = "1029384756"
        flow.close()
        #expect(flow.paymentMemo == nil, "Closing the composer clears the memo with the rest of the form")
    }
}

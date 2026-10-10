import Foundation
import Testing
@testable import Spectra

@MainActor
@Suite(.isolatedAppState)
struct SendPreviewAdoptionTests: IsolatedAppStateSuite {

    @Test func quoteCannotBeReusedForAnotherWalletHoldingOrNetwork() {
        let store = SendPreviewStore()
        let preview = SendPreview.solana(preview: SolanaSendPreview(
            estimatedNetworkFee: "0.000005", spendableBalance: "1", feeRateDescription: nil,
            maxSendable: "0.999995"))
        let quote = OwnedSendPreview(walletId: "w", holdingKey: "solana:native", chainId: Chain.solana, amount: "1",
            preview: preview, networkFee: "0.000005", networkFeeValue: nil, amountValue: nil,
            details: nil, shortcuts: [100: "0.999994999"], recipient: nil)
        store.apply(quote)
        let sol = AssetHolding.fixture(name: "Solana", symbol: "SOL", chainId: Chain.solana, amount: "1")
        #expect(store.quote(walletId: "w", coin: sol)?.shortcuts[100] == "0.999994999")
        #expect(store.quote(walletId: "other", coin: sol) == nil)
        let token = AssetHolding.fixture(name: "Other", symbol: "OTH", chainId: Chain.solana, tokenStandard: "SPL",
            contractAddress: "other", amount: "1")
        #expect(store.quote(walletId: "w", coin: token) == nil)
        // The same asset on another network is another holding.
        let devnet = AssetHolding.fixture(name: "Solana", symbol: "SOL", chainId: Chain.solanaDevnet, amount: "1")
        #expect(store.quote(walletId: "w", coin: devnet) == nil)
        store.reset()
        #expect(store.quote(walletId: "w", coin: sol) == nil)
    }

    /// An override turned on starts from what the network asks, so it is an
    /// edit of the quote rather than an empty field that is an error at once.
    @Test func overridesStartFromTheQuoteAndKeepWhatWasTyped() {
        let flow = makeState().sendFlow
        flow.useCustomEvmFees = true
        #expect(flow.customEvmMaxFeeGwei.isEmpty, "no quote, nothing to start from")
        flow.useCustomEvmFees = false
        flow.previewStore.apply(OwnedSendPreview(
            walletId: "w", holdingKey: "ethereum:native", chainId: .ethereum, amount: "1",
            preview: .ethereum(preview: EvmSendPreview(
                nonce: 7, gasLimit: 21_000, maxFeePerGasGwei: "20", maxPriorityFeePerGasGwei: "1.5",
                estimatedNetworkFee: "0.00042", spendableBalance: nil, feeRateDescription: nil, maxSendable: nil)),
            networkFee: "0.00042", networkFeeValue: nil, amountValue: nil, details: nil, shortcuts: [:],
            recipient: nil))
        flow.useCustomEvmFees = true
        flow.evmManualNonceEnabled = true
        #expect(flow.customEvmMaxFeeGwei == AmountPresentation.decimalFieldText("20"))
        #expect(flow.customEvmPriorityFeeGwei == AmountPresentation.decimalFieldText("1.5"))
        #expect(flow.evmManualNonce == "7")
        #expect(flow.customEvmFeeValidationError == nil)
        #expect(flow.evmNonceValidationError == nil)
        flow.evmManualNonce = "9"
        flow.evmManualNonceEnabled = false
        flow.evmManualNonceEnabled = true
        #expect(flow.evmManualNonce == "9", "a typed value is not replaced")
    }

    /// An override that does not parse asks core for nothing: the field says
    /// what is wrong, and the last quote stays as the network's terms.
    @Test func anUnparsedOverrideAsksForNoQuote() async {
        let store = makeState()
        store.sendFlow.evmManualNonceEnabled = true
        store.sendFlow.evmManualNonce = "not a number"
        await store.refreshSendPreview()
        #expect(store.sendFlow.previewError == nil)
        #expect(!store.sendFlow.isPreparingPreview)
    }

    @Test func previewDiscardsStaleSuccessAndFailureForEveryFormEdit() {
        let store = makeState()
        let edits: [(AppState) -> Void] = [
            { $0.sendFlow.walletId = "other" }, { $0.sendFlow.holdingKey = "other" },
            { $0.sendFlow.amount = "2" }, { $0.sendFlow.address = "other" },
            { $0.sendFlow.evmManualNonceEnabled.toggle() }, { $0.sendFlow.evmManualNonce = "invalid" },
            { $0.sendFlow.useCustomEvmFees.toggle() }, { $0.sendFlow.customEvmMaxFeeGwei = "invalid" },
            { $0.sendFlow.customEvmPriorityFeeGwei = "invalid" },
            { $0.sendFlow.previewRequestId = UUID() },
        ]
        let target = { store.sendPreviewTarget }
        for edit in edits {
            let input = store.sendFlow.previewInput(for: target())
            let request = store.sendFlow.previewRequestId
            edit(store)
            store.sendFlow.previewError = "current quote failure"
            store.sendFlow.adoptPreviewResult(.failure(NSError(domain: "old", code: 1)),
                requestId: request, input: input, target: target)
            #expect(store.sendFlow.previewError == "current quote failure")
            store.sendFlow.adoptPreviewResult(.success(nil), requestId: request, input: input, target: target)
            #expect(store.sendFlow.previewError == "current quote failure")
        }
        // A quote's failure is the quote's, not the build's or the broadcast's.
        store.sendFlow.session.error = "current build failure"
        store.sendFlow.adoptPreviewResult(.failure(NSError(domain: "current", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "current failure"])),
            requestId: store.sendFlow.previewRequestId, input: store.sendFlow.previewInput(for: target()),
            target: target)
        #expect(store.sendFlow.previewError == "current failure")
        #expect(store.sendFlow.session.error == "current build failure")
        let oldRequest = store.sendFlow.previewRequestId
        store.cancelSend()
        #expect(store.sendFlow.previewRequestId != oldRequest)
    }

}

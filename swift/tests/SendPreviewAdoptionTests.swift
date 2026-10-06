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
            store.sendFlow.session.error = "current form message"
            store.sendFlow.adoptPreviewResult(.failure(NSError(domain: "old", code: 1)),
                requestId: request, input: input, target: target)
            #expect(store.sendFlow.session.error == "current form message")
            store.sendFlow.adoptPreviewResult(.success(nil), requestId: request, input: input, target: target)
            #expect(store.sendFlow.session.error == "current form message")
        }
        store.sendFlow.adoptPreviewResult(.failure(NSError(domain: "current", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "current failure"])),
            requestId: store.sendFlow.previewRequestId, input: store.sendFlow.previewInput(for: target()),
            target: target)
        #expect(store.sendFlow.session.error == "current failure")
        let oldRequest = store.sendFlow.previewRequestId
        store.cancelSend()
        #expect(store.sendFlow.previewRequestId != oldRequest)
    }

}

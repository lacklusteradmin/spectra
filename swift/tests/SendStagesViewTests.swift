import Foundation
import Testing
import SwiftUI
import UIKit
@testable import Spectra

@MainActor
@Suite(.isolatedAppState, .serialized)
struct SendStagesViewTests: IsolatedAppStateSuite {

    enum RenderStage: String, CaseIterable {
        case prepared, signed, pending, confirmed
    }

    @Test(arguments: RenderStage.allCases)
    func durableStagesRenderWithoutChoosingOrSubmittingNodes(stage: RenderStage) async throws {
        let state = makeState()
        state.sendFlow.session.endpoints = ["https://ethereum.example/rpc"]
        let hasSubmission = stage == .pending || stage == .confirmed
        let hash = "0x3f5d2c8af442824595b091e1761b8db5bf8d9ef8fa93b512d283850d08a7ef11"
        let artifact = SendArtifact(id: "render-fixture", revision: 1,
            stage: stage == .prepared ? .prepared : .signed,
            walletId: "fixture", chainId: Chain.ethereumSepolia,
            sender: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            recipient: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8", memo: nil,
            amount: "0.25", asset: "ETH", symbol: "ETH", staking: nil, operation: nil, createdAt: 0, reviewDigest: "reviewed-content",
            review: SendArtifactReview(
                warnings: [.risk(warning: .newAddress), .ownAddress], staking: nil, transferTerms: nil,
                networkFee: NetworkFee(amount: "0.000021", bound: .estimate, value: nil)),
            preparedDetails: "Nonce: 7\nMaximum gas: 25200", signingPayloadHex: "02",
            signedPayload: stage == .prepared ? nil : "0x02…", transactionHash: hasSubmission ? hash : nil,
            attempts: hasSubmission ? [BroadcastAttempt(endpoint: "https://ethereum.example/rpc", attemptedAt: 0,
                outcome: .accepted, transactionHash: hash, detail: "Accepted by the node.")] : [], selectedEndpoints: [])
        #expect(artifact.reviewWarningMessages.count == 2)
        var record: TransactionRecord? = hasSubmission ? TransactionRecord(
            id: artifact.id, walletId: artifact.walletId, kind: .send,
            status: stage == .confirmed ? .confirmed : .pending,
            walletName: "Main Wallet", assetDisplayName: "Ethereum", symbol: artifact.symbol,
            chainId: artifact.chainId, amount: artifact.amount,
            address: artifact.recipient, transactionHash: hash) : nil
        record?.actions = TransactionActions(recheckUnavailableReason: nil,
            rebroadcastUnavailableReason: stage == .confirmed ? "Transaction is confirmed." : nil)
        let progress = SendProgress.staged(
            artifact, transaction: record, next: stage == .confirmed ? .done : nil,
            canRebroadcast: stage == .pending)
        state.sendFlow.session.progress = progress
        let view = ScrollView {
            SendStagesView(store: state, progress: progress).padding()
        }.frame(width: 393, height: 852).background(Color(.systemBackground))
        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 393, height: 852)
        window.rootViewController = UIHostingController(rootView: view)
        // This independent render window never takes another parallel suite's key window.
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }
        // Lay out the hosting hierarchy before allowing its native compositor
        // to settle. Taking the first layout and snapshot together can omit glyphs.
        window.rootViewController?.view.layoutIfNeeded()
        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(300))
        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(100))
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            #expect(window.drawHierarchy(in: window.bounds, afterScreenUpdates: true))
        }
        let pixels = try #require(image.cgImage?.dataProvider?.data) as Data
        #expect(Set(pixels).count > 16, "The render must contain content, not an empty canvas")
        Attachment.record(image, named: "Send stage: \(stage.rawValue)")
        #expect(state.sendFlow.session.selectedEndpoints.isEmpty, "Rendering must not select destinations or submit")
        #expect(state.sendFlow.session.artifact == artifact, "Rendering must leave the durable send unchanged")
    }

    /// What a token's own rules do is said before signing, in core's exact
    /// amounts and with the symbol of the coin each is paid in.
    @Test func transferTermWarningsAreWordedWithCoreAmounts() {
        let hook = "8vB8ZeymdzgkRnm9YPGMqAhNfCsc1vN5r5ga58GtYvb1"
        let fee = artifact(on: .solana, symbol: "FEE", warnings: [
            .tokenFee(fee: "0.005", received: "0.995"), .transferHook(program: hook),
        ]).reviewWarningMessages
        #expect(fee.count == 2)
        #expect(fee[0].contains(AmountPresentation.localizedDecimal("0.005"))
            && fee[0].contains(AmountPresentation.localizedDecimal("0.995")) && fee[0].contains("FEE"))
        #expect(fee[1].contains(hook))
        let carried = artifact(on: .cardano, symbol: "TEST", warnings: [.carriesNative(amount: "1.05508")])
            .reviewWarningMessages
        #expect(carried.count == 1)
        #expect(carried[0].contains(AmountPresentation.localizedDecimal("1.05508")) && carried[0].contains("ADA"))
        let registration = artifact(on: .near, symbol: "TEST", warnings: [.registersRecipient(deposit: "0.00125")])
            .reviewWarningMessages
        #expect(registration.count == 1)
        #expect(registration[0].contains(AmountPresentation.localizedDecimal("0.00125"))
            && registration[0].contains("NEAR"))
    }

    private func artifact(on chain: Chain, symbol: String, warnings: [ReviewWarning]) -> SendArtifact {
        SendArtifact(id: "terms-fixture", revision: 0, stage: .prepared,
            walletId: "fixture", chainId: chain, sender: "sender", recipient: "recipient", memo: nil,
            amount: "1", asset: "token", symbol: symbol,
            staking: nil, operation: nil, createdAt: 0, reviewDigest: "reviewed-content",
            review: SendArtifactReview(warnings: warnings, staking: nil, transferTerms: nil),
            preparedDetails: "", signingPayloadHex: "", signedPayload: nil, transactionHash: nil,
            attempts: [], selectedEndpoints: [])
    }
}

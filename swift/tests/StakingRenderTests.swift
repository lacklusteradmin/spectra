import Foundation
import SwiftUI
import Testing
import UIKit

@testable import Spectra

@MainActor
@Suite(.isolatedAppState, .serialized)
struct StakingRenderTests: IsolatedAppStateSuite {
    enum RenderStage: String, CaseIterable {
        case positions, queuedRewards, prepared, withdrawalFees, signed, pending, confirmed, recovery, repaired
    }

    @Test(arguments: RenderStage.allCases)
    func reviewAndPositionsRenderWithoutSigningOrSubmitting(stage: RenderStage) async throws {
        let store = makeState()
        var operations = StakingTestSupport.operations()
        var signed = false
        var broadcast = false
        operations.sign = { _, _, _ in
            signed = true
            return StakingTestSupport.artifact(stage: .signed)
        }
        operations.broadcast = { _, _ in
            broadcast = true
            return StakingTestSupport.artifact(stage: .signed, submitted: true)
        }
        let chain: Chain = [.queuedRewards, .recovery, .withdrawalFees, .repaired].contains(stage) ? .icp : .solana
        let vm = StakingViewModel(chain: chain, bridge: bridge, operations: operations)
        vm.selectWallet("wallet")
        var artifact = StakingTestSupport.artifact(
            stage: [.prepared, .withdrawalFees, .repaired].contains(stage) ? .prepared : .signed,
            submitted: stage == .pending || stage == .confirmed || stage == .recovery, chain: chain)
        if stage == .withdrawalFees {
            artifact.staking?.action = .withdraw
            artifact.staking?.positionId = "42"
            artifact.review.staking?.networkFee = "0.0001"
            artifact.review.staking?.refundableDeposit = nil
            artifact.review.staking?.feeIsDeductedFromAmount = true
        }
        if stage == .repaired {
            artifact.revision = 2
            artifact.reviewDigest = "reviewed-recovery"
            artifact.staking?.lockupSeconds = 600
            artifact.review.staking?.lockupSeconds = nil
            artifact.review.staking?.networkFee = "0"
            artifact.review.staking?.refundableDeposit = nil
            artifact.review.staking?.fundingAlreadyCompleted = true
        }
        vm.session.artifact = artifact
        vm.session.endpoints = ["node-a", "node-b"]
        vm.positions = [StakingTestSupport.position()]
        if stage == .queuedRewards {
            vm.positions[0].pendingRewardsSmallestUnit = "123456789"
            vm.positions[0].rewardsUnlockTimeUnix = 1_800_604_800
        }
        vm.hasLoadedPositions = true
        if stage == .pending || stage == .confirmed {
            vm.transaction = StakingTestSupport.transaction(status: stage == .confirmed ? .confirmed : .pending)
        }
        if stage == .recovery { vm.transaction = StakingTestSupport.transaction(status: .failed) }
        let view = ScrollView {
            if stage == .positions || stage == .queuedRewards {
                StakingPositionsView(vm: vm, canSign: true, requiresPassword: false)
            } else {
                StakingTransactionView(
                    store: store, vm: vm, artifact: artifact,
                    confirmsSigning: .constant(false), confirmsBroadcast: .constant(false))
            }
        }.frame(width: 393, height: 852).background(Color(.systemBackground))
        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 393, height: 852)
        window.rootViewController = UIHostingController(rootView: view)
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }
        window.rootViewController?.view.layoutIfNeeded()
        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(300))
        window.layoutIfNeeded()
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            #expect(window.drawHierarchy(in: window.bounds, afterScreenUpdates: true))
        }
        let pixels = try #require(image.cgImage?.dataProvider?.data) as Data
        #expect(Set(pixels).count > 16)
        Attachment.record(image, named: "Staking: \(stage.rawValue)")
        #expect(!signed && !broadcast, "Rendering must never use a key or submit a transaction")
        #expect(vm.session.artifact == artifact)
        #expect(vm.session.selectedEndpoints.isEmpty)
        #expect(vm.password.isEmpty)
    }
}

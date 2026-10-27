import Foundation
import Testing

@testable import Spectra

@MainActor
@Suite(.isolatedAppState, .timeLimit(.minutes(1)))
struct StakingViewModelTests: IsolatedAppStateSuite {
    @Test func changingWalletDiscardsAnOldPositionResponse() async {
        var operations = StakingTestSupport.operations()
        let oldRead = SuspensionGate<[StakingPosition]>()
        operations.positions = { wallet, _, _, _ in
            wallet == "old" ? await oldRead.wait() : [StakingTestSupport.position("new-position")]
        }
        let vm = StakingState(chain: .solana, bridge: bridge, operations: operations)
        vm.selectWallet("old")
        let work = Task { await vm.loadWalletData() }
        await oldRead.reached()
        vm.selectWallet("new")
        await vm.loadWalletData()
        oldRead.resume([StakingTestSupport.position("old-position")])
        await work.value
        #expect(vm.walletId == "new")
        #expect(vm.positions.map(\.id) == ["new-position"])
        #expect(vm.hasLoadedPositions)
    }

    @Test func aLatePositionReadCannotReplaceANewerSnapshotForTheSameWallet() async {
        var operations = StakingTestSupport.operations()
        let oldRead = SuspensionGate<[StakingPosition]>()
        var reads = 0
        operations.positions = { _, _, _, _ in
            reads += 1
            if reads == 1 { return await oldRead.wait() }
            return [StakingTestSupport.position("current-position")]
        }
        let vm = StakingState(chain: .solana, bridge: bridge, operations: operations)
        vm.selectWallet("wallet")
        let old = Task { await vm.loadWalletData() }
        await oldRead.reached()
        await vm.loadWalletData()
        oldRead.resume([StakingTestSupport.position("obsolete-position")])
        await old.value
        #expect(vm.positions.map(\.id) == ["current-position"])
    }

    @Test func closingDuringPreparationAuthorizationNeverUsesTheWalletKey() async throws {
        var operations = StakingTestSupport.operations()
        var built = false
        operations.build = { _, _ in
            built = true
            return StakingTestSupport.artifact()
        }
        let authentication = SuspensionGate<String?>()
        let vm = StakingState(
            chain: .solana, bridge: bridge, operations: operations,
            authentication: { _ in await authentication.wait() })
        vm.selectWallet("wallet")
        vm.validatorId = "validator"
        vm.amount = "1"
        vm.password = "secret"
        vm.begin(.build)
        let id = try #require(vm.request?.id)
        let store = makeState()
        let work = Task { await vm.perform(id, store: store) }
        await authentication.reached()
        vm.cancel()
        authentication.resume(nil)
        await work.value
        #expect(!built)
        #expect(vm.session.artifact == nil)
        #expect(vm.password.isEmpty)
        #expect(!vm.isBusy)
    }

    @Test func closingDuringSigningAuthorizationNeverInvokesSigner() async throws {
        var operations = StakingTestSupport.operations()
        var signed = false
        operations.sign = { _, _, _ in
            signed = true
            return StakingTestSupport.artifact(stage: .signed)
        }
        let authentication = SuspensionGate<String?>()
        let vm = StakingState(
            chain: .solana, bridge: bridge, operations: operations,
            authentication: { _ in await authentication.wait() })
        vm.selectWallet("wallet")
        vm.session.progress = StakingTestSupport.progress(StakingTestSupport.artifact())
        vm.password = "secret"
        vm.begin(.sign)
        let id = try #require(vm.request?.id)
        let store = makeState()
        let work = Task { await vm.perform(id, store: store) }
        await authentication.reached()
        vm.selectWallet("other-wallet")
        authentication.resume(nil)
        await work.value
        #expect(!signed)
        #expect(vm.walletId == "other-wallet")
        #expect(vm.session.artifact == nil)
        #expect(vm.password.isEmpty)
    }

    @Test func buildSignAndBroadcastRequireSeparateIntentAndUseTheReviewedDigest() async throws {
        var operations = StakingTestSupport.operations()
        var builds = 0
        var signs = 0
        var broadcasts = 0
        var completed = false
        operations.build = { request, password in
            builds += 1
            #expect(request.walletId == "wallet")
            #expect(request.amount == "1.234567891")
            #expect(password == "build-password")
            return StakingTestSupport.artifact()
        }
        operations.sign = { id, digest, password in
            signs += 1
            #expect(id == "staking-review")
            #expect(digest == "immutable-review")
            #expect(password == "build-password", "The password typed once signs too")
            return StakingTestSupport.artifact(stage: .signed)
        }
        operations.broadcast = { id, endpoints in
            broadcasts += 1
            #expect(id == "staking-review")
            #expect(endpoints == ["node-b"])
            return StakingTestSupport.artifact(stage: .signed, submitted: true)
        }
        let vm = StakingState(
            chain: .solana, bridge: bridge, operations: operations,
            authentication: { _ in nil }, broadcastCompletion: { _ in completed = true })
        let store = makeState()
        vm.selectWallet("wallet")
        vm.validatorId = "validator"
        vm.amount = "1.234567891"
        vm.password = "build-password"
        vm.begin(.build)
        await vm.perform(try #require(vm.request?.id), store: store)
        #expect(builds == 1 && signs == 0 && broadcasts == 0)
        #expect(vm.session.selectedEndpoints.isEmpty)
        #expect(vm.password == "build-password")
        vm.begin(.sign)
        await vm.perform(try #require(vm.request?.id), store: store)
        #expect(signs == 1 && broadcasts == 0)
        #expect(vm.session.artifact?.stage == .signed)
        vm.session.selectedEndpoints = ["node-b"]
        vm.begin(.broadcast)
        await vm.perform(try #require(vm.request?.id), store: store)
        #expect(broadcasts == 1 && completed)
        #expect(vm.session.progress?.transaction?.status == .pending, "Acceptance never establishes confirmation")
    }

    @Test func broadcastCompletionSurvivesClosingTheScreen() async throws {
        var operations = StakingTestSupport.operations()
        let gate = SuspensionGate<SendArtifact>()
        operations.broadcast = { _, _ in await gate.wait() }
        var completedId: String?
        let vm = StakingState(
            chain: .solana, bridge: bridge, operations: operations,
            broadcastCompletion: { completedId = $0.id })
        vm.selectWallet("wallet")
        vm.session.progress = StakingTestSupport.progress(StakingTestSupport.artifact(stage: .signed))
        vm.session.endpoints = ["node-b"]
        vm.session.selectedEndpoints = ["node-b"]
        vm.begin(.broadcast)
        let id = try #require(vm.request?.id)
        let store = makeState()
        let work = Task { await vm.perform(id, store: store) }
        await gate.reached()
        vm.cancel()
        gate.resume(StakingTestSupport.artifact(stage: .signed, submitted: true))
        await work.value
        #expect(completedId == "staking-review")
        #expect(vm.session.artifact == nil)
    }
    @Test func privateStatusRecheckAuthorizesOnlyTheReadAndKeepsTheOriginalOperation() async throws {
        var operations = StakingTestSupport.operations()
        var reads = 0
        var keyActions = 0
        var authorizations = 0
        operations.build = { _, _ in
            keyActions += 1
            return StakingTestSupport.artifact()
        }
        operations.sign = { _, _, _ in
            keyActions += 1
            return StakingTestSupport.artifact(stage: .signed)
        }
        operations.broadcast = { _, _ in
            keyActions += 1
            return StakingTestSupport.artifact(stage: .signed, submitted: true)
        }
        operations.recheck = { id, password in
            reads += 1
            #expect(id == "staking-review")
            #expect(password == "read-password")
            return StakingTestSupport.artifact(stage: .signed, submitted: true, chain: .icp)
        }
        let vm = StakingState(
            chain: .icp, bridge: bridge, operations: operations,
            authentication: { _ in
                authorizations += 1
                return nil
            })
        vm.selectWallet("wallet")
        vm.session.progress = StakingTestSupport.progress(
            StakingTestSupport.artifact(stage: .signed, submitted: true, chain: .icp))
        vm.password = "read-password"
        vm.begin(.recheck)
        await vm.perform(try #require(vm.request?.id), store: makeState())
        #expect(reads == 1 && authorizations == 1)
        #expect(keyActions == 0, "A receipt query never signs or repeats a staking operation")
        #expect(vm.session.artifact?.id == "staking-review")
        #expect(vm.password == "read-password", "Kept for the page's next step")
    }

    @Test func aFailedBuildKeepsThePasswordAndShowsTheErrorAtTheForm() async throws {
        var operations = StakingTestSupport.operations()
        operations.build = { _, _ in
            throw SpectraBridgeError.InvalidInput(message: LocalizableMessage(template: "Wrong password", args: []))
        }
        let vm = StakingState(
            chain: .solana, bridge: bridge, operations: operations, authentication: { _ in nil })
        vm.selectWallet("wallet")
        vm.validatorId = "validator"
        vm.amount = "1"
        vm.password = "typo"
        vm.begin(.build)
        await vm.perform(try #require(vm.request?.id), store: makeState())
        #expect(vm.password == "typo")
        #expect(vm.stepError != nil)
        #expect(vm.positionsError == nil)
    }

    @Test func resumingLoadsTheSavedReviewWithoutBuildingOrSigningAgain() async throws {
        var operations = StakingTestSupport.operations()
        var keyActions = 0
        operations.build = { _, _ in
            keyActions += 1
            return StakingTestSupport.artifact()
        }
        operations.sign = { _, _, _ in
            keyActions += 1
            return StakingTestSupport.artifact(stage: .signed)
        }
        let restored = StakingTestSupport.artifact("saved-review", stage: .signed)
        operations.inspect = { id in
            #expect(id == "saved-review")
            return restored
        }
        let vm = StakingState(chain: .solana, bridge: bridge, operations: operations)
        vm.selectWallet("wallet")
        vm.begin(.resume("saved-review"))
        await vm.perform(try #require(vm.request?.id), store: makeState())
        #expect(vm.session.artifact == restored)
        #expect(vm.session.endpoints == ["node-a", "node-b"])
        #expect(vm.session.selectedEndpoints.isEmpty)
        #expect(keyActions == 0)
    }

    @Test func repairingAnInterruptedNeuronRequiresANewReviewAndSeparateSigningAndSubmission() async throws {
        var operations = StakingTestSupport.operations()
        var recovered = StakingTestSupport.artifact(chain: .icp)
        recovered.revision = 2
        recovered.reviewDigest = "reviewed-recovery"
        recovered.review.networkFee = NetworkFee(amount: "0", bound: .exact, value: nil)
        recovered.review.staking?.fundingAlreadyCompleted = true
        var repairs = 0
        var signs = 0
        var broadcasts = 0
        operations.repair = { id, password in
            repairs += 1
            #expect(id == "staking-review")
            #expect(password == "repair-password")
            return recovered
        }
        operations.sign = { id, digest, password in
            signs += 1
            #expect(id == "staking-review")
            #expect(digest == "reviewed-recovery")
            #expect(password == "repair-password", "The password typed once signs too")
            var signed = recovered
            signed.stage = .signed
            return signed
        }
        operations.broadcast = { _, nodes in
            broadcasts += 1
            #expect(nodes == ["node-b"])
            return StakingTestSupport.artifact(stage: .signed, submitted: true, chain: .icp)
        }
        let vm = StakingState(
            chain: .icp, bridge: bridge, operations: operations,
            authentication: { _ in nil }, broadcastCompletion: { _ in })
        let store = makeState()
        vm.selectWallet("wallet")
        vm.session.progress = .staged(
            StakingTestSupport.artifact(stage: .signed, submitted: true, chain: .icp),
            transaction: StakingTestSupport.transaction(status: .failed), next: .done, canRepair: true)
        vm.password = "repair-password"
        vm.begin(.repair)
        await vm.perform(try #require(vm.request?.id), store: store)
        #expect(repairs == 1 && signs == 0 && broadcasts == 0)
        #expect(vm.session.artifact == recovered)
        #expect(vm.session.selectedEndpoints.isEmpty)
        #expect(vm.session.progress?.transaction == nil)
        #expect(vm.password == "repair-password")
        vm.begin(.sign)
        await vm.perform(try #require(vm.request?.id), store: store)
        #expect(signs == 1 && broadcasts == 0)
        vm.session.selectedEndpoints = ["node-b"]
        vm.begin(.broadcast)
        await vm.perform(try #require(vm.request?.id), store: store)
        #expect(broadcasts == 1)
    }

    /// Repairing is core's to allow: a review core does not offer it on is
    /// not rebuilt, whatever its stage.
    @Test func repairWaitsForCoreToAllowIt() async throws {
        var operations = StakingTestSupport.operations()
        var repairs = 0
        operations.repair = { id, _ in
            repairs += 1
            return StakingTestSupport.artifact(id, chain: .icp)
        }
        let vm = StakingState(
            chain: .icp, bridge: bridge, operations: operations, authentication: { _ in nil })
        vm.selectWallet("wallet")
        vm.session.progress = .staged(
            StakingTestSupport.artifact(stage: .signed, submitted: true, chain: .icp),
            transaction: StakingTestSupport.transaction(status: .confirmed), next: .done, canRepair: false)
        vm.begin(.repair)
        await vm.perform(try #require(vm.request?.id), store: makeState())
        #expect(repairs == 0)
    }
}

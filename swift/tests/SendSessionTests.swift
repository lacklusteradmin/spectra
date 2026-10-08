import Foundation
import Testing
@testable import Spectra

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct SendSessionTests {
    private func artifact(_ id: String, stage: SendStage = .prepared) -> SendArtifact {
        SendArtifact(id: id, revision: 0, stage: stage, walletId: "wallet", chainId: .ethereum,
            sender: "sender", recipient: "recipient", amount: "1.000000000000000001", asset: "ETH", symbol: "ETH", staking: nil, operation: nil,
            createdAt: 0, reviewDigest: "digest-\(id)",
            review: SendArtifactReview(warnings: [.newAddress], recipientWarnings: [], requiresSelfSendConfirmation: true, staking: nil, transferTerms: nil),
            preparedDetails: "exact transaction", signingPayloadHex: "", signedPayload: nil,
            transactionHash: nil, attempts: [], selectedEndpoints: [])
    }

    @Test func lateEndpointResponseCannotMixTwoResumedArtifacts() async {
        let session = SendSession()
        let oldGate = SuspensionGate<[String]>()
        let newGate = SuspensionGate<[String]>()
        let old = Task {
            await session.load(operation: .resume, prepare: { self.artifact("old") },
                endpoints: { _ in await oldGate.wait() })
        }
        await oldGate.reached()
        #expect(session.artifact == nil, "Do not expose an artifact before its endpoints arrive")
        session.reset()
        let current = Task {
            await session.load(operation: .resume, prepare: { self.artifact("new") },
                endpoints: { _ in await newGate.wait() })
        }
        await newGate.reached()
        oldGate.resume(["old-node"])
        let oldAdopted = await old.value
        #expect(!oldAdopted)
        #expect(session.operation == .resume, "Old cleanup must not clear the new request's busy state")
        newGate.resume(["new-node"])
        let adopted = await current.value
        #expect(adopted)
        #expect(session.artifact?.id == "new")
        #expect(session.endpoints == ["new-node"])
        #expect(session.operation == nil)
    }

    @Test func closingDuringAuthenticationNeverCallsSigner() async {
        let session = SendSession()
        session.artifact = artifact("old")
        let authentication = SuspensionGate<String?>()
        var signed = false
        let work = Task {
            await session.sign(password: nil, authenticate: { await authentication.wait() }, sign: { _, _, _ in
                signed = true
                return self.artifact("old", stage: .signed)
            })
        }
        await authentication.reached()
        session.reset()
        session.artifact = artifact("new")
        authentication.resume(nil)
        await work.value
        #expect(!signed)
        #expect(session.artifact?.id == "new")
        #expect(session.error == nil)
    }

    @Test func closedBuildFailureCannotOverwriteNewSessionError() async {
        let session = SendSession()
        let gate = SuspensionGate<Bool>()
        let work = Task {
            await session.load(operation: .build, prepare: {
                _ = await gate.wait()
                throw NSError(domain: "obsolete", code: 1)
            }, endpoints: { _ in Issue.record("Failed builds have no endpoints"); return [] })
        }
        await gate.reached()
        session.reset()
        session.error = "current error"
        gate.resume(true)
        let adopted = await work.value
        #expect(!adopted)
        #expect(session.error == "current error")
        #expect(session.artifact == nil)
    }

    @Test func broadcastCompletionAfterCloseStillReturnsCommittedResultWithoutReopeningArtifact() async {
        let session = SendSession()
        session.artifact = artifact("old", stage: .signed)
        session.endpoints = ["selected", "unselected"]
        session.selectedEndpoints = ["selected"]
        let gate = SuspensionGate<Bool>()
        let work = Task {
            await session.broadcast { id, endpoints in
                #expect(id == "old")
                #expect(endpoints == ["selected"])
                _ = await gate.wait()
                return self.artifact("old", stage: .signed)
            }
        }
        await gate.reached()
        session.reset()
        session.artifact = artifact("new")
        session.error = "New session error"
        gate.resume(true)
        let result = await work.value
        #expect(result?.id == "old", "Application completion must run even after the composer closes")
        #expect(session.artifact?.id == "new")
        #expect(session.error == "New session error")
    }

    @Test func broadcastCompletionRefreshesApplicationAfterComposerIsReplaced() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let service = try WalletService(endpoints: [])
        let bridge = WalletServiceBridge(databasePath: directory.appendingPathComponent("state.sqlite").path, service: service)
        _ = try await bridge.ready()
        let store = AppState(bridge: bridge, startServices: false)
        let wallet = WalletView(name: "Sender", chainId: Chain.ethereum, addresses: [Chain.ethereum: "0x1111111111111111111111111111111111111111"])
        _ = try await bridge.ready().applyStateCommand(command: .upsertWallet(wallet: wallet.walletState()))
        store.sendFlow.session.artifact = artifact("old", stage: .signed)
        let gate = SuspensionGate<Bool>()
        let work = Task {
            guard let result = await store.sendFlow.session.broadcast(submit: { _, _ in
                _ = await gate.wait()
                let record = TransactionRecord(id: "old", walletId: wallet.id, kind: .send, status: .pending,
                    walletName: wallet.name, assetDisplayName: "Ether", symbol: "ETH", chainId: Chain.ethereum,
                    amount: "1", address: "0x2222222222222222222222222222222222222222")
                _ = try await service.applyTransactionCommand(command: .upsert(records: [record]))
                return self.artifact("old", stage: .signed)
            }) else { return }
            await store.handleBroadcastCompletion(result)
        }
        await gate.reached()
        store.sendFlow.reset()
        store.sendFlow.session.artifact = artifact("new")
        gate.resume(true)
        await work.value
        #expect(store.transactions.map(\.id) == ["old"])
        #expect(store.transactionCount == 1)
        #expect(store.sendFlow.session.artifact?.id == "new")
    }
}

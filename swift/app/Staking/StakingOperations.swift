import Foundation

/// The screen calls these operations on core. Tests can suspend an individual
/// operation to exercise cancellation without opening a network connection.
@MainActor
struct StakingOperations {
    var validators: @MainActor (Chain) async throws -> [StakingValidator]
    var positions: @MainActor (String, Chain, [String], String?) async throws -> [StakingPosition]
    var saved: @MainActor () async throws -> [SendArtifact]
    var build: @MainActor (StakingRequest, String?) async throws -> SendArtifact
    var endpoints: @MainActor (Chain) async throws -> [String]
    var sign: @MainActor (String, String, String?) async throws -> SendArtifact
    var broadcast: @MainActor (String, [String]) async throws -> SendArtifact
    var inspect: @MainActor (String) async throws -> SendArtifact
    var transaction: @MainActor (String) async throws -> TransactionRecord?
    var recheck: @MainActor (String, String?) async throws -> SendArtifact
    var repair: @MainActor (String, String?) async throws -> SendArtifact

    static func live(bridge: WalletServiceBridge) -> Self {
        Self(
            validators: { try await bridge.ready().fetchStakingValidators(chainId: $0) },
            positions: {
                try await bridge.ready().fetchStakingPositions(walletId: $0, chainId: $1, targets: $2, password: $3)
            },
            saved: { try await bridge.ready().listSends() },
            build: { try await bridge.ready().buildStaking(request: $0, password: $1) },
            endpoints: { try await bridge.ready().stakingBroadcastEndpoints(chainId: $0) },
            sign: { try await bridge.ready().signSend(id: $0, reviewDigest: $1, password: $2) },
            broadcast: { try await bridge.ready().broadcastSend(id: $0, endpoints: $1) },
            inspect: { try await bridge.ready().inspectSend(id: $0) },
            transaction: { try await bridge.ready().transaction(id: $0) },
            recheck: { try await bridge.ready().recheckStaking(id: $0, password: $1) },
            repair: { try await bridge.ready().repairStaking(id: $0, password: $1) }
        )
    }
}

import Foundation
import Testing
@testable import Spectra

@MainActor
@Suite(.isolatedAppState)
struct AssetPrecisionBridgeTests: IsolatedAppStateSuite {
    @Test func precisionSnapshotUpdatesAndRejectsStaleResults() async throws {
        let store = makeState()
        #expect(store.amounts.formattedAssetAmountValue("1", deploymentId: "ethereum:native") == "—")
        let contract = "0x1111111111111111111111111111111111111111"
        _ = try await bridge.ready().applyStateCommand(command: .addCustomToken(
            chainId: Chain.ethereum, symbol: "CUSTOM", name: "Custom", contract: contract,
            coingeckoId: "", coinpaprikaId: "", decimals: 6))
        let old = try await bridge.ready().portfolioSnapshot()
        let id = "ethereum:erc-20:\(contract)"
        store.applyPortfolioSnapshot(old)
        #expect(store.assetPrecision?.byDeploymentId[id] == 6)
        #expect(store.assetPrecision?.byDeploymentId["bitcoin:native"] == 8)
        #expect(store.assetPrecision?.byDeploymentId["peercoin:native"] == 6)
        #expect(store.assetPrecision?.byDeploymentId["peercoin-testnet:native"] == 6)
        _ = try await bridge.ready().applyStateCommand(command: .setCustomTokenDecimals(chainId: Chain.ethereum, contract: contract, decimals: 4))
        let current = try await bridge.ready().portfolioSnapshot()
        store.applyPortfolioSnapshot(current)
        store.applyPortfolioSnapshot(old)
        #expect(store.assetPrecision?.byDeploymentId[id] == 4)
        // The render path uses the coherent core snapshot.
        // Cut, never rounded up.
        #expect(store.amounts.formattedAssetAmountValue("0.123456", deploymentId: id).hasSuffix("1234"))
        _ = try await bridge.ready().applyStateCommand(command: .removeCustomToken(chainId: Chain.ethereum, contract: contract))
        store.applyPortfolioSnapshot(try await bridge.ready().portfolioSnapshot())
        #expect(store.assetPrecision?.byDeploymentId[id] == nil)
        #expect(store.assetPrecision?.unknownDecimals == 18)
    }
}

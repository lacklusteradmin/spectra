import Foundation
import Testing
@testable import Spectra
@MainActor
@Suite(.isolatedAppState)
struct WalletDiagnosticsStateTests: IsolatedAppStateSuite {
    /// Core marks a chain degraded or healthy as part of the refresh that
    /// found it so; this state only adopts what core recorded.
    @Test func aDegradedChainShowsABannerAndSurvivesAReload() async throws {
        _ = try await bridge.ready().applyDiagnosticCommand(command: 
            .degraded(chainId: Chain.ethereum, reason: .failed(message: "Ethereum refresh timed out. Using cached balances and history.")))
        let state = WalletDiagnosticsState(bridge: bridge)
        await state.loadFromSQLite()
        #expect(state.chainDegradedBanners.count == 1)
        #expect(state.chainDegradedBanners.first?.chain == Chain.ethereum)
        #expect(state.chainDegradedBanners.first?.message.contains("Ethereum refresh timed out.") == true)
        #expect(state.operationalLogs.count == 1)
        #expect(state.operationalLogs.first?.input.level == .warning)
        #expect(state.operationalLogs.first?.input.chainId == Chain.ethereum)
    }
    @Test func aHealthyChainClearsItsBannerAndLogsTheRecovery() async throws {
        _ = try await bridge.ready().applyDiagnosticCommand(command: .degraded(chainId: Chain.solana, reason: .historyRefreshFailed))
        _ = try await bridge.ready().applyDiagnosticCommand(command: .healthy(chainId: Chain.solana))
        let state = WalletDiagnosticsState(bridge: bridge)
        await state.loadFromSQLite()
        #expect(state.chainDegradedBanners.isEmpty)
        #expect(state.operationalLogs.count == 2)
        #expect(state.operationalLogs.first?.input.level == .info)
        #expect(state.operationalLogs.first?.input.chainId == Chain.solana)
        #expect(state.operationalLogs.first?.input.message == "Chain recovered")
    }
    /// An appended line crosses the binding with every field. Trimming and
    /// the 800-line cap are core's rules, tested in `operational_events.rs`.
    @Test func appendedLogCrossesTheBindingWithEveryField() async throws {
        let state = WalletDiagnosticsState(bridge: bridge)
        state.appendOperationalLog(category: "Network", message: "Request failed", chain: Chain.bitcoin, source: "rpc")
        await state.flushPendingPersistence()
        #expect(state.operationalLogs.first?.input.level == .error)
        #expect(state.operationalLogs.first?.input.category == "Network")
        #expect(state.operationalLogs.first?.input.message == "Request failed")
        #expect(state.operationalLogs.first?.input.chainId == Chain.bitcoin)
        #expect(state.operationalLogs.first?.input.source == "rpc")
    }
    @Test func exportOperationalLogsTextIncludesHeaderAndFields() async throws {
        let state = WalletDiagnosticsState(bridge: bridge)
        state.appendOperationalLog(
            category: "Chain Sync", message: "Ethereum refresh timed out.", chain: Chain.ethereum, source: "network")
        await state.flushPendingPersistence()
        let text = state.exportOperationalLogsText(networkSyncStatusText: "Network Status: Healthy")
        #expect(text.contains("Spectra Operational Logs"))
        #expect(text.contains("Entries: 1"))
        #expect(text.contains("Network Status: Healthy"))
        #expect(text.contains("[ERROR]"))
        #expect(text.contains("source=network"))
        #expect(text.contains("chain=\(Chain.ethereum.id)"))
    }

}

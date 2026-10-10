import Foundation

// The diagnostics screen's actions. Swift holds which runs are in flight; core
// runs them, records what they found and logs how they went.
extension AppState {
    func runHistoryDiagnostics(for chain: Chain) async {
        await chainDiagnosticsState.run(\.runningHistory, chain: chain) {
            // No deadline of its own: a UniFFI call does not stop when its Swift
            // task is cancelled, and core's transport timeouts bound the run.
            await self.refreshHistory(chain: chain)
        }
    }

    /// Core probes the network the family is on and keeps the result.
    func runEndpointDiagnostics(for chain: Chain) async {
        await chainDiagnosticsState.run(\.checkingEndpoints, chain: chain) {
            do {
                _ = try await self.bridge.ready().probeChainEndpoints(chain: chain)
            } catch {
                self.appendOperationalLog(category: "Endpoints", message: error.localizedDescription, chain: chain)
            }
        }
    }

    /// Core resolves the selected network and effective RPC, runs the tests
    /// and logs their outcome, which the chain's operational events show.
    func runSelfTests(for chain: Chain) async {
        await chainDiagnosticsState.run(\.runningSelfTests, chain: chain) {
            do {
                _ = try await self.bridge.ready().runConfiguredSelfTests(chain: chain)
            } catch {
                self.appendOperationalLog(category: "Self-Tests", message: error.localizedDescription, chain: chain)
            }
            await self.diagnostics.loadFromSQLite()
        }
    }

    /// Core runs the rescan and logs how it went.
    func runUTXORescan(chain: Chain) async {
        await chainDiagnosticsState.run(\.runningRescans, chain: chain) {
            await self.performCoreRefresh(.deepRescan(chainId: chain))
        }
    }

    /// A family's diagnostics as core recorded them, on the network it is on.
    func chainDiagnostics(for chain: Chain) async throws -> ChainDiagnostics {
        try await bridge.ready().chainDiagnostics(chain: chain)
    }

    /// Read-only keypool diagnostics. Reading does not reserve an address.
    /// The reserved address and path are those recorded when the index was handed out.
    func chainKeypoolDiagnostics(for chain: Chain) async throws -> [KeypoolDiagnostic] {
        try await bridge.ready().keypoolDiagnostics(chain: chain)
    }

    func operationalEvents(for chain: Chain) async -> [DiagnosticLog] {
        (try? await bridge.ready().operationalEvents(chainId: chain)) ?? []
    }
}

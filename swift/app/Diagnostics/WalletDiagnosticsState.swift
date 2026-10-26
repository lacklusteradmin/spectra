import Foundation

/// Why a chain's data is stale, worded for the dashboard notice.
struct ChainDegradedBanner: Identifiable {
    let chain: Chain
    let message: String
    let lastGoodSyncAt: Date?
    var id: Chain { chain }
    var chainName: String { chain.displayName }
}

@MainActor
@Observable
final class WalletDiagnosticsState {
    @ObservationIgnored private let bridge: WalletServiceBridge // Persistence dependency.
    init(bridge: WalletServiceBridge = .shared) { self.bridge = bridge }

    private static let operationalLogTimestampFormatter = ISO8601DateFormatter()
    private var snapshot = DiagnosticState(degraded: [:], lastGoodUnix: [:], logs: [])
    private(set) var operationalLogs: [DiagnosticLog] = []
    private(set) var operationalLogsRevision: UInt64 = 0
    @ObservationIgnored private(set) var pendingCommand: Task<Void, Never>?
    @ObservationIgnored private var revision: UInt64 = 0

    private func adopt(_ state: DiagnosticState) {
        snapshot = state
        operationalLogs = state.logs
        operationalLogsRevision &+= 1
    }
    private func enqueue(_ command: DiagnosticCommand) {
        revision &+= 1
        let previous = pendingCommand
        let bridge = self.bridge
        // A queued event finishes even when its diagnostics view is closed.
        pendingCommand = Task { @MainActor [weak self] in
            await previous?.value
            // A diagnostics write that fails has nowhere to be logged; the
            // next read adopts whatever core kept.
            guard let result = try? await bridge.ready().applyDiagnosticCommand(command: command) else { return }
            self?.adopt(result)
        }
    }
    func loadFromSQLite() async {
        await pendingCommand?.value
        let started = revision
        guard let state = try? await bridge.ready().diagnosticState(), started == revision else { return }
        adopt(state)
    }
    private var lastGoodSyncByChain: [Chain: Date] { snapshot.lastGoodUnix.mapValues { Date(timeIntervalSince1970: $0) } }
    /// One banner per degraded chain, ordered by name. Core keys both maps by chain.
    var chainDegradedBanners: [ChainDegradedBanner] {
        snapshot.degraded.map { chain, reason in
            ChainDegradedBanner(
                chain: chain, message: localizedDegradedMessage(reason, chain: chain),
                lastGoodSyncAt: lastGoodSyncByChain[chain])
        }.sorted { $0.chainName.localizedCaseInsensitiveCompare($1.chainName) == .orderedAscending }
    }
    func clearOperationalLogs() { enqueue(.clearLogs(chainId: nil)) }
    func exportOperationalLogsText(networkSyncStatusText: String, events: [DiagnosticLog]? = nil) -> String {
        let entries = events ?? operationalLogs
        let header = [
            AppLocalization.string("Spectra Operational Logs"),
            AppLocalization.format("Generated: %@", Self.operationalLogTimestampFormatter.string(from: Date())),
            AppLocalization.format("Entries: %d", entries.count), networkSyncStatusText, "",
        ]
        let lines = entries.map { log in
            let event = log.input
            var parts: [String] = [
                Self.operationalLogTimestampFormatter.string(from: log.timestamp), "[\(event.level.exportTag)]",
                "[\(event.category)]", event.message,
            ]
            if let source = event.source, !source.isEmpty { parts.append("source=\(source)") }
            if let chain = event.chainId { parts.append("chain=\(chain.id)") }
            if let transactionHash = event.transactionHash, !transactionHash.isEmpty { parts.append("tx=\(transactionHash)") }
            return parts.joined(separator: " | ")
        }
        return (header + lines).joined(separator: "\n")
    }
    /// Every line this side writes is a failure; core logs the rest itself.
    func appendOperationalLog(category: String, message: String, chain: Chain? = nil, source: String? = nil) {
        enqueue(.append(input: DiagnosticLogInput(level: .error, category: category, message: message,
            chainId: chain, transactionHash: nil, source: source)))
    }
    /// Core stores why a chain is stale; the sentence is worded here.
    private func localizedDegradedMessage(_ reason: ChainDegradation, chain: Chain) -> String {
        let chainName = chain.displayName
        let detail: String
        switch reason {
        case .historyRefreshFailed:
            detail = AppLocalization.format("%@ history refresh failed. Using cached history.", chainName)
        case .historyPartiallyLoaded:
            detail = AppLocalization.format("%@ history loaded with partial provider failures.", chainName)
        case .failed(let message):
            detail = message
        }
        return [detail, degradedSyncSuffix(for: chain)].filter { !$0.isEmpty }.joined(separator: " ")
    }
    private func degradedSyncSuffix(for chain: Chain) -> String {
        let copy = DiagnosticsContentCopy.current
        if let lastGood = lastGoodSyncByChain[chain] {
            return String(
                format: copy.degradedLastGoodSyncFormat, lastGood.appFormatted(time: .shortened)
            )
        }
        return copy.degradedNoPriorSuccessfulSyncYet
    }
}

/// Which diagnostics runs are in flight, keyed by chain, and a revision
/// that tells screens to re-read what core recorded. Results live in core.
@MainActor
@Observable
final class WalletChainDiagnosticsState {
    var diagnosticsRevision: Int = 0
    var runningHistory: Set<Chain> = []
    var checkingEndpoints: Set<Chain> = []
    var runningSelfTests: Set<Chain> = []
    var runningRescans: Set<Chain> = []

    /// Hold `chain`'s slot in `runs` for `operation`, then tell screens to
    /// re-read. A run already in flight is not started twice.
    func run(
        _ runs: ReferenceWritableKeyPath<WalletChainDiagnosticsState, Set<Chain>>, chain: Chain,
        _ operation: () async -> Void
    ) async {
        guard self[keyPath: runs].insert(chain).inserted else { return }
        await operation()
        self[keyPath: runs].remove(chain)
        diagnosticsRevision &+= 1
    }
}

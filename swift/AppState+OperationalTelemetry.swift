import Foundation
import SwiftUI
extension AppState {
    var networkSyncStatusText: String {
        let reachability = isNetworkReachable ? AppLocalization.string("reachable") : AppLocalization.string("offline")
        let constrained = isConstrainedNetwork ? AppLocalization.string("constrained") : AppLocalization.string("unconstrained")
        let expensive = isExpensiveNetwork ? AppLocalization.string("expensive") : AppLocalization.string("non-expensive")
        return AppLocalization.format(
            "Network: %@, %@, %@", reachability, constrained, expensive
        )
    }
    func exportOperationalLogsText(events: [DiagnosticLog]? = nil) -> String {
        diagnostics.exportOperationalLogsText(networkSyncStatusText: networkSyncStatusText, events: events)
    }
    /// Log a failure on this side of the boundary — a platform API, or a call
    /// into core that threw. Core logs the work it performs itself.
    func appendOperationalLog(
        _ level: DiagnosticLogLevel, category: String, message: String, chain: Chain? = nil, walletId: String? = nil,
        transactionHash: String? = nil, source: String? = nil, metadata: String? = nil
    ) {
        diagnostics.appendOperationalLog(
            level, category: category, message: message, chain: chain, walletId: walletId, transactionHash: transactionHash,
            source: source, metadata: metadata
        )
    }
}

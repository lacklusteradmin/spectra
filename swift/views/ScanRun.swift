import Foundation
import Observation

/// One run of a private-funds scan at a time — a Zcash wallet's shielded
/// pools, a Litecoin wallet's MWEB funds — each run with its own lifetime.
/// Core keeps every batch, so a cancelled run resumes where it stopped.
@MainActor
@Observable
final class ScanRun {
    /// The password the first batch derives the scanning keys with.
    var password = ""
    var error: String?
    private(set) var requestId: UUID?
    var isRunning: Bool { requestId != nil }

    func begin() {
        guard requestId == nil else { return }
        requestId = UUID()
    }

    func cancel() {
        requestId = nil
        password = ""
    }

    func isCurrent(_ request: UUID) -> Bool { requestId == request && !Task.isCancelled }

    func finish(_ request: UUID) {
        if requestId == request { cancel() }
    }
}

/// What a private-funds page's sheet does: move the wallet's transparent
/// funds in, or pay from the private ones.
enum PrivateFundsFlow: String, Identifiable {
    case moveIn, send
    var id: String { rawValue }
}

import Foundation

/// Native scan controls and progress; each run has its own lifetime.
@MainActor
@Observable
final class MoneroSyncViewModel {
    var status: MoneroSyncStatus?
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

    private func isCurrent(_ request: UUID) -> Bool {
        requestId == request && !Task.isCancelled
    }

    func sync(request: UUID,
              operation: @MainActor (String?, (MoneroSyncStatus) -> Void) async -> String?) async {
        guard isCurrent(request) else { return }
        defer {
            if requestId == request {
                password = ""
                requestId = nil
            }
        }
        error = nil
        let failure = await operation(password.isEmpty ? nil : password) { status in
            guard self.isCurrent(request) else { return }
            self.status = status
        }
        guard isCurrent(request) else { return }
        error = failure
    }
}

import Foundation

/// Transient native flow state; persisted domain data remains in core.
@MainActor
@Observable
final class ReceiveFlowState {
    var walletId: String = ""
    var holdingKey: String = ""
    var resolvedAddress: String = ""
    var error: String?
    @ObservationIgnored var requestId = UUID() // Reject stale asynchronous results.
    var isResolving: Bool = false
    var isPresented: Bool = false {
        didSet { if oldValue && !isPresented { clearAddress() } }
    }

    func clearAddress() {
        requestId = UUID()
        resolvedAddress = ""
        error = nil
        isResolving = false
    }

    func reset() {
        clearAddress()
        walletId = ""
        holdingKey = ""
        isPresented = false
    }
}

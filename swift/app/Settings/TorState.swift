import Foundation

/// Live Tor bootstrap/connection state, as core's refresh engine reports it.
/// Drives the dashboard indicator and the settings status row.
///
/// Whether Tor runs follows core's committed settings; core applies them and
/// reports every status change through the refresh observer. This only asks
/// for a reconnect.
@MainActor
@Observable
final class TorState {
    @ObservationIgnored private let bridge: WalletServiceBridge // Service identity is not view state.
    private(set) var status: TorStatus = .stopped

    init(bridge: WalletServiceBridge) { self.bridge = bridge }

    /// The refresh observer's writer.
    func adopt(_ status: TorStatus) { self.status = status }

    func reconnect() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            do { self.status = try await self.bridge.ready().reconnectTor() }
            catch { self.status = .error(message: userErrorMessage(error)) }
        }
    }
}

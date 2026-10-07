import Foundation

extension AppState {
    func moneroSyncStatus(walletId: String) async throws -> MoneroSyncStatus? {
        try await bridge.ready().moneroSyncStatus(walletId: walletId)
    }

    /// Scan a Monero wallet to the chain tip on this device, one core batch at
    /// a time, reporting each batch's status. `nil` once scanned or cancelled;
    /// otherwise why it stopped. Core persists every completed batch, so a
    /// cancelled scan resumes where it left off. The first batch starts at the
    /// restore height the wallet was imported with.
    func syncMoneroWallet(
        walletId: String, password: String?, progress: (MoneroSyncStatus) -> Void
    ) async -> String? {
        if let failure = await authenticate(.send, reason: AppLocalization.string("Authorize local wallet sync")) {
            return failure
        }
        do {
            while true {
                try Task.checkCancellation()
                let status = try await bridge.ready().syncMoneroWallet(walletId: walletId, password: password)
                progress(status)
                if status.complete { break }
            }
        } catch is CancellationError {
            return nil
        } catch {
            return userErrorMessage(error)
        }
        await refreshBalances()
        return nil
    }
}

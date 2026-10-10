import Foundation

/// The app's one `WalletService` and the refresh engine on it.
///
/// Callers use core's API directly: `try await bridge.ready().portfolioSnapshot()`.
/// This type only owns what core cannot know — which database file and cache
/// directory this device uses, and the Keychain-backed secret store — and
/// makes every async call wait for the same database binding.
@MainActor final class WalletServiceBridge {
    static let shared = WalletServiceBridge()
    private let suppliedService: WalletService?
    private let databasePath: String?
    private var stateIsOpen = false
    private var _service: WalletService?
    private var _refreshEngine: RefreshEngine?

    init(databasePath: String? = nil, service: WalletService? = nil) {
        self.databasePath = databasePath
        self.suppliedService = service
    }

    /// The service, bound to its database. Core serializes concurrent opens
    /// and retries failures without caching them.
    func ready() async throws -> WalletService {
        let svc = try service()
        if !stateIsOpen {
            _ = try await svc.openState(databasePath: sqliteDbPath())
            if suppliedService == nil {
                _ = try await svc.configureNetworkRuntime(cacheDir: torCacheDirectory())
            }
            stateIsOpen = true
        }
        return svc
    }

    /// Delete a store `openState` reported unreadable, with the secrets of the
    /// wallets it names, and bind the empty one core opens in its place.
    func discardUnreadableStore() async throws {
        let svc = try service()
        _ = try await svc.discardState(databasePath: sqliteDbPath())
        if suppliedService == nil {
            _ = try await svc.configureNetworkRuntime(cacheDir: torCacheDirectory())
        }
        stateIsOpen = true
    }

    /// The service without waiting for the database: for synchronous calls
    /// that read the Keychain (seed reveal, funds scan) or register the store.
    /// Those calls block on the Keychain and on key derivation, so callers run
    /// them off the main actor.
    func service() throws -> WalletService {
        if let existing = _service { return existing }
        let svc = try suppliedService ?? WalletService.newCatalog()
        if suppliedService == nil { svc.setSecretStore(store: SpectraSecretStoreAdapter()) }
        _service = svc
        return svc
    }

    private func sqliteDbPath() -> String {
        if let databasePath { return databasePath }
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first?.path ?? NSTemporaryDirectory()
        return "\(docs)/spectra_state.db"
    }

    private func torCacheDirectory() -> String {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?.path ?? NSTemporaryDirectory()
    }
}

// ── Refresh engine ─────────────────────────────────────────────────────
// Core runs the balance sweep and the maintenance tick; the platform reports
// device conditions and receives results through the observer.
extension WalletServiceBridge {
    func refreshEngine() async throws -> RefreshEngine {
        let svc = try await ready()
        if let engine = _refreshEngine { return engine }
        let engine = RefreshEngine(walletService: svc)
        _refreshEngine = engine
        return engine
    }
    func setRefreshObserver(_ observer: RefreshObserver) async throws {
        try await refreshEngine().setObserver(observer: observer)
    }
}

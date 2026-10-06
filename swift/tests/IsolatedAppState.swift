import Foundation
import Testing
@testable import Spectra

/// A throwaway database and wallet service for one test.
///
/// A suite opts in with `@Suite(.isolatedAppState)` and conforms to
/// `IsolatedAppStateSuite` to reach it. The trait opens the state before each
/// test and, after it, lets every `AppState` the test made finish its queued
/// writes before the directory goes.
@MainActor
final class IsolatedAppState {
    let bridge: WalletServiceBridge
    let service: WalletService
    let directory: URL
    private var states: [AppState] = []

    init() async throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        service = try WalletService(endpoints: [])
        service.setSecretStore(store: TestSecretStore())
        bridge = WalletServiceBridge(databasePath: directory.appendingPathComponent("state.sqlite").path, service: service)
        _ = try await bridge.ready()
    }

    func makeState() -> AppState {
        let state = AppState(bridge: bridge, startServices: false)
        states.append(state)
        return state
    }

    func close() async throws {
        for state in states {
            await state.stateCommands.awaitPending()
            await state.diagnostics.flushPendingPersistence()
        }
        states.removeAll()
        try FileManager.default.removeItem(at: directory)
    }
}

struct IsolatedAppStateScope: SuiteTrait, TestTrait, TestScoping {
    @TaskLocal static var current: IsolatedAppState?

    var isRecursive: Bool { true }

    func provideScope(
        for test: Test, testCase: Test.Case?, performing function: @Sendable () async throws -> Void
    ) async throws {
        let state = try await IsolatedAppState()
        do {
            try await Self.$current.withValue(state) { try await function() }
        } catch {
            try? await state.close()
            throw error
        }
        try await state.close()
    }
}

extension Trait where Self == IsolatedAppStateScope {
    static var isolatedAppState: Self { Self() }
}

@MainActor
protocol IsolatedAppStateSuite {}

extension IsolatedAppStateSuite {
    private var isolated: IsolatedAppState {
        guard let current = IsolatedAppStateScope.current else {
            preconditionFailure("run this suite with @Suite(.isolatedAppState)")
        }
        return current
    }
    var bridge: WalletServiceBridge { isolated.bridge }
    var service: WalletService { isolated.service }
    var directory: URL { isolated.directory }
    func makeState() -> AppState { isolated.makeState() }
}

final class TestSecretStore: SecretStore, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [SecretClass: [String: String]] = [:]
    func loadSecret(kind: SecretClass, key: String) throws -> String {
        try lock.withLock {
            guard let value = values[kind]?[key] else { throw SecretStoreError.NotFound }
            return value
        }
    }
    func saveSecret(kind: SecretClass, key: String, value: String) throws {
        lock.withLock { values[kind, default: [:]][key] = value }
    }
    func deleteSecret(kind: SecretClass, key: String) throws {
        lock.withLock { _ = values[kind]?.removeValue(forKey: key) }
    }
    /// No enclave in a test store: the device key is kept as core minted it.
    func wrapDeviceKey(key: Data) throws -> Data { key }
    func unwrapDeviceKey(wrapped: Data) throws -> Data { wrapped }
}

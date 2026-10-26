import Foundation

/// The one queue every `StateCommand` goes through.
///
/// Core serialises its own writes and stamps each state with a revision, so
/// adoption order is already safe; the queue keeps the order the user acted
/// in, so a later edit is never overtaken by an earlier one. `AppState` owns
/// it, adopts what each command committed, and hands it to the domain objects
/// that send commands of their own.
@MainActor
final class StateCommandQueue {
    private let bridge: WalletServiceBridge
    /// The tail of the queue: it settles once every command issued so far has.
    private var tail: Task<Void, Never>?
    /// Adopts what a command committed, before the sender's handler runs.
    /// `AppState` sets it as soon as it exists.
    var adopt: @MainActor (StateTransition) async -> Void = { _ in }

    init(bridge: WalletServiceBridge) { self.bridge = bridge }

    deinit { tail?.cancel() }

    /// Send a command after every command issued before it, then adopt what
    /// core committed. `then` runs after adoption, inside the queue, with the result.
    @discardableResult
    func enqueue(
        _ command: StateCommand,
        then handle: @escaping @MainActor (Result<StateTransition, Error>) async -> Void = { _ in }
    ) -> Task<StateTransition, Error> {
        let previous = tail
        let task = Task { @MainActor [weak self] () throws -> StateTransition in
            await previous?.value
            guard let self else { throw CancellationError() }
            do {
                let transition = try await self.bridge.ready().applyStateCommand(command: command)
                await self.adopt(transition)
                await handle(.success(transition))
                return transition
            } catch {
                await handle(.failure(error))
                throw error
            }
        }
        tail = Task { _ = try? await task.value }
        return task
    }

    /// Send a command, wait for it to be committed and adopted, and return it.
    @discardableResult
    func apply(_ command: StateCommand) async throws -> StateTransition {
        try await enqueue(command).value
    }

    /// Wait until every command issued so far has been committed and adopted.
    func awaitPending() async {
        await tail?.value
    }
}

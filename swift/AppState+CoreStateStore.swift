// MARK: - State commands
//
// Core is the canonical store for wallets, settings, contacts, tokens, alerts
// and pins; the `@Observable` values on AppState and its domain objects are
// projections of it, with one writer each. Every change is a `StateCommand`
// sent through `stateCommands`, and what core committed lands back through
// `applyCoreState` and the portfolio snapshot. Assigning a projection directly
// is a bug.

import Foundation

extension AppState {
    /// What the command queue does with each committed command, before the
    /// sender's own handler runs: adopt core's state and the portfolio it changed.
    func adoptCommittedTransition(_ transition: StateTransition) async {
        applyCoreState(transition.state)
        await rebuildWalletDerivedStateFromCore()
    }

    /// Load core's state and mirror it. `ready()` has opened the database, so
    /// this reads what core holds rather than opening it again.
    func loadCoreOwnedState() async {
        do {
            let state = try await self.bridge.ready().appState()
            applyCoreState(state)
        } catch {
            appendOperationalLog(.error, category: "Storage", message: error.localizedDescription)
        }
    }

    func reloadCoreProjections() async {
        // Core-owned domain state first: it is the authority, so anything
        // loaded after it must not contradict it.
        await loadCoreOwnedState()
        await diagnostics.loadFromSQLite()
        // Opening the state folds this build's built-in tokens in, and carries
        // settings, alerts and contacts; the five preferences this platform
        // keeps were read from `UserDefaults` when `preferences` was created.
        await rebuildWalletDerivedStateFromCore()
        await refreshTransactionProjection()
    }

    /// Send a command whose only failure surface is `commandError`.
    func sendStateCommand(_ command: StateCommand) {
        stateCommands.enqueue(command) { [weak self] result in
            switch result {
            case .success: self?.commandError = nil
            case .failure(let error): self?.reportCommandError(error)
            }
        }
    }

    /// Show a failed command on the dashboard. The message drops the detail,
    /// so the log keeps it.
    func reportCommandError(_ error: Error) {
        commandError = userErrorMessage(error)
        appendOperationalLog(.error, category: "State", message: String(describing: error))
    }

    @discardableResult
    func removeWallet(id: String) async -> Bool {
        do {
            try await stateCommands.apply(.removeWallet(walletId: id))
            await refreshTransactionProjection()
            commandError = nil
            return true
        } catch {
            reportCommandError(error)
            return false
        }
    }
}

import Foundation

/// The known-token projection — the catalog and the user's custom tokens — as
/// core last committed it.
///
/// Every rule about a token — the symbol, the contract's format for the chain
/// that would host it, the duplicate, the precision, and where the row sorts —
/// is the reducer's. Core decides, a refusal comes back as an event carrying
/// its reason, and this side supplies the words.
@MainActor
@Observable
final class TokenPreferencesState {
    @ObservationIgnored private let commands: StateCommandQueue // Where edits go; not view state.
    @ObservationIgnored private let diagnostics: WalletDiagnosticsState // Logs failed commands.
    /// Core's list as last adopted. Change it through `addCustom` or `removeCustom`.
    private(set) var entries: [TokenPreferenceEntry] = []
    /// Why core refused the last token-preference change, if it did.
    var error: String?

    init(commands: StateCommandQueue, diagnostics: WalletDiagnosticsState) {
        self.commands = commands
        self.diagnostics = diagnostics
    }

    /// The only writer of `entries`, called with each committed core state.
    func adopt(_ entries: [TokenPreferenceEntry]) {
        if self.entries != entries { self.entries = entries }
    }

    func removeCustom(_ entry: TokenPreferenceEntry) {
        let command = StateCommand.removeCustomToken(chainId: entry.token.chainId, contract: entry.token.contract)
        commands.enqueue(command) { [weak self] result in
            guard let self else { return }
            self.error = self.errorMessage(result)
        }
    }

    /// Teach the wallet a token the catalog does not ship, or edit one it was
    /// taught. Returns the refusal to show beside the form, or `nil` once core
    /// has accepted it.
    /// Add a token under `standard`, one of `chain`'s protocols — the
    /// network's only one when `nil` — or edit `editing`, which keeps its own.
    func addCustom(
        chain: Chain, standard: String? = nil, symbol: String, name: String, contractAddress: String,
        coingeckoId: String = "", coinpaprikaId: String = "", decimals: UInt32, editing: TokenPreferenceEntry? = nil
    ) async -> String? {
        let command: StateCommand
        if let editing {
            command = .updateCustomToken(
                chainId: editing.token.chainId,
                contract: editing.token.contract, symbol: symbol, name: name,
                coingeckoId: coingeckoId, coinpaprikaId: coinpaprikaId, decimals: decimals)
        } else {
            command = .addCustomToken(
                standard: standard, chainId: chain, symbol: symbol, name: name,
                contract: contractAddress, coingeckoId: coingeckoId,
                coinpaprikaId: coinpaprikaId, decimals: decimals)
        }
        let result: Result<StateTransition, Error>
        do { result = .success(try await commands.apply(command)) } catch { result = .failure(error) }
        error = errorMessage(result)
        return error
    }

    /// The words for a token command's outcome, or `nil` when core accepted it.
    private func errorMessage(_ result: Result<StateTransition, Error>) -> String? {
        switch result {
        case .success(let transition):
            for case .tokenPreferenceRejected(let reason) in transition.events {
                return tokenPreferenceRejectionMessage(reason)
            }
            return nil
        case .failure(let error):
            diagnostics.appendOperationalLog(.error, category: "Tokens", message: String(describing: error))
            return userErrorMessage(error)
        }
    }
}

private func tokenPreferenceRejectionMessage(_ reason: TokenPreferenceRejection) -> String {
    switch reason {
    case .unknownChain: return AppLocalization.string("That network cannot hold tokens.")
    case .emptySymbol: return AppLocalization.string("Symbol is required.")
    case .symbolTooLong: return AppLocalization.string("Symbol is too long.")
    case .invalidPriceId: return AppLocalization.string("Enter a price provider ID, not a URL or name.")
    case .emptyName: return AppLocalization.string("Token name is required.")
    case .emptyContract: return AppLocalization.string("Token identifier is required.")
    case .invalidContract: return AppLocalization.string("That token identifier is not valid for this network.")
    case .duplicateToken: return AppLocalization.string("This network already knows this token.")
    case .tooManyDecimals: return AppLocalization.string("That is more decimal places than a token has.")
    case .builtInToken: return AppLocalization.string("Built-in tokens cannot be edited or removed.")
    case .unknownToken: return AppLocalization.string("That token is no longer in the list.")
    case .standardRequired: return AppLocalization.string("Choose the standard the token was issued under.")
    case .unsupportedStandard: return AppLocalization.string("That network does not support this token standard.")
    }
}

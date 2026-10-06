import Foundation

/// The price alerts, as core last committed them.
///
/// Core owns the rules, evaluates them on its refresh and says which fired;
/// this holds the list the alerts screen shows and sends its edits.
@MainActor
@Observable
final class PriceAlertsState {
    @ObservationIgnored private let commands: StateCommandQueue // Where edits go; not view state.
    /// Core's rules as last adopted; edits send individual commands.
    private(set) var rules: [PriceAlertRule] = []

    init(commands: StateCommandQueue) { self.commands = commands }

    /// The only writer of `rules`, called with each committed core state.
    func adopt(_ rules: [PriceAlertRule]) {
        if self.rules != rules { self.rules = rules }
    }

    /// Send a price-alert edit. Core's refusal arrives as an event carrying its
    /// reason, and is thrown here in words.
    func edit(_ command: StateCommand) async throws {
        let transition = try await commands.apply(command)
        for case .priceAlertRejected(let reason) in transition.events {
            throw DisplayedError(priceAlertRejectionMessage(reason))
        }
    }
}

private func priceAlertRejectionMessage(_ reason: PriceAlertRejection) -> String {
    switch reason {
    case .missingCurrencyRate:
        return AppLocalization.string("Exchange rates for this currency have not loaded yet. Try again shortly.")
    case .invalidTarget: return AppLocalization.string("Enter a target price above zero.")
    case .unknownAsset: return AppLocalization.string("This asset is no longer in your wallets.")
    case .duplicateAlert: return AppLocalization.string("An identical alert already exists.")
    case .alertNotFound: return AppLocalization.string("This alert no longer exists.")
    }
}

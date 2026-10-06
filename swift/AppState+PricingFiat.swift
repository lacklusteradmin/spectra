import Foundation
import SwiftUI
extension AppState {
    /// The only writer of the two quote notices, called while adopting a
    /// newer, coherent portfolio snapshot.
    func applyQuoteProjection(_ state: ResidentState) {
        let prices = state.quotes.pricesError.map(priceRefreshMessage)
        let rates = state.quotes.fiatError.map(fiatRateRefreshMessage)
        if quoteRefreshError != prices { quoteRefreshError = prices }
        if fiatRatesRefreshError != rates { fiatRatesRefreshError = rates }
    }

    var portfolioQuotedTotal: QuotedTotal? { portfolioValuation?.portfolio }
    func setPortfolioInclusion(_ isIncluded: Bool, for walletId: String) {
        sendStateCommand(.setWalletPortfolioInclusion(walletId: walletId, included: isIncluded))
    }
    var portfolio: [AssetHolding] { walletDerivedCache.portfolio }
}

/// Core says why the stored prices were kept; the sentence is this app's.
private func priceRefreshMessage(_ failure: QuoteRefreshFailure) -> String {
    switch failure {
    case .noUsableQuote:
        return AppLocalization.string("No price provider returned a usable price. Showing the last known prices.")
    case .unreachable:
        return AppLocalization.string("Couldn't reach a price provider. Showing the last known prices.")
    }
}

/// Core says why the stored rates were kept; the sentence is this app's.
private func fiatRateRefreshMessage(_ failure: QuoteRefreshFailure) -> String {
    switch failure {
    case .noUsableQuote:
        return AppLocalization.string("No exchange-rate provider returned a usable rate. Showing the last known rates.")
    case .unreachable:
        return AppLocalization.string("Couldn't reach an exchange-rate provider. Showing the last known rates.")
    }
}

/// Core's currencies, with the order and name a picker needs.
/// The code comes from core's formatting rules, which carry it.
extension FiatCurrency: CaseIterable, Identifiable {
    private static let catalog = fiatCurrencyCatalog()
    private static let rulesByCurrency = Dictionary(uniqueKeysWithValues: catalog.map { ($0.currency, $0) })
    public static var allCases: [FiatCurrency] { catalog.map(\.currency) }
    var displayRules: FiatAmountRules { Self.rulesByCurrency[self]! }
    public var id: String { code }
    /// The ISO 4217 code.
    var code: String { displayRules.code }
    /// The currency's name and code, in the display language. The system
    /// names every ISO 4217 currency, so no table here has to.
    var displayName: String {
        let name = AppLocalization.locale.localizedString(forCurrencyCode: code) ?? code
        return "\(name) (\(code))"
    }
}

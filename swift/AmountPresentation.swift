import Foundation

/// A render-time value over core projections. No storage, services or side
/// effects, and no money arithmetic: amounts arrive as exact decimals and
/// every fiat figure arrives in the display currency, both from core.
@MainActor
struct AmountPresentation {
    let assetPrecision: AssetPrecisionCatalog?
    let valuation: PortfolioValuation?
    /// The currency the fiat figures are in when core has not valued anything
    /// yet — the user's selection.
    let selectedFiatCurrency: FiatCurrency

    private var currency: FiatCurrency { valuation?.currency ?? selectedFiatCurrency }

    // MARK: - Fiat

    /// A display-currency figure, or "—" when core has none.
    func formattedFiat(_ value: Double?, currency explicit: FiatCurrency? = nil) -> String {
        formattedFiatIfAvailable(value, currency: explicit) ?? "—"
    }
    func formattedFiatIfAvailable(_ value: Double?, currency explicit: FiatCurrency? = nil) -> String? {
        guard let value, value.isFinite else { return nil }
        let currency = explicit ?? currency
        let formatter = AmountFormatters.shared.fiatFormatter(for: currency)
        let minimumVisible = currency.displayRules.minimumVisible
        if value > 0, value < minimumVisible, let threshold = formatter.string(from: NSNumber(value: minimumVisible)) {
            return "<\(threshold)"
        }
        return formatter.string(from: NSNumber(value: value))
    }
    /// The figure alone. An unpriced holding already shows "—" on its own
    /// row, so the total does not repeat it.
    func formattedQuotedTotal(_ total: QuotedTotal?) -> String {
        guard let total, let fiat = total.fiatTotal else { return "—" }
        return formattedFiat(fiat)
    }
    func formattedWalletTotal(walletId: String) -> String {
        formattedQuotedTotal(valuation?.wallets[walletId])
    }
    /// What a wallet's holding is worth, as core valued it.
    func holdingValue(walletId: String, coin: AssetHolding) -> Double? {
        valuation?.holdingValues[walletId]?[coin.id]
    }
    /// One unit of a held asset, as core priced it.
    func price(of coin: AssetHolding) -> Double? { valuation?.prices[coin.id] }
    /// A price alert's target, as core converted it.
    func alertTarget(_ alert: PriceAlertRule) -> Double? { valuation?.alertTargets[alert.id] }

    // MARK: - Asset amounts

    /// An exact decimal with the display locale's decimal separator. No
    /// grouping, and no digit is added or dropped.
    static func localizedDecimal(_ text: String) -> String {
        let separator = AppLocalization.locale.decimalSeparator ?? "."
        return separator == "." ? text : text.replacingOccurrences(of: ".", with: separator)
    }
    /// What a decimal field holds, as core reads it. The decimal pad types
    /// the device region's separator, which may be a comma; core reads only
    /// `.`, so every decimal field goes through this on its way to core.
    static func canonicalDecimalInput(_ text: String, locale: Locale = .current) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let separator = locale.decimalSeparator ?? "."
        return separator == "." ? trimmed : trimmed.replacingOccurrences(of: separator, with: ".")
    }
    /// The reverse: a decimal core wrote, as a decimal field holds it, so a
    /// figure filled in for the user — a shortcut, a position, a scanned
    /// amount — edits with the separator the decimal pad types.
    static func decimalFieldText(_ canonical: String, locale: Locale = .current) -> String {
        let separator = locale.decimalSeparator ?? "."
        return separator == "." ? canonical : canonical.replacingOccurrences(of: ".", with: separator)
    }
    /// The amount alone, as a compact row shows it: core picks the places
    /// and cuts, never rounds up.
    func formattedAssetAmountValue(_ amount: String, deploymentId: String?) -> String {
        guard let decimals = supportedDecimalPlaces(deploymentId: deploymentId),
              let text = formatAssetAmount(amount: amount, assetDecimals: UInt32(decimals))
        else { return "—" }
        let value = Self.localizedDecimal(text.value)
        return text.belowThreshold ? "<" + value : value
    }
    func formattedAssetAmount(_ amount: String, symbol: String, deploymentId: String?) -> String {
        "\(formattedAssetAmountValue(amount, deploymentId: deploymentId)) \(symbol)"
    }
    func formattedTransactionAmount(_ transaction: TransactionRecord) -> String {
        formattedAssetAmount(transaction.amount, symbol: transaction.symbol, deploymentId: transaction.deploymentId)
    }
    /// Every digit the record holds.
    func formattedTransactionDetailAmount(_ transaction: TransactionRecord) -> String {
        "\(Self.localizedDecimal(transaction.amount)) \(transaction.symbol)"
    }

    // MARK: - Network fees

    /// A fee in the chain's gas token, exactly as core stated it.
    func formattedNetworkFee(_ fee: String, chain: Chain) -> String {
        "\(Self.localizedDecimal(fee)) \(chain.gasTokenSymbol)"
    }
    /// The fee with its display-currency value beside it, when core had one.
    func formattedNetworkFee(_ fee: String, value: Double?, chain: Chain) -> String {
        let native = formattedNetworkFee(fee, chain: chain)
        guard let fiat = formattedFiatIfAvailable(value) else { return native }
        return "\(native) (~\(fiat))"
    }
    /// A fee as a compact row shows it: core's significant-digit policy at
    /// the gas token's precision, with its display-currency value when core
    /// had one. The exact figure is the transaction detail's to show.
    func compactNetworkFee(_ fee: String, value: Double?, chain: Chain) -> String {
        let native: String
        if let decimals = chain.nativeDecimals, let text = formatAssetAmount(amount: fee, assetDecimals: decimals) {
            let amount = Self.localizedDecimal(text.value)
            native = "\(text.belowThreshold ? "<" : "")\(amount) \(chain.gasTokenSymbol)"
        } else {
            native = formattedNetworkFee(fee, chain: chain)
        }
        guard let fiat = formattedFiatIfAvailable(value) else { return native }
        return "\(native) (~\(fiat))"
    }
    /// A gas price as a fee row shows it: four significant digits, enough to
    /// compare quotes by. Core states it exactly; the rounding is only for
    /// the row. A receipt states the rate exactly.
    func compactGasPrice(gwei: String) -> String {
        guard let value = Double(gwei) else { return "\(Self.localizedDecimal(gwei)) gwei" }
        return "\(AmountFormatters.shared.gasPriceFormatter.string(from: NSNumber(value: value)) ?? gwei) gwei"
    }
    /// A gas price in gwei, exactly as core stated it: a rate, not an amount
    /// of anything held.
    func formattedGasPrice(gwei: String) -> String {
        "\(Self.localizedDecimal(gwei)) gwei"
    }

    // MARK: - Transaction detail rows

    func receiptEffectiveGasPriceText(for transaction: TransactionRecord) -> String? {
        guard let gwei = transaction.receiptEffectiveGasPriceGwei else { return nil }
        return formattedGasPrice(gwei: gwei)
    }
    func receiptNetworkFeeText(for transaction: TransactionRecord) -> String? {
        guard let fee = transaction.receiptNetworkFee else { return nil }
        let chain = transaction.chain
        return formattedNetworkFee(fee, chain: chain)
    }
    func confirmedNetworkFeeText(for transaction: TransactionRecord) -> String? {
        guard let fee = transaction.confirmedNetworkFee else { return nil }
        let chain = transaction.chain
        return formattedNetworkFee(fee, chain: chain)
    }
    func storedFeeRateText(for transaction: TransactionRecord) -> String? {
        guard let description = transaction.feeRateDescription?.trimmingCharacters(in: .whitespacesAndNewlines),
            !description.isEmpty
        else { return nil }
        return description
    }
    /// What the detail sheet names as the record's history source. Core says
    /// what the stored id means; the sentence around a chain's providers is
    /// this app's to translate, and Spectra's own reader is not named at all.
    func historySourceText(for transaction: TransactionRecord) -> String? {
        switch transaction.transactionHistorySource.flatMap({ historySource(source: $0) }) {
        case .provider(let name): return name
        case .chainProviders(let chainId): return AppLocalization.format("%@ providers", chainId.displayName)
        case .internal, nil: return nil
        }
    }

    private func supportedDecimalPlaces(deploymentId: String?) -> Int? {
        guard let assetPrecision else { return nil }
        return Int(deploymentId.flatMap { assetPrecision.byDeploymentId[$0] } ?? assetPrecision.unknownDecimals)
    }
}

/// Native formatter reuse does not require constructing AppState or opening
/// core. Formatters are rebuilt when the display locale changes.
@MainActor
private final class AmountFormatters {
    static let shared = AmountFormatters()
    private var locale = AppLocalization.locale
    private var cachedCurrencyFormatters: [FiatCurrency: NumberFormatter] = [:]
    private var cachedGasPriceFormatter: NumberFormatter?

    private func matchDisplayLocale() {
        let current = AppLocalization.locale
        guard current != locale else { return }
        locale = current
        cachedCurrencyFormatters = [:]
        cachedGasPriceFormatter = nil
    }
    var gasPriceFormatter: NumberFormatter {
        matchDisplayLocale()
        if let formatter = cachedGasPriceFormatter { return formatter }
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.usesSignificantDigits = true
        formatter.maximumSignificantDigits = 4
        cachedGasPriceFormatter = formatter
        return formatter
    }
    func fiatFormatter(for currency: FiatCurrency) -> NumberFormatter {
        matchDisplayLocale()
        if let formatter = cachedCurrencyFormatters[currency] { return formatter }
        let rules = currency.displayRules
        let decimals = Int(rules.decimals)
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .currency
        formatter.currencyCode = rules.code
        formatter.minimumFractionDigits = decimals
        formatter.maximumFractionDigits = decimals
        cachedCurrencyFormatters[currency] = formatter
        return formatter
    }
}

//! Price and fiat-rate fetching service.
//!
//! Handles every supported provider end-to-end: build the URL, fetch with
//! retry, decode the JSON, and resolve each requested coin to a USD price keyed
//! by its `holding_key`.
//!
//! A coin the provider does not quote is simply absent from the map. Nothing
//! here substitutes a constant for a missing quote — a stablecoin's price is
//! the market's answer, and a wallet that cannot show a depeg is wrong exactly
//! when being right matters.
//!
//! Fiat rates follow the same shape: provider → `HashMap<currency, rate>`
//! where every rate is USD-relative (`USD == 1.0`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::http::{HttpClient, RetryProfile};

// ── Provider catalog

/// Market-data providers, in the order their answers are preferred.
///
/// This used to be a user setting with one arm selected and no fallback: a
/// provider that was down, rate limited, or simply did not list a coin yielded
/// no prices at all, and the only cure was a trip to Settings. All three run
/// now and their answers merge, so coverage is the union.
///
/// CoinGecko comes first because the catalog already carries a `coingecko_id`
/// for every asset, so it is the one provider that can be asked precisely
/// rather than by symbol.
const PRICE_PROVIDERS: &[PriceProvider] = &[
    PriceProvider::CoinGecko,
    PriceProvider::CoinPaprika,
    PriceProvider::CoinLore,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceProvider {
    CoinGecko,
    CoinPaprika,
    CoinLore,
}

impl PriceProvider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CoinGecko => "CoinGecko",
            Self::CoinPaprika => "CoinPaprika",
            Self::CoinLore => "CoinLore",
        }
    }
}

/// Fiat-rate providers, likewise in preference order.
const FIAT_RATE_PROVIDERS: &[FiatRateProvider] = &[
    FiatRateProvider::OpenER,
    FiatRateProvider::ExchangeRateHost,
    FiatRateProvider::Frankfurter,
    FiatRateProvider::FawazAhmed,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiatRateProvider {
    OpenER,
    ExchangeRateHost,
    Frankfurter,
    FawazAhmed,
}

impl FiatRateProvider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenER => "Open ER",
            Self::ExchangeRateHost => "ExchangeRate.host",
            Self::Frankfurter => "Frankfurter API",
            Self::FawazAhmed => "Fawaz Ahmed Currency API",
        }
    }
}

// ── Inputs / outputs

/// One coin the caller wants priced. `holding_key` is the Swift-side
/// identifier returned in the quote map, `symbol` is used as a provider
/// fallback, and `coin_gecko_id` is the canonical market-data id used by
/// id-indexed providers (CoinGecko, CoinPaprika, CoinLore).
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceRequestCoin {
    pub holding_key: String,
    pub symbol: String,
    pub coin_gecko_id: String,
}

/// Keyed by `holding_key`. Value is USD price.
pub type PriceQuoteMap = HashMap<String, f64>;

// ── Market-data endpoints (mirror ChainBackendRegistry)

const COINGECKO_SIMPLE_PRICE_URL: &str = "https://api.coingecko.com/api/v3/simple/price";
const COINPAPRIKA_TICKERS_URL: &str = "https://api.coinpaprika.com/v1/tickers";
const COINLORE_TICKERS_URL: &str = "https://api.coinlore.net/api/tickers/?start=0&limit=1000";

const OPEN_ER_LATEST_USD_URL: &str = "https://open.er-api.com/v6/latest/USD";
const FRANKFURTER_LATEST_URL: &str = "https://api.frankfurter.app/latest";
const EXCHANGE_RATE_HOST_LIVE_URL: &str = "https://api.exchangerate.host/live";
const FAWAZ_AHMED_USD_RATES_URL: &str =
    "https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/usd.json";

// ── Public entry points

/// Fetch USD prices for the supplied coins from the given provider.
///
/// Returns a map keyed by `holding_key` so the caller can diff against its
/// existing price cache. Missing coins are simply absent from the map —
/// callers should fall back to their last known price instead of erroring.
pub async fn fetch_prices(coins: &[PriceRequestCoin]) -> Result<PriceQuoteMap, String> {
    let answers = futures::future::join_all(PRICE_PROVIDERS.iter().map(|provider| async move {
        let result = match provider {
            PriceProvider::CoinGecko => fetch_coingecko_quotes(coins).await,
            PriceProvider::CoinPaprika => fetch_coinpaprika_quotes(coins).await,
            PriceProvider::CoinLore => fetch_coinlore_quotes(coins).await,
        };
        (*provider, result)
    }))
    .await;
    merge_in_preference_order(answers, "no price provider answered")
}

/// Fold provider answers into one map, keeping the first answer for each key in
/// provider order.
///
/// A provider that fails contributes nothing rather than failing the whole
/// fetch — that is the point of asking more than one. The error is reserved for
/// the case where every provider failed, which is a different thing from every
/// provider answering "I do not list that".
fn merge_in_preference_order<P: Copy + std::fmt::Debug, V>(
    answers: Vec<(P, Result<HashMap<String, V>, String>)>,
    nobody_answered: &str,
) -> Result<HashMap<String, V>, String> {
    let mut merged: HashMap<String, V> = HashMap::new();
    let mut failures: Vec<String> = Vec::new();
    for (provider, result) in answers {
        match result {
            Ok(values) => {
                for (key, value) in values {
                    merged.entry(key).or_insert(value);
                }
            }
            Err(e) => failures.push(format!("{provider:?}: {e}")),
        }
    }
    if merged.is_empty() && !failures.is_empty() {
        return Err(format!("{nobody_answered} ({})", failures.join("; ")));
    }
    Ok(merged)
}

/// Fetch USD-relative fiat rates for the requested non-USD currencies.
/// USD itself is always returned as `1.0`.
pub async fn fetch_fiat_rates(currencies: &[String]) -> Result<HashMap<String, f64>, String> {
    // Strip USD from the query list but always include it in the output.
    let targets: Vec<String> = currencies
        .iter()
        .filter(|c| c.to_uppercase() != "USD")
        .cloned()
        .collect();

    let targets = &targets;
    let answers =
        futures::future::join_all(FIAT_RATE_PROVIDERS.iter().map(|provider| async move {
            let result = match provider {
                FiatRateProvider::OpenER => fetch_open_er_rates(targets).await,
                FiatRateProvider::ExchangeRateHost => {
                    fetch_exchange_rate_host_rates(targets).await
                }
                FiatRateProvider::Frankfurter => fetch_frankfurter_rates(targets).await,
                FiatRateProvider::FawazAhmed => fetch_fawaz_ahmed_rates(targets).await,
            };
            (*provider, result)
        }))
        .await;
    let mut rates = merge_in_preference_order(answers, "no fiat-rate provider answered")?;
    rates.insert("USD".to_string(), 1.0);
    Ok(rates)
}

// ── CoinGecko

#[derive(Debug, Deserialize)]
struct CoinGeckoQuoteEntry {
    #[serde(default)]
    usd: Option<f64>,
}

/// CoinGecko response shape: `{"bitcoin": {"usd": 1234.5}, ...}`.
type CoinGeckoResponse = HashMap<String, CoinGeckoQuoteEntry>;

async fn fetch_coingecko_quotes(coins: &[PriceRequestCoin]) -> Result<PriceQuoteMap, String> {
    // Group by normalized gecko id; skip coins without one.
    let mut grouped: HashMap<String, Vec<&PriceRequestCoin>> = HashMap::new();
    for coin in coins {
        let id = coin.coin_gecko_id.trim().to_lowercase();
        if id.is_empty() {
            continue;
        }
        grouped.entry(id).or_default().push(coin);
    }
    if grouped.is_empty() {
        return Ok(PriceQuoteMap::new());
    }

    let mut ids: Vec<String> = grouped.keys().cloned().collect();
    ids.sort();
    let ids_csv = ids.join(",");

    let url = format!(
        "{COINGECKO_SIMPLE_PRICE_URL}?ids={ids}&vs_currencies=usd",
        ids = urlencoding_csv(&ids_csv),
    );
    let mut headers: HashMap<&str, &str> = HashMap::new();
    headers.insert("Accept", "application/json");

    let resp = HttpClient::shared()
        .get_json_with_headers::<CoinGeckoResponse>(&url, &headers, RetryProfile::ChainRead)
        .await
        .map_err(|e| format!("coingecko: {e}"))?;

    let mut resolved = PriceQuoteMap::new();
    for (id, entry) in resp {
        let Some(usd) = entry.usd else { continue };
        if usd <= 0.0 {
            continue;
        }
        if let Some(list) = grouped.get(&id.to_lowercase()) {
            for coin in list {
                resolved.insert(coin.holding_key.clone(), usd);
            }
        }
    }
    Ok(resolved)
}

/// URL-encode just the comma-separated id list (no full percent encoding
/// needed for alnum + `-`, which is the CoinGecko slug shape).
fn urlencoding_csv(csv: &str) -> String {
    csv.replace(' ', "%20")
}

// ── CoinPaprika

#[derive(Debug, Deserialize)]
struct PaprikaQuotes {
    #[serde(rename = "USD")]
    usd: Option<PaprikaUsd>,
}

#[derive(Debug, Deserialize)]
struct PaprikaUsd {
    #[serde(default)]
    price: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PaprikaTicker {
    id: String,
    symbol: String,
    #[serde(default)]
    quotes: Option<PaprikaQuotes>,
}

async fn fetch_coinpaprika_quotes(coins: &[PriceRequestCoin]) -> Result<PriceQuoteMap, String> {
    let mut resolved = PriceQuoteMap::new();

    let tickers: Vec<PaprikaTicker> = HttpClient::shared()
        .get_json(COINPAPRIKA_TICKERS_URL, RetryProfile::ChainRead)
        .await?;

    let by_id: HashMap<String, &PaprikaTicker> =
        tickers.iter().map(|t| (t.id.clone(), t)).collect();
    let mut by_symbol: HashMap<String, &PaprikaTicker> = HashMap::new();
    for t in &tickers {
        by_symbol.entry(t.symbol.to_uppercase()).or_insert(t);
    }

    for coin in coins {
        if resolved.contains_key(&coin.holding_key) {
            continue;
        }
        // Try the gecko-id → paprika-id lookup first, then fall back to
        // the symbol index.
        if let Some(id) = paprika_id_for(&coin.coin_gecko_id, &coin.symbol) {
            if let Some(ticker) = by_id.get(id) {
                if let Some(price) = ticker.quotes.as_ref().and_then(|q| q.usd.as_ref()?.price) {
                    if price > 0.0 {
                        resolved.insert(coin.holding_key.clone(), price);
                        continue;
                    }
                }
            }
        }
        let symbol = coin.symbol.trim().to_uppercase();
        if let Some(ticker) = by_symbol.get(&symbol) {
            if let Some(price) = ticker.quotes.as_ref().and_then(|q| q.usd.as_ref()?.price) {
                if price > 0.0 {
                    resolved.insert(coin.holding_key.clone(), price);
                }
            }
        }
    }

    Ok(resolved)
}

fn paprika_id_for(gecko_id: &str, symbol: &str) -> Option<&'static str> {
    static GECKO_MAP: std::sync::LazyLock<HashMap<&'static str, &'static str>> =
        std::sync::LazyLock::new(|| {
            HashMap::from([
                ("bitcoin", "btc-bitcoin"),
                ("ethereum", "eth-ethereum"),
                ("optimism", "op-optimism"),
                ("binancecoin", "bnb-binance-coin"),
                ("bitcoin-cash", "bch-bitcoin-cash"),
                ("bitcoin-cash-sv", "bsv-bitcoin-sv"),
                ("litecoin", "ltc-litecoin"),
                ("dogecoin", "doge-dogecoin"),
                ("cardano", "ada-cardano"),
                ("solana", "sol-solana"),
                ("tron", "trx-tron"),
                ("stellar", "xlm-stellar"),
                ("ripple", "xrp-xrp"),
                ("xrp", "xrp-xrp"),
                ("monero", "xmr-monero"),
                ("ethereum-classic", "etc-ethereum-classic"),
                ("sui", "sui-sui"),
                ("internet-computer", "icp-internet-computer"),
                ("near", "near-near-protocol"),
                ("polkadot", "dot-polkadot-token"),
                ("hyperliquid", "hype-hyperliquid"),
                ("tether", "usdt-tether"),
                ("usd-coin", "usdc-usd-coin"),
                ("dai", "dai-dai"),
                ("wrapped-bitcoin", "wbtc-wrapped-bitcoin"),
                ("chainlink", "link-chainlink"),
                ("uniswap", "uni-uniswap"),
                ("aave", "aave-aave"),
                ("shiba-inu", "shib-shiba-inu"),
                ("bitget-token", "bgb-bitget-token"),
                ("leo-token", "leo-unus-sed-leo"),
                ("cronos", "cro-cronos"),
                ("ethena-usde", "usde-ethena-usde"),
                ("ripple-usd", "rlusd-ripple-usd"),
                ("pax-gold", "paxg-pax-gold"),
                ("tether-gold", "xaut-tether-gold"),
                ("usdd", "usdd-usdd"),
                ("global-dollar", "usdg-global-dollar"),
            ])
        });
    static SYMBOL_MAP: std::sync::LazyLock<HashMap<&'static str, &'static str>> =
        std::sync::LazyLock::new(|| {
            HashMap::from([
                ("BTC", "btc-bitcoin"),
                ("ETH", "eth-ethereum"),
                ("OP", "op-optimism"),
                ("BNB", "bnb-binance-coin"),
                ("BCH", "bch-bitcoin-cash"),
                ("BSV", "bsv-bitcoin-sv"),
                ("LTC", "ltc-litecoin"),
                ("DOGE", "doge-dogecoin"),
                ("ADA", "ada-cardano"),
                ("SOL", "sol-solana"),
                ("TRX", "trx-tron"),
                ("XLM", "xlm-stellar"),
                ("XRP", "xrp-xrp"),
                ("XMR", "xmr-monero"),
                ("ETC", "etc-ethereum-classic"),
                ("SUI", "sui-sui"),
                ("ICP", "icp-internet-computer"),
                ("NEAR", "near-near-protocol"),
                ("DOT", "dot-polkadot-token"),
                ("HYPE", "hype-hyperliquid"),
                ("USDT", "usdt-tether"),
                ("USDC", "usdc-usd-coin"),
                ("DAI", "dai-dai"),
                ("BGB", "bgb-bitget-token"),
                ("LEO", "leo-unus-sed-leo"),
                ("CRO", "cro-cronos"),
                ("USDE", "usde-ethena-usde"),
                ("RLUSD", "rlusd-ripple-usd"),
                ("PAXG", "paxg-pax-gold"),
                ("XAUT", "xaut-tether-gold"),
                ("USDD", "usdd-usdd"),
                ("USDG", "usdg-global-dollar"),
            ])
        });

    let gecko = gecko_id.trim();
    // All keys are lowercase ASCII, so only lowercase if needed.
    if gecko.bytes().any(|b| b.is_ascii_uppercase()) {
        let lower = gecko.to_lowercase();
        if let Some(v) = GECKO_MAP.get(lower.as_str()) {
            return Some(v);
        }
    } else if let Some(v) = GECKO_MAP.get(gecko) {
        return Some(v);
    }

    let sym = symbol.trim();
    if sym.bytes().any(|b| b.is_ascii_lowercase()) {
        let upper = sym.to_uppercase();
        SYMBOL_MAP.get(upper.as_str()).copied()
    } else {
        SYMBOL_MAP.get(sym).copied()
    }
}

// ── CoinLore

#[derive(Debug, Deserialize)]
struct CoinLoreTicker {
    symbol: String,
    nameid: String,
    #[serde(rename = "price_usd")]
    price_usd: String,
}

#[derive(Debug, Deserialize)]
struct CoinLoreResponse {
    data: Vec<CoinLoreTicker>,
}

async fn fetch_coinlore_quotes(coins: &[PriceRequestCoin]) -> Result<PriceQuoteMap, String> {
    let mut resolved = PriceQuoteMap::new();

    let resp: CoinLoreResponse = HttpClient::shared()
        .get_json(COINLORE_TICKERS_URL, RetryProfile::ChainRead)
        .await?;

    let mut by_nameid: HashMap<String, &CoinLoreTicker> = HashMap::new();
    for t in &resp.data {
        by_nameid.entry(t.nameid.to_lowercase()).or_insert(t);
    }
    let mut by_symbol: HashMap<String, &CoinLoreTicker> = HashMap::new();
    for t in &resp.data {
        by_symbol.entry(t.symbol.to_uppercase()).or_insert(t);
    }

    for coin in coins {
        if resolved.contains_key(&coin.holding_key) {
            continue;
        }
        let gecko = coin.coin_gecko_id.trim().to_lowercase();
        let nameid = coinlore_nameid_for(&gecko);
        let ticker = by_nameid.get(nameid).copied().or_else(|| {
            let sym = coin.symbol.trim().to_uppercase();
            by_symbol.get(&sym).copied()
        });
        let Some(ticker) = ticker else { continue };
        let Ok(price) = ticker.price_usd.parse::<f64>() else {
            continue;
        };
        if price > 0.0 {
            resolved.insert(coin.holding_key.clone(), price);
        }
    }

    Ok(resolved)
}

fn coinlore_nameid_for(gecko_id: &str) -> &str {
    match gecko_id {
        "ripple" | "xrp" => "ripple",
        other => other,
    }
}

// ── Fiat rates

#[derive(Debug, Deserialize)]
struct OpenERResponse {
    rates: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
struct FrankfurterResponse {
    rates: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
struct ExchangeRateHostResponse {
    #[serde(default)]
    quotes: Option<HashMap<String, f64>>,
}

#[derive(Debug, Deserialize)]
struct FawazAhmedResponse {
    usd: HashMap<String, f64>,
}

async fn fetch_open_er_rates(currencies: &[String]) -> Result<HashMap<String, f64>, String> {
    let resp: OpenERResponse = HttpClient::shared()
        .get_json(OPEN_ER_LATEST_USD_URL, RetryProfile::ChainRead)
        .await?;
    Ok(filter_rates(resp.rates, currencies))
}

async fn fetch_frankfurter_rates(currencies: &[String]) -> Result<HashMap<String, f64>, String> {
    if currencies.is_empty() {
        return Ok(HashMap::new());
    }
    let to_csv = currencies.join(",");
    let url = format!("{FRANKFURTER_LATEST_URL}?from=USD&to={to_csv}");
    let resp: FrankfurterResponse = HttpClient::shared()
        .get_json(&url, RetryProfile::ChainRead)
        .await?;
    Ok(filter_rates(resp.rates, currencies))
}

async fn fetch_exchange_rate_host_rates(
    currencies: &[String],
) -> Result<HashMap<String, f64>, String> {
    if currencies.is_empty() {
        return Ok(HashMap::new());
    }
    let currencies_csv = currencies.join(",");
    let url = format!("{EXCHANGE_RATE_HOST_LIVE_URL}?source=USD&currencies={currencies_csv}");
    let resp: ExchangeRateHostResponse = HttpClient::shared()
        .get_json(&url, RetryProfile::ChainRead)
        .await?;
    let quotes = resp.quotes.unwrap_or_default();
    let mut out = HashMap::new();
    for currency in currencies {
        let key = format!("USD{currency}");
        if let Some(rate) = quotes.get(&key) {
            if *rate > 0.0 {
                out.insert(currency.clone(), *rate);
            }
        }
    }
    Ok(out)
}

async fn fetch_fawaz_ahmed_rates(currencies: &[String]) -> Result<HashMap<String, f64>, String> {
    let resp: FawazAhmedResponse = HttpClient::shared()
        .get_json(FAWAZ_AHMED_USD_RATES_URL, RetryProfile::ChainRead)
        .await?;
    // Fawaz uses lower-case currency keys — normalize upward.
    let normalized: HashMap<String, f64> = resp
        .usd
        .into_iter()
        .map(|(k, v)| (k.to_uppercase(), v))
        .collect();
    Ok(filter_rates(normalized, currencies))
}

fn filter_rates(rates: HashMap<String, f64>, allowed: &[String]) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for currency in allowed {
        let upper = currency.to_uppercase();
        if let Some(rate) = rates.get(&upper) {
            if *rate > 0.0 {
                out.insert(upper, *rate);
            }
        }
    }
    out
}

// ── Client-side merge policies (called from Swift after fetch)

#[derive(Debug, Clone, uniffi::Record)]
pub struct PriceMergeOutcome {
    pub updated_prices: HashMap<String, f64>,
    pub had_meaningful_change: bool,
}

pub fn merge_price_updates(
    existing: HashMap<String, f64>,
    fetched: HashMap<String, f64>,
) -> PriceMergeOutcome {
    let mut updated_prices = existing;
    let mut had_meaningful_change = false;
    for (key, value) in fetched {
        if updated_prices.get(&key).copied() != Some(value) {
            updated_prices.insert(key, value);
            had_meaningful_change = true;
        }
    }
    PriceMergeOutcome {
        updated_prices,
        had_meaningful_change,
    }
}

#[uniffi::export]
pub fn price_merge_live_updates(
    existing: HashMap<String, f64>,
    fetched: HashMap<String, f64>,
) -> PriceMergeOutcome {
    merge_price_updates(existing, fetched)
}

pub fn merge_fiat_rate_updates(
    fetched: HashMap<String, f64>,
    existing: HashMap<String, f64>,
    currencies: Vec<String>,
    base_currency: String,
) -> HashMap<String, f64> {
    let mut out: HashMap<String, f64> = HashMap::new();
    out.insert(base_currency.clone(), 1.0);
    for currency in currencies {
        if currency == base_currency {
            continue;
        }
        if let Some(&rate) = fetched.get(&currency) {
            if rate > 0.0 {
                out.insert(currency, rate);
                continue;
            }
        }
        if let Some(&rate) = existing.get(&currency) {
            if rate > 0.0 {
                out.insert(currency, rate);
            }
        }
    }
    out
}

#[uniffi::export]
pub fn price_merge_fiat_rate_updates(
    fetched: HashMap<String, f64>,
    existing: HashMap<String, f64>,
    currencies: Vec<String>,
    base_currency: String,
) -> HashMap<String, f64> {
    merge_fiat_rate_updates(fetched, existing, currencies, base_currency)
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    #[test]
    fn price_merge_detects_change_only_on_difference() {
        let existing = HashMap::from([("BTC".to_string(), 50000.0)]);
        let fetched = HashMap::from([("BTC".to_string(), 50000.0)]);
        let outcome = merge_price_updates(existing, fetched);
        assert!(!outcome.had_meaningful_change);

        let existing = HashMap::from([("BTC".to_string(), 50000.0)]);
        let fetched = HashMap::from([("BTC".to_string(), 51000.0)]);
        let outcome = merge_price_updates(existing, fetched);
        assert!(outcome.had_meaningful_change);
        assert_eq!(outcome.updated_prices.get("BTC"), Some(&51000.0));
    }

    #[test]
    fn price_merge_preserves_missing_keys() {
        let existing = HashMap::from([("BTC".to_string(), 50000.0), ("ETH".to_string(), 3000.0)]);
        let fetched = HashMap::from([("BTC".to_string(), 51000.0)]);
        let outcome = merge_price_updates(existing, fetched);
        assert_eq!(outcome.updated_prices.get("ETH"), Some(&3000.0));
    }

    #[test]
    fn fiat_merge_prefers_fetched_falls_back_to_existing() {
        let fetched = HashMap::from([("EUR".to_string(), 0.90)]);
        let existing = HashMap::from([("JPY".to_string(), 150.0), ("EUR".to_string(), 0.85)]);
        let currencies = vec!["USD".to_string(), "EUR".to_string(), "JPY".to_string()];
        let out = merge_fiat_rate_updates(fetched, existing, currencies, "USD".to_string());
        assert_eq!(out.get("USD"), Some(&1.0));
        assert_eq!(out.get("EUR"), Some(&0.90));
        assert_eq!(out.get("JPY"), Some(&150.0));
    }

    #[test]
    fn fiat_merge_drops_zero_rates() {
        let fetched = HashMap::from([("EUR".to_string(), 0.0)]);
        let existing: HashMap<String, f64> = HashMap::new();
        let out = merge_fiat_rate_updates(
            fetched,
            existing,
            vec!["USD".to_string(), "EUR".to_string()],
            "USD".to_string(),
        );
        assert_eq!(out.get("USD"), Some(&1.0));
        assert!(!out.contains_key("EUR"));
    }
}


#[cfg(test)]
mod merging_beats_choosing {
    use super::merge_in_preference_order;
    use std::collections::HashMap;

    fn ok(pairs: &[(&str, f64)]) -> Result<HashMap<String, f64>, String> {
        Ok(pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect())
    }

    /// Coverage is the union: a coin only one provider lists still gets a
    /// price. Under the old single-select this depended on which provider the
    /// user had picked in Settings.
    #[test]
    fn every_provider_contributes_what_only_it_has() {
        let merged = merge_in_preference_order(
            vec![
                ("first", ok(&[("btc", 1.0)])),
                ("second", ok(&[("obscure", 7.0)])),
                ("third", ok(&[("rarer", 9.0)])),
            ],
            "nobody",
        )
        .expect("some provider answered");
        assert_eq!(merged.len(), 3);
        assert_eq!(merged["obscure"], 7.0);
        assert_eq!(merged["rarer"], 9.0);
    }

    /// Where several answer for one coin, the earlier provider wins, so the
    /// answer does not drift between refreshes.
    #[test]
    fn the_first_provider_in_order_wins_a_contested_key() {
        let merged = merge_in_preference_order(
            vec![
                ("first", ok(&[("btc", 100.0)])),
                ("second", ok(&[("btc", 101.0)])),
            ],
            "nobody",
        )
        .expect("some provider answered");
        assert_eq!(merged["btc"], 100.0);
    }

    /// A provider that fails contributes nothing and does not fail the fetch —
    /// that is the whole point of asking more than one. Under the old code this
    /// was an `Err` all the way to the caller and no prices at all.
    #[test]
    fn one_failure_does_not_lose_the_others_answers() {
        let merged = merge_in_preference_order(
            vec![
                ("first", Err("429 rate limited".to_string())),
                ("second", ok(&[("btc", 100.0)])),
            ],
            "nobody",
        )
        .expect("the second provider answered");
        assert_eq!(merged["btc"], 100.0);
    }

    /// Only when every provider failed is it an error — which is a different
    /// thing from every provider answering "I do not list that".
    #[test]
    fn all_failing_is_an_error_but_all_answering_empty_is_not() {
        let err = merge_in_preference_order::<&str, f64>(
            vec![
                ("first", Err("down".to_string())),
                ("second", Err("timeout".to_string())),
            ],
            "no price provider answered",
        )
        .expect_err("nobody answered");
        assert!(err.contains("no price provider answered"));
        assert!(err.contains("down") && err.contains("timeout"));

        let empty = merge_in_preference_order(vec![("first", ok(&[]))], "nobody")
            .expect("answering with nothing is an answer");
        assert!(empty.is_empty());
    }
}

//! Price and fiat-rate fetching service.
//!
//! Ports the legacy Swift `LivePriceService` / `FiatRateService` (at
//! `Views/Models/PricingModels.swift`) into Rust so Swift can stop owning
//! the market-data surface. The Rust side handles every supported provider
//! end-to-end: build the URL, fetch with retry, decode the JSON, resolve
//! each requested coin to a USD price keyed by its `holding_key`, and
//! fold in stablecoin pinning where the provider doesn't list the token.
//!
//! Fiat rates follow the same shape: provider → `HashMap<currency, rate>`
//! where every rate is USD-relative (`USD == 1.0`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::http::{HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Provider catalog
// ----------------------------------------------------------------

/// Market-data providers Swift currently supports. Matches the Swift
/// `PricingProvider` enum raw values so existing settings round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceProvider {
    CoinGecko,
    Binance,
    CoinbaseExchange,
    CoinPaprika,
    CoinLore,
}

impl PriceProvider {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "CoinGecko" | "coingecko" => Some(Self::CoinGecko),
            "Binance" | "Binance Public API" | "binance" => Some(Self::Binance),
            "Coinbase Exchange API" | "coinbaseExchange" => Some(Self::CoinbaseExchange),
            "CoinPaprika" | "coinpaprika" => Some(Self::CoinPaprika),
            "CoinLore" | "coinlore" => Some(Self::CoinLore),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiatRateProvider {
    OpenER,
    ExchangeRateHost,
    Frankfurter,
    FawazAhmed,
}

impl FiatRateProvider {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "Open ER" | "openER" => Some(Self::OpenER),
            "ExchangeRate.host" | "exchangeRateHost" => Some(Self::ExchangeRateHost),
            "Frankfurter API" | "frankfurter" => Some(Self::Frankfurter),
            "Fawaz Ahmed Currency API" | "fawazAhmed" => Some(Self::FawazAhmed),
            _ => None,
        }
    }
}

// ----------------------------------------------------------------
// Inputs / outputs
// ----------------------------------------------------------------

/// One coin the caller wants priced. `holding_key` is the Swift-side
/// identifier returned in the quote map, `symbol` is the ticker used by
/// symbol-indexed providers (Binance, Coinbase), `coin_gecko_id` is used
/// by the id-indexed providers (CoinGecko, CoinPaprika, CoinLore).
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceRequestCoin {
    pub holding_key: String,
    pub symbol: String,
    pub coin_gecko_id: String,
}

/// Keyed by `holding_key`. Value is USD price.
pub type PriceQuoteMap = HashMap<String, f64>;

// ----------------------------------------------------------------
// Stablecoin pinning
// ----------------------------------------------------------------

const USD_STABLECOINS: &[&str] = &[
    "USDT", "USDC", "DAI", "FDUSD", "TUSD", "BUSD", "USDE", "PYUSD", "USDS", "USDD", "USDG",
    "USD1",
];

fn is_usd_stablecoin(symbol: &str) -> bool {
    let upper = symbol.trim().to_uppercase();
    USD_STABLECOINS.iter().any(|s| *s == upper)
}

fn stablecoin_quotes(coins: &[PriceRequestCoin]) -> PriceQuoteMap {
    let mut out = PriceQuoteMap::new();
    for c in coins {
        if is_usd_stablecoin(&c.symbol) {
            out.insert(c.holding_key.clone(), 1.0);
        }
    }
    out
}

// ----------------------------------------------------------------
// Market-data endpoints (mirror ChainBackendRegistry)
// ----------------------------------------------------------------

const COINGECKO_SIMPLE_PRICE_URL: &str = "https://api.coingecko.com/api/v3/simple/price";
const COINGECKO_PRO_SIMPLE_PRICE_URL: &str = "https://pro-api.coingecko.com/api/v3/simple/price";
const BINANCE_TICKER_PRICE_URL: &str = "https://api.binance.com/api/v3/ticker/price";
const COINBASE_EXCHANGE_RATES_URL: &str = "https://api.coinbase.com/v2/exchange-rates?currency=USD";
const COINPAPRIKA_TICKERS_URL: &str = "https://api.coinpaprika.com/v1/tickers";
const COINLORE_TICKERS_URL: &str = "https://api.coinlore.net/api/tickers/?start=0&limit=1000";

const OPEN_ER_LATEST_USD_URL: &str = "https://open.er-api.com/v6/latest/USD";
const FRANKFURTER_LATEST_URL: &str = "https://api.frankfurter.app/latest";
const EXCHANGE_RATE_HOST_LIVE_URL: &str = "https://api.exchangerate.host/live";
const FAWAZ_AHMED_USD_RATES_URL: &str =
    "https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/usd.json";

// ----------------------------------------------------------------
// Public entry points
// ----------------------------------------------------------------

/// Fetch USD prices for the supplied coins from the given provider.
///
/// Returns a map keyed by `holding_key` so the caller can diff against its
/// existing price cache. Missing coins are simply absent from the map —
/// callers should fall back to their last known price instead of erroring.
///
/// `api_key` is only consulted by the CoinGecko provider; pass an empty
/// string for the others.
pub async fn fetch_prices(
    provider: PriceProvider,
    coins: &[PriceRequestCoin],
    api_key: &str,
) -> Result<PriceQuoteMap, String> {
    match provider {
        PriceProvider::CoinGecko => fetch_coingecko_quotes(coins, api_key).await,
        PriceProvider::Binance => fetch_binance_quotes(coins).await,
        PriceProvider::CoinbaseExchange => fetch_coinbase_exchange_quotes(coins).await,
        PriceProvider::CoinPaprika => fetch_coinpaprika_quotes(coins).await,
        PriceProvider::CoinLore => fetch_coinlore_quotes(coins).await,
    }
}

/// Fetch USD-relative fiat rates for the requested non-USD currencies.
/// USD itself is always returned as `1.0`.
pub async fn fetch_fiat_rates(
    provider: FiatRateProvider,
    currencies: &[String],
) -> Result<HashMap<String, f64>, String> {
    // Strip USD from the query list but always include it in the output.
    let targets: Vec<String> = currencies
        .iter()
        .filter(|c| c.to_uppercase() != "USD")
        .cloned()
        .collect();

    let mut rates = match provider {
        FiatRateProvider::OpenER => fetch_open_er_rates(&targets).await?,
        FiatRateProvider::ExchangeRateHost => fetch_exchange_rate_host_rates(&targets).await?,
        FiatRateProvider::Frankfurter => fetch_frankfurter_rates(&targets).await?,
        FiatRateProvider::FawazAhmed => fetch_fawaz_ahmed_rates(&targets).await?,
    };
    rates.insert("USD".to_string(), 1.0);
    Ok(rates)
}

// ----------------------------------------------------------------
// CoinGecko
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CoinGeckoQuoteEntry {
    #[serde(default)]
    usd: Option<f64>,
}

/// CoinGecko response shape: `{"bitcoin": {"usd": 1234.5}, ...}`.
type CoinGeckoResponse = HashMap<String, CoinGeckoQuoteEntry>;

async fn fetch_coingecko_quotes(
    coins: &[PriceRequestCoin],
    api_key: &str,
) -> Result<PriceQuoteMap, String> {
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

    let trimmed_key = api_key.trim();

    // Attempt order mirrors the Swift implementation: pro first (if key
    // looks pro-style), then demo fallback.
    let attempts: Vec<(&str, Option<(&str, &str)>)> = if trimmed_key.is_empty() {
        vec![(COINGECKO_SIMPLE_PRICE_URL, None)]
    } else {
        vec![
            (
                COINGECKO_PRO_SIMPLE_PRICE_URL,
                Some(("x-cg-pro-api-key", "x_cg_pro_api_key")),
            ),
            (
                COINGECKO_SIMPLE_PRICE_URL,
                Some(("x-cg-demo-api-key", "x_cg_demo_api_key")),
            ),
        ]
    };

    let client = HttpClient::shared();
    let mut last_err = String::from("coingecko: no attempts");

    for (base_url, key_placement) in attempts {
        let mut url = format!(
            "{base_url}?ids={ids}&vs_currencies=usd",
            base_url = base_url,
            ids = urlencoding_csv(&ids_csv),
        );
        let mut headers: HashMap<&str, &str> = HashMap::new();
        if let Some((header_name, query_name)) = key_placement {
            if !trimmed_key.is_empty() {
                url.push_str(&format!("&{}={}", query_name, trimmed_key));
                headers.insert(header_name, trimmed_key);
            }
        }
        headers.insert("Accept", "application/json");

        match client
            .get_json_with_headers::<CoinGeckoResponse>(&url, &headers, RetryProfile::ChainRead)
            .await
        {
            Ok(resp) => {
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
                if !resolved.is_empty() {
                    return Ok(resolved);
                }
                last_err = "coingecko: empty response".to_string();
            }
            Err(e) => {
                last_err = format!("coingecko: {e}");
            }
        }
    }
    Err(last_err)
}

/// URL-encode just the comma-separated id list (no full percent encoding
/// needed for alnum + `-`, which is the CoinGecko slug shape).
fn urlencoding_csv(csv: &str) -> String {
    csv.replace(' ', "%20")
}

// ----------------------------------------------------------------
// Binance
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BinanceTicker {
    symbol: String,
    price: String,
}

async fn fetch_binance_quotes(coins: &[PriceRequestCoin]) -> Result<PriceQuoteMap, String> {
    let mut resolved = stablecoin_quotes(coins);

    let tickers: Vec<BinanceTicker> = HttpClient::shared()
        .get_json(BINANCE_TICKER_PRICE_URL, RetryProfile::ChainRead)
        .await?;

    let mut price_by_symbol: HashMap<String, f64> = HashMap::with_capacity(tickers.len());
    for t in tickers {
        if let Ok(v) = t.price.parse::<f64>() {
            if v > 0.0 {
                price_by_symbol.insert(t.symbol.to_uppercase(), v);
            }
        }
    }

    for coin in coins {
        if resolved.contains_key(&coin.holding_key) {
            continue;
        }
        let symbol = coin.symbol.trim().to_uppercase();
        for quote in &["USDT", "FDUSD", "USDC"] {
            let candidate = format!("{symbol}{quote}");
            if let Some(price) = price_by_symbol.get(&candidate) {
                resolved.insert(coin.holding_key.clone(), *price);
                break;
            }
        }
    }

    Ok(resolved)
}

// ----------------------------------------------------------------
// Coinbase Exchange
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CoinbasePayload {
    rates: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct CoinbaseEnvelope {
    data: CoinbasePayload,
}

async fn fetch_coinbase_exchange_quotes(
    coins: &[PriceRequestCoin],
) -> Result<PriceQuoteMap, String> {
    let mut resolved = stablecoin_quotes(coins);

    let env: CoinbaseEnvelope = HttpClient::shared()
        .get_json(COINBASE_EXCHANGE_RATES_URL, RetryProfile::ChainRead)
        .await?;

    for coin in coins {
        if resolved.contains_key(&coin.holding_key) {
            continue;
        }
        let symbol = coin.symbol.trim().to_uppercase();
        let Some(raw) = env.data.rates.get(&symbol) else {
            continue;
        };
        let Ok(rate) = raw.parse::<f64>() else {
            continue;
        };
        if rate > 0.0 {
            // Coinbase rates are quoted as "1 USD in target currency", so
            // USD→coin price is the inverse.
            resolved.insert(coin.holding_key.clone(), 1.0 / rate);
        }
    }

    Ok(resolved)
}

// ----------------------------------------------------------------
// CoinPaprika
// ----------------------------------------------------------------

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
    let mut resolved = stablecoin_quotes(coins);

    let tickers: Vec<PaprikaTicker> = HttpClient::shared()
        .get_json(COINPAPRIKA_TICKERS_URL, RetryProfile::ChainRead)
        .await?;

    let by_id: HashMap<String, &PaprikaTicker> =
        tickers.iter().map(|t| (t.id.clone(), t)).collect();
    let mut by_symbol: HashMap<String, &PaprikaTicker> = HashMap::new();
    for t in &tickers {
        by_symbol
            .entry(t.symbol.to_uppercase())
            .or_insert(t);
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
        std::sync::LazyLock::new(|| HashMap::from([
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
            ("pepe", "pepe-pepe"),
            ("bitget-token", "bgb-bitget-token"),
            ("leo-token", "leo-unus-sed-leo"),
            ("cronos", "cro-cronos"),
            ("ethena-usde", "usde-ethena-usde"),
            ("ripple-usd", "rlusd-ripple-usd"),
            ("pax-gold", "paxg-pax-gold"),
            ("tether-gold", "xaut-tether-gold"),
            ("usdd", "usdd-usdd"),
            ("global-dollar", "usdg-global-dollar"),
        ]));
    static SYMBOL_MAP: std::sync::LazyLock<HashMap<&'static str, &'static str>> =
        std::sync::LazyLock::new(|| HashMap::from([
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
        ]));

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

// ----------------------------------------------------------------
// CoinLore
// ----------------------------------------------------------------

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
    let mut resolved = stablecoin_quotes(coins);

    let resp: CoinLoreResponse = HttpClient::shared()
        .get_json(COINLORE_TICKERS_URL, RetryProfile::ChainRead)
        .await?;

    let mut by_nameid: HashMap<String, &CoinLoreTicker> = HashMap::new();
    for t in &resp.data {
        by_nameid
            .entry(t.nameid.to_lowercase())
            .or_insert(t);
    }
    let mut by_symbol: HashMap<String, &CoinLoreTicker> = HashMap::new();
    for t in &resp.data {
        by_symbol
            .entry(t.symbol.to_uppercase())
            .or_insert(t);
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

fn coinlore_nameid_for<'a>(gecko_id: &'a str) -> &'a str {
    match gecko_id {
        "ripple" | "xrp" => "ripple",
        other => other,
    }
}

// ----------------------------------------------------------------
// Fiat rates
// ----------------------------------------------------------------

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

fn filter_rates(
    rates: HashMap<String, f64>,
    allowed: &[String],
) -> HashMap<String, f64> {
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

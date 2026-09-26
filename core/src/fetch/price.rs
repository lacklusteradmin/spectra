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

use crate::fetch::http::{HttpClient, RetryProfile};

// ── Provider catalog

/// Market-data providers in preference order. Merge results for combined
/// coverage, querying by catalog provider ids only. Assets without a
/// provider id stay unpriced there; tickers do not establish identity.
const PRICE_PROVIDERS: &[PriceProvider] = &[PriceProvider::CoinGecko, PriceProvider::CoinPaprika];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceProvider {
    CoinGecko,
    CoinPaprika,
}

impl PriceProvider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CoinGecko => "CoinGecko",
            Self::CoinPaprika => "CoinPaprika",
        }
    }
}

/// Fiat-rate providers, likewise in preference order.
///
/// Two, where there were four. ExchangeRate.host moved behind an API key: its
/// `/live` endpoint answers `200` with `{"success":false,"error":{"code":101,
/// "type":"missing_access_key"}}`, and decoding `quotes` as an optional field
/// turned that into an empty success — an arm that quoted nothing and did not
/// even reach the failure list a total outage is reported from. Frankfurter
/// serves ECB reference rates, which do not list AED, so it could not cover
/// [`crate::store::state::FIAT_CURRENCY_CODES`] however healthy it was.
///
/// The two left both quote every code in that list, keyless, from independent
/// infrastructure — er-api's own API and a jsDelivr CDN — and agreed to within
/// 0.2% when this was cut. Merging here is not the union it is for spot
/// prices, where a second provider lists coins the first does not: every
/// provider quotes the same dozen currencies, so a third and fourth arm buy
/// availability alone, against a number that moves once a day, refreshes every
/// six hours, and falls back to the last good rate when no one answers.
const FIAT_RATE_PROVIDERS: &[FiatRateProvider] =
    &[FiatRateProvider::OpenER, FiatRateProvider::FawazAhmed];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiatRateProvider {
    OpenER,
    FawazAhmed,
}

impl FiatRateProvider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenER => "Open ER",
            Self::FawazAhmed => "Fawaz Ahmed Currency API",
        }
    }
}

// ── Inputs / outputs

/// One coin the caller wants priced. `holding_key` is the caller's own
/// identifier, returned in the quote map. Each provider has its own explicit id.
///
/// It also carried the ticker symbol, for providers to match on when the id
/// missed. Nothing matches on symbol any more, so a front end no longer sends
/// one — see [`PRICE_PROVIDERS`].
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceRequestCoin {
    pub holding_key: String,
    pub coingecko_id: String,
    pub coinpaprika_id: String,
}

/// Keyed by `holding_key`. Value is USD price.
pub type PriceQuoteMap = HashMap<String, f64>;

// ── Market-data endpoints (mirror ChainBackendRegistry)

const COINGECKO_SIMPLE_PRICE_URL: &str = "https://api.coingecko.com/api/v3/simple/price";
const COINPAPRIKA_TICKERS_URL: &str = "https://api.coinpaprika.com/v1/tickers";

const OPEN_ER_LATEST_USD_URL: &str = "https://open.er-api.com/v6/latest/USD";
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
        let id = coin.coingecko_id.trim().to_lowercase();
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
    #[serde(default)]
    quotes: Option<PaprikaQuotes>,
}

async fn fetch_coinpaprika_quotes(coins: &[PriceRequestCoin]) -> Result<PriceQuoteMap, String> {
    let resolved = PriceQuoteMap::new();

    if coins
        .iter()
        .all(|coin| coin.coinpaprika_id.trim().is_empty())
    {
        return Ok(resolved);
    }
    let tickers: Vec<PaprikaTicker> = HttpClient::shared()
        .get_json(COINPAPRIKA_TICKERS_URL, RetryProfile::ChainRead)
        .await?;

    Ok(resolve_paprika_quotes(coins, &tickers))
}

fn resolve_paprika_quotes(coins: &[PriceRequestCoin], tickers: &[PaprikaTicker]) -> PriceQuoteMap {
    let mut resolved = PriceQuoteMap::new();
    let by_id: HashMap<&str, &PaprikaTicker> = tickers.iter().map(|t| (t.id.as_str(), t)).collect();

    for coin in coins {
        if resolved.contains_key(&coin.holding_key) {
            continue;
        }
        let id = coin.coinpaprika_id.trim();
        let Some(ticker) = by_id.get(id) else {
            continue;
        };
        if let Some(price) = ticker.quotes.as_ref().and_then(|q| q.usd.as_ref()?.price)
            && price > 0.0
        {
            resolved.insert(coin.holding_key.clone(), price);
        }
    }

    resolved
}

// ── Fiat rates

#[derive(Debug, Deserialize)]
struct OpenERResponse {
    rates: HashMap<String, f64>,
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
        if let Some(rate) = rates.get(&upper)
            && *rate > 0.0
        {
            out.insert(upper, *rate);
        }
    }
    out
}

// ── Client-side merge policy
//
// Spot prices merge in `service::network_prices::apply_price_result`, where
// the fetch result and the last good quote are both in hand. Fiat rates keep
// a function of their own: the set of currencies to carry forward is an
// argument, not whatever the previous map happened to hold.

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
        if let Some(&rate) = fetched.get(&currency)
            && rate > 0.0
        {
            out.insert(currency, rate);
            continue;
        }
        if let Some(&rate) = existing.get(&currency)
            && rate > 0.0
        {
            out.insert(currency, rate);
        }
    }
    out
}

#[cfg(test)]
mod merge_tests {
    use super::*;

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

#[cfg(test)]
mod explicit_provider_tests {
    use super::*;
    #[test]
    fn paprika_uses_its_own_id_even_without_gecko_or_with_a_conflicting_gecko_id() {
        let tickers: Vec<PaprikaTicker> = serde_json::from_str(
            r#"[
            {"id":"custom-coin","quotes":{"USD":{"price":2.5}}},
            {"id":"btc-bitcoin","quotes":{"USD":{"price":90000}}}
        ]"#,
        )
        .unwrap();
        let coins = ["", "bitcoin"]
            .into_iter()
            .enumerate()
            .map(|(i, gecko)| PriceRequestCoin {
                holding_key: i.to_string(),
                coingecko_id: gecko.into(),
                coinpaprika_id: "custom-coin".into(),
            })
            .chain(std::iter::once(PriceRequestCoin {
                holding_key: "no-paprika".into(),
                coingecko_id: "bitcoin".into(),
                coinpaprika_id: String::new(),
            }))
            .collect::<Vec<_>>();
        let prices = resolve_paprika_quotes(&coins, &tickers);
        assert_eq!(prices.get("0"), Some(&2.5));
        assert_eq!(prices.get("1"), Some(&2.5));
        assert!(!prices.contains_key("no-paprika"));
    }
}

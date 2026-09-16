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
/// no prices at all, and the only cure was a trip to Settings. Both run now
/// and their answers merge, so coverage is the union.
///
/// CoinGecko comes first because it prices every asset the catalog carries an
/// id for; CoinPaprika is asked about whatever it also lists. Both are asked
/// by id. Matching a quote to a holding by ticker symbol was how BUSD — Bera
/// USD here — priced as Binance USD, so nothing does it now: an asset the
/// catalog cannot name at a provider goes unpriced there.
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
/// identifier, returned in the quote map; `coin_gecko_id` is the catalog id
/// every provider is resolved from.
///
/// It also carried the ticker symbol, for providers to match on when the id
/// missed. Nothing matches on symbol any more, so a front end no longer sends
/// one — see [`PRICE_PROVIDERS`].
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceRequestCoin {
    pub holding_key: String,
    pub coin_gecko_id: String,
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
    #[serde(default)]
    quotes: Option<PaprikaQuotes>,
}

async fn fetch_coinpaprika_quotes(coins: &[PriceRequestCoin]) -> Result<PriceQuoteMap, String> {
    let mut resolved = PriceQuoteMap::new();

    let tickers: Vec<PaprikaTicker> = HttpClient::shared()
        .get_json(COINPAPRIKA_TICKERS_URL, RetryProfile::ChainRead)
        .await?;

    let by_id: HashMap<&str, &PaprikaTicker> = tickers.iter().map(|t| (t.id.as_str(), t)).collect();

    for coin in coins {
        if resolved.contains_key(&coin.holding_key) {
            continue;
        }
        let Some(id) = paprika_id_for(&coin.coin_gecko_id) else {
            continue;
        };
        let Some(ticker) = by_id.get(id) else {
            continue;
        };
        if let Some(price) = ticker.quotes.as_ref().and_then(|q| q.usd.as_ref()?.price) {
            if price > 0.0 {
                resolved.insert(coin.holding_key.clone(), price);
            }
        }
    }

    Ok(resolved)
}

/// One asset's ids at the market-data providers, as the catalogs state them.
///
/// Kept off `ChainEntry` and `TokenEntry`: no front end prices anything, so a
/// column there would only be bytes crossing the FFI on every catalog call.
#[derive(Debug, Clone)]
pub(crate) struct AssetMarketIds {
    pub coingecko_id: String,
    /// CoinPaprika's own slug, or empty where it lists nothing the catalog
    /// could identify. Not derivable: Aave is `aave-new` and Cronos is
    /// `cro-cryptocom-chain`.
    pub coinpaprika_id: String,
}

/// The catalogs' market-data ids, indexed by CoinGecko id.
///
/// That is the key because a price request identifies its asset by that id and
/// nothing else. Chains and tokens that share one agree on the rest of the
/// row, which `market_ids_agree_on_shared_gecko_ids` holds them to.
fn market_ids_for(gecko_id: &str) -> Option<&'static AssetMarketIds> {
    static BY_GECKO_ID: std::sync::LazyLock<HashMap<&'static str, &'static AssetMarketIds>> =
        std::sync::LazyLock::new(|| {
            crate::tokens::market_ids()
                .iter()
                .filter(|m| !m.coingecko_id.is_empty())
                .map(|m| (m.coingecko_id.as_str(), m))
                .collect()
        });

    let gecko = gecko_id.trim();
    if gecko.is_empty() {
        return None;
    }
    // Catalog ids are lowercase, so only lowercase when the caller's is not.
    if gecko.bytes().any(|b| b.is_ascii_uppercase()) {
        BY_GECKO_ID.get(gecko.to_lowercase().as_str()).copied()
    } else {
        BY_GECKO_ID.get(gecko).copied()
    }
}

/// The CoinPaprika id for an asset, or `None` when the catalogs do not name
/// one — an unlisted asset, or one whose identity there we could not verify.
///
/// This was a pair of hand-written tables covering a third of the token
/// catalog, three of whose ids no longer resolved (`aave-aave`, `cro-cronos`,
/// `leo-unus-sed-leo`). Whatever they missed fell through to a symbol match
/// against several thousand paprika listings, which is a wrong asset's price,
/// not a missing one.
fn paprika_id_for(gecko_id: &str) -> Option<&'static str> {
    let ids = market_ids_for(gecko_id)?;
    (!ids.coinpaprika_id.is_empty()).then_some(ids.coinpaprika_id.as_str())
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
        if let Some(rate) = rates.get(&upper) {
            if *rate > 0.0 {
                out.insert(upper, *rate);
            }
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
mod market_id_tests {
    use super::*;
    use std::collections::HashSet;

    fn all_rows() -> Vec<&'static AssetMarketIds> {
        crate::tokens::market_ids().iter().collect()
    }

    /// Ten chains share ETH and the CRO token shares Cronos's coin, so one
    /// CoinGecko id appears on several catalog rows. They are one asset, so
    /// they must name one listing — `market_ids_for` keys on the gecko id and
    /// would otherwise answer whichever row it indexed last.
    #[test]
    fn market_ids_agree_on_shared_gecko_ids() {
        let mut seen: HashMap<&str, &AssetMarketIds> = HashMap::new();
        for row in all_rows() {
            if let Some(first) = seen.insert(row.coingecko_id.as_str(), row) {
                assert_eq!(
                    first.coinpaprika_id, row.coinpaprika_id,
                    "catalog rows for {} disagree about where it is listed",
                    row.coingecko_id
                );
            }
        }
    }

    /// The other direction: two different assets pinned to one listing means
    /// one of them is priced as the other. XAUT and XAUT0 are separate
    /// listings, and a copied line is how they would stop being.
    #[test]
    fn no_two_assets_claim_one_listing() {
        let mut owner: HashMap<&str, &str> = HashMap::new();
        for row in all_rows() {
            let id = row.coinpaprika_id.as_str();
            if id.is_empty() {
                continue;
            }
            let claimant = owner.entry(id).or_insert(&row.coingecko_id);
            assert_eq!(
                *claimant, row.coingecko_id,
                "coinpaprika listing {id} is claimed by both {claimant} and {}",
                row.coingecko_id
            );
        }
    }

    /// Ids go to the provider as written, and both index theirs in lowercase.
    #[test]
    fn catalog_ids_are_lowercase_and_trimmed() {
        for row in all_rows() {
            for id in [&row.coingecko_id, &row.coinpaprika_id] {
                assert_eq!(id.trim().to_lowercase(), *id, "{id} is not a plain id");
            }
        }
    }

    /// A token added without deciding where it is listed used to price as
    /// whatever else shared its ticker. Blank is now a decision, and this is
    /// the list of assets it has been made for.
    #[test]
    fn only_deliberately_unlisted_assets_have_no_paprika_id() {
        let blank: HashSet<&str> = all_rows()
            .iter()
            .filter(|r| r.coinpaprika_id.is_empty())
            .map(|r| r.coingecko_id.as_str())
            .collect();
        // honey-3 is Bera USD. CoinPaprika's BUSD is Binance USD, a different
        // token, and it has no listing for ours — so it prices this nowhere
        // rather than pricing it as something else.
        assert_eq!(blank, HashSet::from(["honey-3"]));
    }

    /// The ids the old hand-written table got wrong (`aave-aave`,
    /// `cro-cronos`, `leo-unus-sed-leo` resolve to nothing at CoinPaprika) and
    /// the ones no table would have guessed.
    #[test]
    fn paprika_ids_come_from_the_catalog() {
        assert_eq!(paprika_id_for("aave"), Some("aave-new"));
        assert_eq!(
            paprika_id_for("crypto-com-chain"),
            Some("cro-cryptocom-chain")
        );
        assert_eq!(paprika_id_for("leo-token"), Some("leo-leo-token"));
        assert_eq!(paprika_id_for("bittorrent"), Some("bttc-bittorrent-chain"));
        assert_eq!(paprika_id_for("usa"), Some("usat"));
        // A caller's id is not required to be normalized; a catalog's is.
        assert_eq!(paprika_id_for("  Bitcoin "), Some("btc-bitcoin"));
        // Unlisted, unknown and absent all mean the same thing: ask nobody.
        assert_eq!(paprika_id_for("honey-3"), None);
        assert_eq!(paprika_id_for("not-a-coin"), None);
        assert_eq!(paprika_id_for(""), None);
    }
}

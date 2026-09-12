//! Network prices: service adapters and dispatch.
use super::*;
use crate::store::state::StateEvent;

#[derive(
    Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Record,
)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRefreshState {
    pub prices: HashMap<String, f64>,
    pub prices_attempt_at: Option<f64>,
    pub prices_success_at: Option<f64>,
    pub prices_error: Option<String>,
    pub fiat_attempt_at: Option<f64>,
    pub fiat_success_at: Option<f64>,
    pub fiat_error: Option<String>,
}
fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn due(
    force: bool,
    now: f64,
    attempted: Option<f64>,
    succeeded: Option<f64>,
    interval: f64,
) -> bool {
    force
        || (attempted.is_none_or(|t| now - t >= 60.0)
            && succeeded.is_none_or(|t| attempted.is_some_and(|a| a > t) || now - t >= interval))
}

fn apply_price_result(
    quotes: &mut QuoteRefreshState,
    time: f64,
    result: Result<HashMap<String, f64>, String>,
) {
    quotes.prices_attempt_at = Some(time);
    match result {
        Ok(fetched) => {
            let valid: HashMap<_, _> = fetched
                .into_iter()
                .filter(|(_, p)| p.is_finite() && *p > 0.0)
                .collect();
            if valid.is_empty() {
                quotes.prices_error = Some("No price provider had a valid quote".into());
            } else {
                quotes.prices.extend(valid);
                quotes.prices_success_at = Some(time);
                quotes.prices_error = None;
            }
        }
        Err(error) => quotes.prices_error = Some(error.to_string()),
    }
}

impl WalletService {
    /// Fetch USD spot prices for the supplied coins from `provider`.
    ///
    /// `provider` is the Swift-side display name (e.g. "CoinGecko").
    /// `coins` are the known tokens. All providers use their public
    /// endpoints — no API key plumbing.
    pub async fn fetch_prices_typed(
        &self,
        coins: Vec<crate::price::PriceRequestCoin>,
    ) -> Result<std::collections::HashMap<String, f64>, SpectraBridgeError> {
        tracing::debug!(coins = coins.len(), "fetch_prices enter");
        match crate::price::fetch_prices(&coins).await {
            Ok(quotes) => {
                tracing::debug!(returned = quotes.len(), "fetch_prices ok");
                Ok(quotes)
            }
            Err(e) => {
                tracing::error!(error = %e, "fetch_prices failed");
                Err(SpectraBridgeError::from(e))
            }
        }
    }

    /// Typed variant — accepts typed currency list and returns typed map directly.
    /// Fetch the display-currency cross rates and store them.
    ///
    /// The rates are core's state: every quoted amount passes through them and
    /// they are what the app shows while a refresh is in flight. iOS fetched
    /// them, merged them with `price_merge_fiat_rate_updates`, and wrote the
    /// result to its own SQLite blob — so the CLI could neither read nor
    /// refresh them, and a launch could seed the merge from an older
    /// `UserDefaults` copy that still won the race. The fetch, the merge and
    /// the write are one operation here; a provider failure leaves the stored
    /// rates alone and says so.
    pub async fn refresh_fiat_rates(
        &self,
    ) -> Result<std::collections::HashMap<String, f64>, SpectraBridgeError> {
        let state = self.refresh_owned_fiat_rates(true).await?;
        if let Some(error) = state.quotes.fiat_error {
            return Err(error.into());
        }
        Ok(state.fiat_rates_from_usd)
    }

    pub async fn fetch_fiat_rates_typed(
        &self,
        currencies: Vec<String>,
    ) -> Result<std::collections::HashMap<String, f64>, SpectraBridgeError> {
        tracing::debug!(currencies = currencies.len(), "fetch_fiat_rates enter");
        match crate::price::fetch_fiat_rates(&currencies).await {
            Ok(rates) => {
                tracing::debug!(returned = rates.len(), "fetch_fiat_rates ok");
                Ok(rates)
            }
            Err(e) => {
                tracing::error!(error = %e, "fetch_fiat_rates failed");
                Err(SpectraBridgeError::from(e))
            }
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Core chooses assets from stored holdings and dashboard pins, then persists valid quotes.
    pub async fn refresh_owned_prices(
        &self,
        force: bool,
    ) -> Result<CoreAppState, SpectraBridgeError> {
        let _guard = self.quote_refresh_lock.lock().await;
        let state = self.app_state().await;
        let time = now();
        if !due(
            force,
            time,
            state.quotes.prices_attempt_at,
            state.quotes.prices_success_at,
            60.0,
        ) {
            return Ok(state);
        }
        let derived = self.wallet_derived_state().await?;
        let mut coins = derived.unique_price_request_coins.clone();
        for symbol in state.settings.pinned_dashboard_assets() {
            if let Some(coin) = self.pinned_prototype(&symbol, &derived).await {
                coins.push(coin);
            }
        }
        let mut requests = HashMap::new();
        for coin in coins {
            let Some(chain) = Chain::from_display_name(&coin.chain_name) else {
                continue;
            };
            let network = state.settings.network_chain(chain);
            if network.is_testnet() || coin.coin_gecko_id.trim().is_empty() {
                continue;
            }
            let key = format!("{}|{}", network.chain_display_name(), coin.symbol);
            requests.insert(
                key.clone(),
                crate::price::PriceRequestCoin {
                    holding_key: key,
                    coin_gecko_id: coin.coin_gecko_id,
                },
            );
        }
        if requests.is_empty() {
            return Ok(state);
        }
        let result = crate::price::fetch_prices(&requests.into_values().collect::<Vec<_>>()).await;
        let transition = self
            .mutate_persisted_state(move |state| {
                apply_price_result(&mut state.quotes, time, result);
                vec![StateEvent {
                    kind: "quotesUpdated".into(),
                    subject_id: None,
                }]
            })
            .await?;
        Ok(transition.state)
    }

    pub async fn refresh_owned_fiat_rates(
        &self,
        force: bool,
    ) -> Result<CoreAppState, SpectraBridgeError> {
        let _guard = self.quote_refresh_lock.lock().await;
        let state = self.app_state().await;
        let time = now();
        if !force
            && (state.settings.fiat_currency_code == "USD"
                || !due(
                    false,
                    time,
                    state.quotes.fiat_attempt_at,
                    state.quotes.fiat_success_at,
                    21600.0,
                ))
        {
            return Ok(state);
        }
        let codes = crate::store::state::fiat_currency_codes();
        let result = crate::price::fetch_fiat_rates(&codes).await;
        let transition = self
            .mutate_persisted_state(move |state| {
                state.quotes.fiat_attempt_at = Some(time);
                match result {
                    Ok(fetched) if fetched.values().any(|p| p.is_finite() && *p > 0.0) => {
                        let fetched = fetched
                            .into_iter()
                            .filter(|(_, p)| p.is_finite() && *p > 0.0)
                            .collect();
                        state.fiat_rates_from_usd = crate::price::merge_fiat_rate_updates(
                            fetched,
                            state.fiat_rates_from_usd.clone(),
                            codes,
                            "USD".into(),
                        );
                        state.quotes.fiat_success_at = Some(time);
                        state.quotes.fiat_error = None;
                    }
                    _ => {
                        state.quotes.fiat_error =
                            Some("No fiat-rate provider answered; using stored rates".into())
                    }
                }
                vec![StateEvent {
                    kind: "quotesUpdated".into(),
                    subject_id: None,
                }]
            })
            .await?;
        Ok(transition.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_policy_bounds_retries_and_honors_force() {
        assert!(!due(false, 130.0, Some(100.0), None, 21600.0));
        assert!(due(false, 160.0, Some(100.0), None, 21600.0));
        assert!(!due(false, 200.0, Some(100.0), Some(100.0), 21600.0));
        assert!(due(true, 101.0, Some(100.0), Some(100.0), 21600.0));
        assert!(!due(false, 50.0, Some(100.0), None, 60.0));
    }
    #[test]
    fn failed_or_invalid_prices_preserve_the_last_quote() {
        let mut quotes = QuoteRefreshState::default();
        apply_price_result(
            &mut quotes,
            100.0,
            Ok(HashMap::from([("ETH".into(), 12.0)])),
        );
        apply_price_result(&mut quotes, 120.0, Err("offline".into()));
        assert_eq!(quotes.prices["ETH"], 12.0);
        assert!(quotes.prices_error.is_some());
        apply_price_result(
            &mut quotes,
            180.0,
            Ok(HashMap::from([
                ("ETH".into(), f64::NAN),
                ("BTC".into(), -1.0),
            ])),
        );
        assert_eq!(quotes.prices["ETH"], 12.0);
        assert!(!quotes.prices.contains_key("BTC"));
        assert_eq!(quotes.prices_success_at, Some(100.0));
        assert!(due(
            false,
            240.0,
            quotes.prices_attempt_at,
            quotes.prices_success_at,
            21600.0
        ));
        apply_price_result(
            &mut quotes,
            240.0,
            Ok(HashMap::from([("BTC".into(), 20.0)])),
        );
        assert_eq!(quotes.prices["ETH"], 12.0);
        assert!(quotes.prices_error.is_none());
    }
    #[tokio::test]
    async fn quote_state_survives_restart_and_no_work_needs_no_network() {
        let path = std::env::temp_dir().join(format!(
            "spectra-quotes-{}.db",
            crate::store::new_transaction_id()
        ));
        let service = WalletService::new_typed(vec![]).unwrap();
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        service
            .mutate_persisted_state(|s| {
                s.quotes.prices.insert("Ethereum|ETH".into(), 12.0);
                s.quotes.prices_attempt_at = Some(now());
                s.quotes.prices_error = Some("provider unavailable".into());
                vec![StateEvent {
                    kind: "quotesUpdated".into(),
                    subject_id: None,
                }]
            })
            .await
            .unwrap();
        let reopened = WalletService::new_typed(vec![]).unwrap();
        let state = reopened
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        assert_eq!(state.quotes.prices["Ethereum|ETH"], 12.0);
        assert_eq!(
            reopened.refresh_owned_prices(false).await.unwrap().quotes,
            state.quotes
        );
        assert_eq!(
            reopened
                .refresh_owned_fiat_rates(false)
                .await
                .unwrap()
                .quotes,
            state.quotes
        );
    }
}

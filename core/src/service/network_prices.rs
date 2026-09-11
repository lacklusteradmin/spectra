//! Network prices: service adapters and dispatch.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
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
        let codes = crate::store::state::fiat_currency_codes();
        let existing = self.app_state().await.fiat_rates_from_usd;
        let fetched = crate::price::fetch_fiat_rates(&codes)
            .await
            .map_err(SpectraBridgeError::from)?;
        let merged = crate::price::merge_fiat_rate_updates(
            fetched,
            existing.clone(),
            codes,
            crate::store::state::FIAT_BASE_CURRENCY.to_string(),
        );
        if merged == existing {
            return Ok(merged);
        }
        let stored = merged.clone();
        self.store_fiat_rates(stored).await?;
        Ok(merged)
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

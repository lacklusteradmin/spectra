//! WalletService — live price + fiat-rate fetch.
//!
//! Sliced out of `service/mod.rs`. The `WalletService` type itself stays in
//! `mod.rs`; methods here live in a separate `impl` block (Rust permits
//! multiple impl blocks per type, and UniFFI exports them as if they were one).

#![allow(unused_imports)]

use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch USD spot prices for the supplied coins from `provider`.
    ///
    /// `provider` is the Swift-side display name (e.g. "CoinGecko").
    /// `coins` are the tracked tokens. All providers use their public
    /// endpoints — no API key plumbing.
    pub async fn fetch_prices_typed(
        &self,
        provider: String,
        coins: Vec<crate::price::PriceRequestCoin>,
    ) -> Result<std::collections::HashMap<String, f64>, SpectraBridgeError> {
        eprintln!("[spectra:prices] enter provider={provider} coins={}", coins.len());
        let parsed_provider = match crate::price::PriceProvider::from_raw(&provider) {
            Some(p) => p,
            None => {
                eprintln!("[spectra:prices] UNKNOWN provider={provider}");
                return Err(format!("unknown price provider: {provider}").into());
            }
        };
        match crate::price::fetch_prices(parsed_provider, &coins).await {
            Ok(quotes) => {
                eprintln!("[spectra:prices] ok provider={provider} returned={}", quotes.len());
                Ok(quotes)
            }
            Err(e) => {
                eprintln!("[spectra:prices] FAIL provider={provider}: {e}");
                Err(SpectraBridgeError::from(e))
            }
        }
    }

    /// Typed variant — accepts typed currency list and returns typed map directly.
    pub async fn fetch_fiat_rates_typed(
        &self,
        provider: String,
        currencies: Vec<String>,
    ) -> Result<std::collections::HashMap<String, f64>, SpectraBridgeError> {
        eprintln!("[spectra:fiat] enter provider={provider} currencies={}", currencies.len());
        let parsed_provider = match crate::price::FiatRateProvider::from_raw(&provider) {
            Some(p) => p,
            None => {
                eprintln!("[spectra:fiat] UNKNOWN provider={provider}");
                return Err(format!("unknown fiat rate provider: {provider}").into());
            }
        };
        match crate::price::fetch_fiat_rates(parsed_provider, &currencies).await {
            Ok(rates) => {
                eprintln!("[spectra:fiat] ok provider={provider} returned={}", rates.len());
                Ok(rates)
            }
            Err(e) => {
                eprintln!("[spectra:fiat] FAIL provider={provider}: {e}");
                Err(SpectraBridgeError::from(e))
            }
        }
    }
}

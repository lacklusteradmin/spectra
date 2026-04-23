//! Stellar chain client.
//!
//! Uses the Horizon REST API for account info, history, fee stats,
//! and transaction submission. Signs with Ed25519 using ed25519-dalek.
//! XDR encoding is done manually (minimal subset for Payment operation).

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::{build_signed_payment_xdr, build_signed_payment_xdr_with_asset, StellarAsset};

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarBalance {
    /// Stroops (1 XLM = 10_000_000 stroops).
    pub stroops: i64,
    pub xlm_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarAssetBalance {
    pub asset_code: String,
    pub asset_issuer: String,
    /// Fixed 7-decimal stroop units (same precision as XLM).
    pub amount_stroops: i64,
    pub amount_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarHistoryEntry {
    pub txid: String,
    pub ledger: u64,
    pub timestamp: String,
    pub from: String,
    pub to: String,
    pub amount_stroops: i64,
    pub fee_charged: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarSendResult {
    pub txid: String,
    /// Base64-encoded signed XDR envelope — stored for rebroadcast.
    pub signed_xdr_b64: String,
}

// ----------------------------------------------------------------
// Horizon API response types
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct HorizonAccount {
    pub(super) balances: Vec<HorizonBalance>,
    pub(super) sequence: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct HorizonBalance {
    pub(super) balance: String,
    pub(super) asset_type: String,
    #[serde(default)]
    pub(super) asset_code: String,
    #[serde(default)]
    pub(super) asset_issuer: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct HorizonFeeStats {
    pub(super) fee_charged: HorizonFeeCharged,
}

#[derive(Debug, Deserialize)]
pub(super) struct HorizonFeeCharged {
    pub(super) mode: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct HorizonPayments {
    #[serde(rename = "_embedded")]
    pub(super) embedded: HorizonPaymentsEmbedded,
}

#[derive(Debug, Deserialize)]
pub(super) struct HorizonPaymentsEmbedded {
    pub(super) records: Vec<HorizonPaymentRecord>,
}

#[derive(Debug, Deserialize)]
pub(super) struct HorizonPaymentRecord {
    #[allow(dead_code)]
    pub(super) id: String,
    #[serde(rename = "type")]
    pub(super) op_type: String,
    #[serde(default)]
    pub(super) from: String,
    #[serde(default)]
    pub(super) to: String,
    #[serde(default)]
    pub(super) amount: String,
    pub(super) created_at: String,
    pub(super) transaction_hash: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct StellarClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl StellarClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}

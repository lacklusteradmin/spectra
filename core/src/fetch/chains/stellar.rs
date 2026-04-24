//! Stellar chain client.
//!
//! Uses the Horizon REST API for account info, history, fee stats,
//! and transaction submission. Signs with Ed25519 using ed25519-dalek.
//! XDR encoding is done manually (minimal subset for Payment operation).

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};



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
pub(crate) struct HorizonAccount {
    pub(crate) balances: Vec<HorizonBalance>,
    pub(crate) sequence: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonBalance {
    pub(crate) balance: String,
    pub(crate) asset_type: String,
    #[serde(default)]
    pub(crate) asset_code: String,
    #[serde(default)]
    pub(crate) asset_issuer: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonFeeStats {
    pub(crate) fee_charged: HorizonFeeCharged,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonFeeCharged {
    pub(crate) mode: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonPayments {
    #[serde(rename = "_embedded")]
    pub(crate) embedded: HorizonPaymentsEmbedded,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonPaymentsEmbedded {
    pub(crate) records: Vec<HorizonPaymentRecord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonPaymentRecord {
    #[allow(dead_code)]
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) op_type: String,
    #[serde(default)]
    pub(crate) from: String,
    #[serde(default)]
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) amount: String,
    pub(crate) created_at: String,
    pub(crate) transaction_hash: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct StellarClient {
    pub(crate) endpoints: Vec<String>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl StellarClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}
// Stellar fetch paths (Horizon): native balance, per-asset balance, sequence,
// base fee, and payments history.


impl StellarClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<StellarBalance, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        let native = account
            .balances
            .iter()
            .find(|b| b.asset_type == "native")
            .ok_or("no native balance")?;
        // Stellar balances are decimal strings (e.g. "100.0000000")
        let stroops = parse_stellar_amount(&native.balance)?;
        Ok(StellarBalance {
            stroops,
            xlm_display: native.balance.clone(),
        })
    }

    /// Fetch a custom (issued) asset balance. `asset_code` is the alphanumeric
    /// asset code (e.g. "USDC"); `asset_issuer` is the G... issuer account.
    /// If the account has no trustline to this asset, returns a zero balance.
    pub async fn fetch_asset_balance(
        &self,
        address: &str,
        asset_code: &str,
        asset_issuer: &str,
    ) -> Result<StellarAssetBalance, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        let entry = account.balances.iter().find(|b| {
            b.asset_type != "native" && b.asset_code == asset_code && b.asset_issuer == asset_issuer
        });
        match entry {
            Some(b) => {
                let stroops = parse_stellar_amount(&b.balance)?;
                Ok(StellarAssetBalance {
                    asset_code: asset_code.to_string(),
                    asset_issuer: asset_issuer.to_string(),
                    amount_stroops: stroops,
                    amount_display: b.balance.clone(),
                })
            }
            None => Ok(StellarAssetBalance {
                asset_code: asset_code.to_string(),
                asset_issuer: asset_issuer.to_string(),
                amount_stroops: 0,
                amount_display: "0.0000000".to_string(),
            }),
        }
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u64, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        account
            .sequence
            .parse::<u64>()
            .map_err(|e| format!("sequence parse: {e}"))
    }

    pub async fn fetch_base_fee(&self) -> Result<u64, String> {
        let stats: HorizonFeeStats = self.get("/fee_stats").await?;
        Ok(stats.fee_charged.mode.parse::<u64>().unwrap_or(100))
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<StellarHistoryEntry>, String> {
        let payments: HorizonPayments = self
            .get(&format!(
                "/accounts/{address}/payments?limit=50&order=desc&include_failed=false"
            ))
            .await?;
        Ok(payments
            .embedded
            .records
            .into_iter()
            .filter(|r| r.op_type == "payment" || r.op_type == "create_account")
            .map(|r| {
                let amount_stroops = parse_stellar_amount(&r.amount).unwrap_or(0);
                let is_incoming = r.to == address;
                StellarHistoryEntry {
                    txid: r.transaction_hash,
                    ledger: 0,
                    timestamp: r.created_at,
                    from: r.from,
                    to: r.to,
                    amount_stroops,
                    fee_charged: 0,
                    is_incoming,
                }
            })
            .collect())
    }
}

pub(crate) fn parse_stellar_amount(s: &str) -> Result<i64, String> {
    // "100.0000000" -> stroops
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let whole: i64 = parts[0].parse().map_err(|e| format!("amount parse: {e}"))?;
    let frac_str = parts.get(1).copied().unwrap_or("0");
    let frac_padded = format!("{:0<7}", frac_str);
    let frac: i64 = frac_padded[..7].parse().unwrap_or(0);
    Ok(whole * 10_000_000 + frac)
}

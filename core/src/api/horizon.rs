//! The Stellar Horizon adapter: accounts, payments history, base fee and
//! envelope submission.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};

use crate::api::http::{HttpClient, RetryProfile, race};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarBalance {
    /// Stroops (1 XLM = 10_000_000 stroops).
    pub stroops: i64,
    pub xlm_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarHistoryEntry {
    pub txid: String,
    pub ledger: u64,
    /// Unix seconds, from Horizon's RFC 3339 `created_at`.
    pub timestamp: u64,
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

// ── Horizon API response types

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonAccount {
    pub(crate) balances: Vec<HorizonBalance>,
    pub(crate) sequence: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonBalance {
    pub(crate) balance: String,
    pub(crate) asset_type: String,
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
    #[serde(rename = "type")]
    pub(crate) op_type: String,
    #[serde(default)]
    pub(crate) from: String,
    #[serde(default)]
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) amount: String,
    /// `native` for XLM; a payment of an issued asset names its own.
    #[serde(default)]
    pub(crate) asset_type: String,
    /// `create_account` names its ends and amount differently.
    #[serde(default)]
    pub(crate) funder: String,
    #[serde(default)]
    pub(crate) account: String,
    #[serde(default)]
    pub(crate) starting_balance: String,
    pub(crate) created_at: String,
    pub(crate) transaction_hash: String,
}

// ── Client

pub struct HorizonClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl HorizonClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }
}
// Stellar fetch paths (Horizon): native balance, per-asset balance, sequence,
// base fee, and payments history.

impl HorizonClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<StellarBalance, ApiError> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        let native = account
            .balances
            .iter()
            .find(|b| b.asset_type == "native")
            .or_decode("no native balance")?;
        // Stellar balances are decimal strings (e.g. "100.0000000")
        let stroops = parse_stellar_amount(&native.balance)?;
        Ok(StellarBalance {
            stroops,
            xlm_display: native.balance.clone(),
        })
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u64, ApiError> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        account
            .sequence
            .parse::<u64>()
            .map_err(|e| ApiError::Decode(format!("sequence parse: {e}")))
    }

    pub async fn fetch_base_fee(&self) -> Result<u64, ApiError> {
        let stats: HorizonFeeStats = self.get("/fee_stats").await?;
        Ok(stats.fee_charged.mode.parse::<u64>().unwrap_or(100))
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<StellarHistoryEntry>, ApiError> {
        let payments: HorizonPayments = self
            .get(&format!(
                "/accounts/{address}/payments?limit=50&order=desc&include_failed=false"
            ))
            .await?;
        stellar_history_from_payments(payments.embedded.records, address)
    }
}

/// The XLM each payment record moved for `address`.
///
/// A `create_account` record carries its ends and amount as `funder`,
/// `account` and `starting_balance`. A payment of an issued asset is not XLM
/// and is left out rather than shown with the asset's amount under XLM's name.
fn stellar_history_from_payments(
    records: Vec<HorizonPaymentRecord>,
    address: &str,
) -> Result<Vec<StellarHistoryEntry>, ApiError> {
    let entries: Result<Vec<Option<StellarHistoryEntry>>, ApiError> = records
        .into_iter()
        .map(|r| {
            let (from, to, amount) = match r.op_type.as_str() {
                "payment" if r.asset_type == "native" => (r.from, r.to, r.amount),
                "create_account" => (r.funder, r.account, r.starting_balance),
                _ => return Ok(None),
            };
            let amount_stroops = parse_stellar_amount(&amount)?;
            // Horizon lists only operations already in a ledger.
            let timestamp = crate::api::time::confirmed_history_time(
                crate::api::time::parse_iso8601_timestamp(&r.created_at)
                    .filter(|t| *t > 0.0)
                    .map(|t| t as u64),
                &r.transaction_hash,
            )?;
            Ok(Some(StellarHistoryEntry {
                txid: r.transaction_hash,
                ledger: 0,
                timestamp,
                is_incoming: to == address,
                from,
                to,
                amount_stroops,
                fee_charged: 0,
            }))
        })
        .collect();
    Ok(entries?.into_iter().flatten().collect())
}

pub(crate) fn parse_stellar_amount(s: &str) -> Result<i64, ApiError> {
    // "100.0000000" -> stroops
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let whole: i64 = parts[0]
        .parse()
        .map_err(|e| ApiError::Decode(format!("amount parse: {e}")))?;
    let frac_str = parts.get(1).copied().unwrap_or("0");
    let frac_padded = format!("{:0<7}", frac_str);
    let frac: i64 = frac_padded[..7].parse().unwrap_or(0);
    Ok(whole * 10_000_000 + frac)
}

impl HorizonClient {
    /// Submit a pre-signed XDR envelope (for rebroadcast).
    pub async fn submit_envelope_b64(&self, tx_b64: &str) -> Result<StellarSendResult, ApiError> {
        let tx_b64 = tx_b64.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let tx_b64 = tx_b64.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: serde_json::Value = client
                    .post_json(
                        &url,
                        &serde_json::json!({"tx": tx_b64}),
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                let hash = resp
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(StellarSendResult {
                    txid: hash,
                    signed_xdr_b64: tx_b64.clone(),
                })
            }
        })
        .await
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "GA5XIGA5C7QTPTWXQHY6MCJRMTRZDOSHR6EFIBNDQTCQHG262N4GGKTM";
    const THEM: &str = "GBUXQE5RNV267EEVS6COJSHRKIE52GFVVA66TMM7UNAYLAOZP36PZ7YX";

    /// Record shapes as Horizon's `/accounts/{id}/payments` returns them.
    #[test]
    fn create_account_and_issued_assets_are_read_by_their_own_fields() {
        let records: HorizonPaymentsEmbedded = serde_json::from_value(serde_json::json!({
            "records": [
                {"type": "create_account", "created_at": "2025-08-06T02:05:01Z",
                 "transaction_hash": "created", "starting_balance": "241.5703630",
                 "funder": THEM, "account": ME},
                {"type": "payment", "created_at": "2025-08-06T01:17:03Z",
                 "transaction_hash": "xlm", "asset_type": "native",
                 "from": ME, "to": THEM, "amount": "82.1781600"},
                {"type": "payment", "created_at": "2025-08-06T01:18:03Z",
                 "transaction_hash": "usdc", "asset_type": "credit_alphanum4",
                 "from": THEM, "to": ME, "amount": "5000.0000000"}
            ]
        }))
        .unwrap();
        let entries = stellar_history_from_payments(records.records, ME).unwrap();
        assert_eq!(entries[0].timestamp, 1_754_445_901, "2025-08-06T02:05:01Z");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].txid, "created");
        assert!(entries[0].is_incoming);
        assert_eq!(entries[0].from, THEM);
        assert_eq!(entries[0].amount_stroops, 2_415_703_630);
        assert_eq!(entries[1].txid, "xlm");
        assert!(!entries[1].is_incoming);
        assert_eq!(entries[1].amount_stroops, 821_781_600);
    }
}

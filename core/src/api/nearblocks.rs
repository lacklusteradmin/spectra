//! The Nearblocks adapter: a NEAR account's transfers, which a NEAR node
//! does not index.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::HttpClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearHistoryEntry {
    pub txid: String,
    pub timestamp_ns: u64,
    /// The receipt's predecessor and receiver.
    pub from: String,
    pub to: String,
    pub amount_yocto: String,
    pub is_incoming: bool,
}

pub struct NearblocksClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl NearblocksClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// The account's NEAR transfers, newest first, from Nearblocks' receipt
    /// list.
    pub async fn fetch_history(&self, account_id: &str) -> Result<Vec<NearHistoryEntry>, ApiError> {
        let page: Value = self
            .client
            .get_path(
                &self.endpoints,
                &format!("/account/{account_id}/txns?per_page=50&order=desc"),
            )
            .await?;
        let receipts = page
            .get("txns")
            .and_then(Value::as_array)
            .or_decode("NEAR history: response has no txns")?;
        near_history_from_receipts(receipts, account_id)
    }
}

/// The NEAR each successful receipt attached, one entry per receipt that
/// moved some into or out of `account_id`.
///
/// A receipt with no deposit — a function call, a key change — moves no NEAR
/// and is not an entry. Nor is one from `system`: that is the protocol
/// refunding unused gas, part of a fee rather than a transfer.
///
/// Every listed receipt has executed in a block, so each has a time.
fn near_history_from_receipts(
    receipts: &[Value],
    account_id: &str,
) -> Result<Vec<NearHistoryEntry>, ApiError> {
    let entries: Result<Vec<Option<NearHistoryEntry>>, ApiError> = receipts
        .iter()
        .map(|receipt| {
            let text = |field: &str| receipt.get(field).and_then(Value::as_str).unwrap_or("");
            let from = text("predecessor_account_id");
            let to = text("receiver_account_id");
            let succeeded = receipt
                .pointer("/receipt_outcome/status")
                .or_else(|| receipt.pointer("/outcomes/status"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !succeeded || from == "system" || (from == account_id) == (to == account_id) {
                return Ok(None);
            }
            // Nearblocks reports the deposit as a JSON number, in exponent
            // form once it is large.
            let Some(amount) = receipt.pointer("/actions_agg/deposit").and_then(|deposit| {
                deposit
                    .as_f64()
                    .or_else(|| deposit.as_str().and_then(|s| s.parse().ok()))
            }) else {
                return Ok(None);
            };
            if amount <= 0.0 {
                return Ok(None);
            }
            let txid = text("transaction_hash");
            let timestamp_ns = crate::api::time::confirmed_history_time(
                text("block_timestamp").parse().ok(),
                txid,
            )?;
            Ok(Some(NearHistoryEntry {
                txid: txid.to_string(),
                timestamp_ns,
                from: from.to_string(),
                to: to.to_string(),
                amount_yocto: format!("{amount}"),
                is_incoming: to == account_id,
            }))
        })
        .collect();
    Ok(entries?.into_iter().flatten().collect())
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use serde_json::json;

    fn receipt(from: &str, to: &str, deposit: Value, status: bool) -> Value {
        json!({
            "predecessor_account_id": from,
            "receiver_account_id": to,
            "receipt_kind": "ACTION",
            "receipt_outcome": {"status": status},
            "transaction_hash": format!("{from}-{to}"),
            "block_timestamp": "1790508277775082937",
            "actions_agg": {"deposit": deposit}
        })
    }

    /// Receipt shapes as `api.nearblocks.io/v1/account/{id}/txns` returns them.
    #[test]
    fn only_successful_deposits_between_the_account_and_another_are_entries() {
        let receipts = [
            receipt("near", "me.near", json!(1e23), true),
            receipt("me.near", "them.near", json!(2.5e24), true),
            receipt("me.near", "contract.near", json!(0), true),
            receipt("system", "me.near", json!(7.2e18), true),
            receipt("me.near", "them.near", json!(1e24), false),
            receipt("me.near", "me.near", json!(1e24), true),
        ];
        let entries = near_history_from_receipts(&receipts, "me.near").unwrap();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert!(entries[0].is_incoming);
        assert_eq!(entries[0].from, "near");
        assert_eq!(entries[0].amount_yocto.parse::<f64>().unwrap(), 1e23);
        assert!(!entries[1].is_incoming);
        assert_eq!(entries[1].to, "them.near");
    }
}

//! NEAR Protocol chain client.
//!
//! Uses the NEAR JSON-RPC API for balance, nonce, block hash, history,
//! and transaction broadcast.
//! Transactions are BORSH-serialized and signed with Ed25519.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::fetch::http::{HttpClient, RetryProfile};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct NearBalance {
    /// yoctoNEAR (1 NEAR = 10^24 yoctoNEAR).
    pub yocto_near: String,
    pub near_display: String,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearSendResult {
    pub txid: String,
    /// Base64-encoded signed transaction — stored for rebroadcast.
    pub signed_tx_b64: String,
}

impl super::SignedSubmission for NearSendResult {
    fn submission_id(&self) -> &str {
        &self.txid
    }
    fn signed_payload(&self) -> &str {
        &self.signed_tx_b64
    }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat {
        super::SignedPayloadFormat::Base64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearFtMetadata {
    pub spec: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

// ── Client

pub struct NearClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl NearClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        crate::fetch::json_rpc::call(
            crate::EndpointApi::NearJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }
}

// NEAR fetch paths: view_account balance, access-key nonce, latest block hash,
// history (indexer), NEP-141 FT balance + metadata, and the UniFFI-exported

impl NearClient {
    pub async fn fetch_balance(&self, account_id: &str) -> Result<NearBalance, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_account",
                    "finality": "final",
                    "account_id": account_id
                }),
            )
            .await?;
        let yocto = result
            .get("amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let display = format_near(&yocto);
        Ok(NearBalance {
            yocto_near: yocto,
            near_display: display,
        })
    }

    pub async fn fetch_access_key_nonce(
        &self,
        account_id: &str,
        public_key_b58: &str,
    ) -> Result<u64, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_access_key",
                    "finality": "final",
                    "account_id": account_id,
                    "public_key": format!("ed25519:{public_key_b58}")
                }),
            )
            .await?;
        result
            .get("nonce")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "view_access_key: missing nonce".to_string())
    }

    pub async fn fetch_latest_block_hash(&self) -> Result<String, String> {
        let result = self.call("block", json!({"finality": "final"})).await?;
        result
            .pointer("/header/hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "block: missing hash".to_string())
    }

    /// The account's NEAR transfers, newest first, from Nearblocks' receipt
    /// list.
    pub async fn fetch_history(
        &self,
        account_id: &str,
        indexer_base: &str,
    ) -> Result<Vec<NearHistoryEntry>, String> {
        let url = format!(
            "{}/account/{}/txns?per_page=50&order=desc",
            indexer_base.trim_end_matches('/'),
            account_id
        );
        let page: Value = self.client.get_json(&url, RetryProfile::ChainRead).await?;
        let receipts = page
            .get("txns")
            .and_then(Value::as_array)
            .ok_or("NEAR history: response has no txns")?;
        near_history_from_receipts(receipts, account_id)
    }

    // ── NEP-141 (fungible token) support

    /// Call a view function on `contract` and return its decoded bytes.
    /// `args` is JSON that will be serialized, base64-encoded, and sent as
    /// `args_base64` per the NEAR `call_function` query type.
    pub(crate) async fn view_function(
        &self,
        contract: &str,
        method: &str,
        args: &Value,
    ) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let args_str = serde_json::to_string(args).map_err(|e| format!("args serialize: {e}"))?;
        let args_b64 = base64::engine::general_purpose::STANDARD.encode(args_str.as_bytes());
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "call_function",
                    "finality": "final",
                    "account_id": contract,
                    "method_name": method,
                    "args_base64": args_b64,
                }),
            )
            .await?;
        // `result.result` is a u8 array.
        let bytes = result
            .get("result")
            .and_then(|v| v.as_array())
            .ok_or("view_function: missing result bytes")?
            .iter()
            .filter_map(|n| n.as_u64().map(|n| n as u8))
            .collect::<Vec<u8>>();
        Ok(bytes)
    }

    pub async fn fetch_ft_balance_of(
        &self,
        contract: &str,
        account_id: &str,
    ) -> Result<u128, String> {
        let bytes = self
            .view_function(
                contract,
                "ft_balance_of",
                &json!({ "account_id": account_id }),
            )
            .await?;
        // Response body is a JSON string like `"1000000"`.
        let s: String =
            serde_json::from_slice(&bytes).map_err(|e| format!("ft_balance_of decode: {e}"))?;
        s.parse::<u128>()
            .map_err(|e| format!("ft_balance_of parse: {e}"))
    }

    pub async fn fetch_ft_metadata(&self, contract: &str) -> Result<NearFtMetadata, String> {
        let bytes = self
            .view_function(contract, "ft_metadata", &json!({}))
            .await?;
        #[derive(Deserialize)]
        struct RawMeta {
            spec: String,
            name: String,
            symbol: String,
            decimals: u8,
        }
        let meta: RawMeta =
            serde_json::from_slice(&bytes).map_err(|e| format!("ft_metadata decode: {e}"))?;
        Ok(NearFtMetadata {
            spec: meta.spec,
            name: meta.name,
            symbol: meta.symbol,
            decimals: meta.decimals,
        })
    }
}

// ── Formatting helpers

fn format_near(yocto: &str) -> String {
    // yocto is a 25-digit decimal; divide by 10^24 for NEAR.
    let n: u128 = yocto.parse().unwrap_or(0);
    let divisor: u128 = 1_000_000_000_000_000_000_000_000; // 10^24
    let whole = n / divisor;
    let frac = n % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:024}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 {
        &trimmed[..6]
    } else {
        trimmed
    };
    format!("{}.{}", whole, capped)
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
) -> Result<Vec<NearHistoryEntry>, String> {
    let entries: Result<Vec<Option<NearHistoryEntry>>, String> = receipts
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
            let timestamp_ns =
                super::confirmed_history_time(text("block_timestamp").parse().ok(), txid)?;
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

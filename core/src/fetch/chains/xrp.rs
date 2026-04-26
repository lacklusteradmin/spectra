//! XRP (Ripple) chain client.
//!
//! Uses the XRP Ledger JSON-RPC / REST API (rippled / Clio).
//! Transactions are serialized using XRP's binary codec (STObject)
//! and signed with secp256k1.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpBalance {
    /// XRP drops (1 XRP = 1_000_000 drops).
    pub drops: u64,
    pub xrp_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpHistoryEntry {
    pub txid: String,
    pub ledger_index: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_drops: u64,
    pub fee_drops: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpSendResult {
    pub txid: String,
    /// Signed tx blob hex — stored for rebroadcast.
    pub tx_blob_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct XrpClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl XrpClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = std::sync::Arc::new(rpc(method, params));
        with_fallback(&self.endpoints, |url| {
            let client = self.client.clone();
            let body = std::sync::Arc::clone(&body);
            async move {
                let resp: Value = client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await?;
                let result = resp
                    .get("result")
                    .ok_or_else(|| "missing result".to_string())?;
                if let Some(status) = result.get("status").and_then(|s| s.as_str()) {
                    if status == "error" {
                        let msg = result
                            .get("error_message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error");
                        return Err(format!("xrp rpc error: {msg}"));
                    }
                }
                Ok(result.clone())
            }
        })
        .await
    }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": [params] })
}
// XRP fetch paths: balance, sequence, fee, history.



impl XrpClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<XrpBalance, String> {
        let result = self
            .call("account_info", json!({"account": address, "ledger_index": "validated"}))
            .await?;
        let drops: u64 = result
            .pointer("/account_data/Balance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or("account_info: missing Balance")?;
        Ok(XrpBalance {
            drops,
            xrp_display: format_xrp(drops),
        })
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u32, String> {
        let result = self
            .call("account_info", json!({"account": address, "ledger_index": "current"}))
            .await?;
        result
            .pointer("/account_data/Sequence")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .ok_or_else(|| "account_info: missing Sequence".to_string())
    }

    pub async fn fetch_fee(&self) -> Result<u64, String> {
        let result = self.call("fee", json!({})).await?;
        result
            .pointer("/drops/open_ledger_fee")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "fee: missing open_ledger_fee".to_string())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<XrpHistoryEntry>, String> {
        let result = self
            .call(
                "account_tx",
                json!({"account": address, "limit": 50, "ledger_index_min": -1, "ledger_index_max": -1}),
            )
            .await?;
        let txs = result
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut entries = Vec::new();
        for item in txs {
            let tx = item.get("tx").unwrap_or(&Value::Null);
            let _meta = item.get("meta").unwrap_or(&Value::Null);

            let txtype = tx.get("TransactionType").and_then(|v| v.as_str()).unwrap_or("");
            if txtype != "Payment" {
                continue;
            }
            let txid = tx.get("hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ledger_index = tx.get("ledger_index").and_then(|v| v.as_u64()).unwrap_or(0);
            // XRP epoch: 2000-01-01, Unix epoch difference = 946684800
            let timestamp = tx
                .get("date")
                .and_then(|v| v.as_u64())
                .map(|d| d + 946_684_800)
                .unwrap_or(0);
            let from = tx.get("Account").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let to = tx.get("Destination").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let amount_drops = tx
                .get("Amount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0u64);
            let fee_drops = tx
                .get("Fee")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0u64);
            let is_incoming = to == address;

            entries.push(XrpHistoryEntry {
                txid,
                ledger_index,
                timestamp,
                from,
                to,
                amount_drops,
                fee_drops,
                is_incoming,
            });
        }
        Ok(entries)
    }
}

fn format_xrp(drops: u64) -> String {
    let whole = drops / 1_000_000;
    let frac = drops % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

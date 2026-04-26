//! Monero chain client.
//!
//! Monero is a privacy coin that uses ring signatures, stealth addresses,
//! and RingCT. Full Monero transaction construction requires a running
//! `monero-wallet-rpc` or embedded wallet with view key scanning.
//!
//! This implementation provides:
//!   - Balance and history via the Monero daemon RPC (getblockcount,
//!     get_transactions) using a wallet-rpc endpoint.
//!   - Transfer via wallet_rpc `transfer` method (the wallet RPC handles
//!     all cryptographic complexity: key image selection, range proofs, etc.)
//!
//! Architecture note: unlike other chains, Monero requires a wallet-rpc
//! process that has already been opened/synced with the view key. The
//! Spectra app is expected to maintain that side-channel. This client
//! only provides the JSON-RPC transport layer.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneroBalance {
    /// Atomic units (1 XMR = 1_000_000_000_000 atomic units / piconeros).
    pub piconeros: u64,
    pub xmr_display: String,
    /// Unlocked balance (spendable).
    pub unlocked_piconeros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneroHistoryEntry {
    pub txid: String,
    pub timestamp: u64,
    pub amount_piconeros: u64,
    pub fee_piconeros: u64,
    pub is_incoming: bool,
    pub confirmations: u64,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneroSendResult {
    pub txid: String,
    pub fee_piconeros: u64,
    pub amount_piconeros: u64,
}

// ----------------------------------------------------------------
// Client (wallet-rpc)
// ----------------------------------------------------------------

pub struct MoneroClient {
    /// Monero wallet-rpc endpoints (http://localhost:18082/json_rpc).
    wallet_rpc_endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl MoneroClient {
    pub fn new(wallet_rpc_endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            wallet_rpc_endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = std::sync::Arc::new(rpc(method, params));
        with_fallback(&self.wallet_rpc_endpoints, |url| {
            let client = self.client.clone();
            let body = std::sync::Arc::clone(&body);
            async move {
                let resp: Value = client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await?;
                if let Some(err) = resp.get("error") {
                    return Err(format!("monero rpc error: {err}"));
                }
                resp.get("result")
                    .cloned()
                    .ok_or_else(|| "missing result".to_string())
            }
        })
        .await
    }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": "0", "method": method, "params": params })
}
// Monero fetch paths (via wallet-rpc): balance, address, history,
// and sub-account creation (read-side metadata).



impl MoneroClient {
    pub async fn fetch_balance(&self, account_index: u32) -> Result<MoneroBalance, String> {
        let result = self
            .call("get_balance", json!({"account_index": account_index}))
            .await?;
        let piconeros = result.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
        let unlocked = result
            .get("unlocked_balance")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok(MoneroBalance {
            piconeros,
            xmr_display: format_xmr(piconeros),
            unlocked_piconeros: unlocked,
        })
    }

    pub async fn fetch_address(&self, account_index: u32) -> Result<String, String> {
        let result = self
            .call(
                "get_address",
                json!({"account_index": account_index, "address_index": [0]}),
            )
            .await?;
        result
            .get("address")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "get_address: missing address".to_string())
    }

    pub async fn fetch_history(
        &self,
        account_index: u32,
    ) -> Result<Vec<MoneroHistoryEntry>, String> {
        // Get incoming transfers.
        let in_result = self
            .call(
                "get_transfers",
                json!({
                    "in": true,
                    "out": true,
                    "account_index": account_index
                }),
            )
            .await?;

        let mut entries = Vec::new();

        for direction in &["in", "out"] {
            if let Some(txs) = in_result.get(direction).and_then(|v| v.as_array()) {
                let is_incoming = *direction == "in";
                for tx in txs {
                    let txid = tx.get("txid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let timestamp = tx.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
                    let amount = tx.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
                    let fee = tx.get("fee").and_then(|v| v.as_u64()).unwrap_or(0);
                    let confirmations = tx
                        .get("confirmations")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let note = tx
                        .get("note")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());

                    entries.push(MoneroHistoryEntry {
                        txid,
                        timestamp,
                        amount_piconeros: amount,
                        fee_piconeros: fee,
                        is_incoming,
                        confirmations,
                        note,
                    });
                }
            }
        }

        // Sort by timestamp descending.
        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(entries)
    }

    /// Create a new wallet account (for HD wallet sub-account).
    pub async fn create_account(&self, label: &str) -> Result<u32, String> {
        let result = self
            .call("create_account", json!({"label": label}))
            .await?;
        result
            .get("account_index")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .ok_or_else(|| "create_account: missing account_index".to_string())
    }
}

fn format_xmr(piconeros: u64) -> String {
    let whole = piconeros / 1_000_000_000_000;
    let frac = piconeros % 1_000_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:012}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

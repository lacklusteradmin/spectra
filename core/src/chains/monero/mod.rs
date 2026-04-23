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

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;

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
    wallet_rpc_endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl MoneroClient {
    pub fn new(wallet_rpc_endpoints: Vec<String>) -> Self {
        Self {
            wallet_rpc_endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = rpc(method, params);
        with_fallback(&self.wallet_rpc_endpoints, |url| {
            let client = self.client.clone();
            let body = body.clone();
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainRead)
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

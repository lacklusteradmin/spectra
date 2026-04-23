//! Solana chain client.
//!
//! Uses the Solana JSON-RPC API for balance, history, and broadcast.
//! Transaction serialization follows the compact (v0) wire format:
//!   [signatures] [message header] [accounts] [recent_blockhash] [instructions]
//!
//! Ed25519 signing is performed using the `ed25519-dalek` crate.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::{
    build_sol_transfer, build_spl_transfer_checked, derive_associated_token_account,
    ASSOCIATED_TOKEN_PROGRAM_ID, SPL_TOKEN_PROGRAM_ID,
};

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct SolanaBalance {
    /// Lamports (1 SOL = 1_000_000_000 lamports).
    pub lamports: u64,
    pub sol_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaHistoryEntry {
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub fee_lamports: u64,
    pub is_incoming: bool,
    pub amount_lamports: u64,
    pub from: String,
    pub to: String,
}

/// Unified history entry covering both native SOL and SPL token transfers.
/// Swift decodes this instead of `SolanaHistoryEntry` for the history tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaTransfer {
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub fee_lamports: u64,
    pub is_incoming: bool,
    /// Human-readable amount ("1.5", "0.001", …).
    pub amount_display: String,
    /// "SOL" for native, mint address for SPL token transfers.
    pub symbol: String,
    /// Empty string for native SOL; mint address for SPL.
    pub mint: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaSendResult {
    pub signature: String,
    #[serde(default)]
    pub signed_tx_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplBalance {
    pub mint: String,
    pub owner: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    /// Best-effort symbol. Solana token symbols live in Metaplex metadata PDAs
    /// which we don't resolve yet; this is an empty string for now.
    pub symbol: String,
}

// ----------------------------------------------------------------
// Solana client
// ----------------------------------------------------------------

pub struct SolanaClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl SolanaClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = rpc(method, params);
        with_fallback(&self.endpoints, |url| {
            let client = self.client.clone();
            let body = body.clone();
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainRead)
                    .await?;
                if let Some(err) = resp.get("error") {
                    return Err(format!("rpc error: {err}"));
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
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

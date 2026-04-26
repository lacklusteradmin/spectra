//! Internet Computer Protocol (ICP) chain client.
//!
//! Uses the IC management canister and Rosetta API for balance and history.
//! ICP ledger interactions use CBOR-encoded Candid messages.
//! Keys are derived with secp256k1 (BIP32); identity is verified via
//! self-authenticating principals (SHA-224 of the DER-encoded public key).
//!
//! For production send, the full Ingress message flow is:
//!   1. Build a `call` envelope (CBOR)
//!   2. Sign with the private key (ECDSA/secp256k1)
//!   3. POST to /api/v2/canister/{canister_id}/call
//!   4. Query with /api/v2/canister/{canister_id}/read_state

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Constants
// ----------------------------------------------------------------

/// ICP e8s per ICP.
pub(crate) const E8S_PER_ICP: u64 = 100_000_000;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpBalance {
    /// E8s (1 ICP = 100_000_000 e8s).
    pub e8s: u64,
    pub icp_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpHistoryEntry {
    pub block_index: u64,
    pub timestamp_ns: u64,
    pub from: String,
    pub to: String,
    pub amount_e8s: u64,
    pub fee_e8s: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpSendResult {
    pub block_index: u64,
}

// ----------------------------------------------------------------
// Client (Rosetta-based for read, direct for write)
// ----------------------------------------------------------------

pub struct IcpClient {
    /// Rosetta API endpoint (https://rosetta-api.internetcomputer.org).
    rosetta_endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl IcpClient {
    pub fn new(rosetta_endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            rosetta_endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn rosetta_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body = std::sync::Arc::new(body.clone());
        with_fallback(&self.rosetta_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = std::sync::Arc::clone(&body);
            async move { client.post_json(&url, &*body, RetryProfile::ChainRead).await }
        })
        .await
    }
}
// ICP fetch paths (via Rosetta): balance and history.

use serde_json::json;


impl IcpClient {
    pub async fn fetch_balance(&self, account_address: &str) -> Result<IcpBalance, String> {
        let resp: Value = self
            .rosetta_post(
                "/account/balance",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address}
                }),
            )
            .await?;
        let e8s: u64 = resp
            .pointer("/balances/0/value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Ok(IcpBalance {
            e8s,
            icp_display: format_icp(e8s),
        })
    }

    pub async fn fetch_history(
        &self,
        account_address: &str,
    ) -> Result<Vec<IcpHistoryEntry>, String> {
        let resp: Value = self
            .rosetta_post(
                "/search/transactions",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address},
                    "limit": 50
                }),
            )
            .await?;

        let txs = resp
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut entries = Vec::new();
        for item in txs {
            let block_index: u64 = item
                .pointer("/block_identifier/index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let timestamp_ns: u64 = item
                .pointer("/transaction/metadata/timestamp")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let ops = item
                .pointer("/transaction/operations")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut from = String::new();
            let mut to = String::new();
            let mut amount_e8s: u64 = 0;
            let mut fee_e8s: u64 = 0;

            for op in &ops {
                let op_type = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let addr = op
                    .pointer("/account/address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let value: i64 = op
                    .pointer("/amount/value")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                match op_type {
                    "TRANSACTION" => {
                        if value < 0 {
                            from = addr;
                            amount_e8s = value.unsigned_abs();
                        } else {
                            to = addr;
                        }
                    }
                    "FEE" => {
                        fee_e8s = value.unsigned_abs();
                    }
                    _ => {}
                }
            }

            let is_incoming = to == account_address;
            entries.push(IcpHistoryEntry {
                block_index,
                timestamp_ns,
                from,
                to,
                amount_e8s,
                fee_e8s,
                is_incoming,
            });
        }
        Ok(entries)
    }
}

fn format_icp(e8s: u64) -> String {
    let whole = e8s / E8S_PER_ICP;
    let frac = e8s % E8S_PER_ICP;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

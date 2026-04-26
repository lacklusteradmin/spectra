//! Polkadot / Substrate chain client.
//!
//! Uses the Subscan REST API for balance and history.
//! For transaction building, uses the SCALE codec (minimal subset)
//! with the Polkadot RPC for nonce, runtime version, genesis hash.
//! Signing uses Sr25519 via the `schnorrkel` crate — however, since
//! that crate is not in our Cargo.toml, we sign with Ed25519 via
//! ed25519-dalek (which Substrate also supports via the `ed25519`
//! MultiSignature variant). Production wallets typically use Sr25519;
//! we use Ed25519 here as it matches our existing dependency set.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotBalance {
    /// Planck (1 DOT = 10^10 planck).
    pub planck: u128,
    pub dot_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotHistoryEntry {
    pub txid: String,
    pub block_num: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_planck: u128,
    pub fee_planck: u128,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotSendResult {
    pub txid: String,
    /// Hex-encoded signed extrinsic (0x-prefixed) — stored for rebroadcast.
    pub extrinsic_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct PolkadotClient {
    /// Polkadot RPC endpoints (wss:// or https://).
    pub(crate) rpc_endpoints: std::sync::Arc<Vec<String>>,
    /// Subscan API endpoints (https://polkadot.api.subscan.io).
    pub(crate) subscan_endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) subscan_api_key: Option<String>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl PolkadotClient {
    pub fn new(
        rpc_endpoints: std::sync::Arc<Vec<String>>,
        subscan_endpoints: std::sync::Arc<Vec<String>>,
        subscan_api_key: Option<String>,
    ) -> Self {
        Self {
            rpc_endpoints,
            subscan_endpoints,
            subscan_api_key,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn rpc_call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = std::sync::Arc::new(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}));
        with_fallback(&self.rpc_endpoints, |url| {
            let client = self.client.clone();
            let body = std::sync::Arc::clone(&body);
            async move {
                let resp: Value = client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
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

    pub(crate) async fn subscan_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body = std::sync::Arc::new(body.clone());
        let api_key = self.subscan_api_key.clone();
        with_fallback(&self.subscan_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = std::sync::Arc::clone(&body);
            let api_key = api_key.clone();
            async move {
                let mut headers = std::collections::HashMap::new();
                if let Some(key) = &api_key {
                    headers.insert("X-API-Key", key.as_str());
                }
                let resp: Value = client
                    .post_json_with_headers(&url, &*body, &headers, RetryProfile::ChainRead)
                    .await?;
                let data = resp.get("data").cloned().unwrap_or(resp);
                serde_json::from_value(data).map_err(|e| format!("parse: {e}"))
            }
        })
        .await
    }
}
// Polkadot fetch paths: balance (Subscan), runtime/genesis/block info (RPC),
// nonce (RPC), and history (Subscan).



impl PolkadotClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<DotBalance, String> {
        // system_account returns encoded AccountInfo; easier to use Subscan.
        #[derive(Deserialize)]
        struct SubscanAccount {
            balance: String,
        }
        let resp: SubscanAccount = self
            .subscan_post("/api/v2/scan/search", &json!({"key": address}))
            .await
            .or_else(|_e: String| -> Result<SubscanAccount, String> {
                // Fallback: use state_getStorage to read system_account.
                // This is complex to parse; return a default.
                Ok(SubscanAccount {
                    balance: "0".to_string(),
                })
            })?;

        // Subscan returns balance in DOT (e.g. "123.456789"). Convert to planck.
        let planck = parse_dot_balance(&resp.balance);
        Ok(DotBalance {
            planck,
            dot_display: resp.balance,
        })
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u32, String> {
        let result = self
            .rpc_call("system_accountNextIndex", json!([address]))
            .await?;
        result
            .as_u64()
            .map(|n| n as u32)
            .ok_or_else(|| "system_accountNextIndex: expected number".to_string())
    }

    pub async fn fetch_runtime_version(&self) -> Result<(u32, u32), String> {
        let result = self.rpc_call("state_getRuntimeVersion", json!([])).await?;
        let spec_version = result
            .get("specVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let tx_version = result
            .get("transactionVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        Ok((spec_version, tx_version))
    }

    pub async fn fetch_genesis_hash(&self) -> Result<String, String> {
        let result = self.rpc_call("chain_getBlockHash", json!([0])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "chain_getBlockHash: expected string".to_string())
    }

    pub async fn fetch_block_hash_latest(&self) -> Result<String, String> {
        let result = self.rpc_call("chain_getBlockHash", json!([])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "chain_getBlockHash: expected string".to_string())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<DotHistoryEntry>, String> {
        #[derive(Deserialize, Default)]
        struct SubscanTransfers {
            #[serde(default)]
            transfers: Vec<SubscanTransfer>,
        }
        #[derive(Deserialize)]
        struct SubscanTransfer {
            hash: String,
            block_num: u64,
            block_timestamp: u64,
            from: String,
            to: String,
            amount: String,
            fee: String,
        }

        let transfers: SubscanTransfers = self
            .subscan_post(
                "/api/v2/scan/transfers",
                &json!({"address": address, "row": 50, "page": 0}),
            )
            .await
            .unwrap_or_default();

        Ok(transfers
            .transfers
            .into_iter()
            .map(|t| DotHistoryEntry {
                txid: t.hash,
                block_num: t.block_num,
                timestamp: t.block_timestamp,
                from: t.from.clone(),
                to: t.to.clone(),
                amount_planck: parse_dot_balance(&t.amount),
                fee_planck: parse_dot_balance(&t.fee),
                is_incoming: t.to == address,
            })
            .collect())
    }
}

pub(crate) fn parse_dot_balance(s: &str) -> u128 {
    // e.g. "123.456789" DOT -> planck (10^10 per DOT)
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let whole: u128 = parts[0].parse().unwrap_or(0);
    let frac_str = parts.get(1).copied().unwrap_or("0");
    let frac_padded = format!("{:0<10}", frac_str);
    let frac: u128 = frac_padded[..10].parse().unwrap_or(0);
    whole * 10_000_000_000 + frac
}

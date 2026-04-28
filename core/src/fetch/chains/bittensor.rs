//! Bittensor (subtensor) chain client.
//!
//! Bittensor is Substrate-based and exposes the standard Polkadot-style
//! JSON-RPC surface: `state_*`, `chain_*`, `system_*`, `author_*`. We use
//! the public OpenTensor entrypoint (`https://entrypoint-finney.opentensor.ai`).
//!
//! History uses Taostats (`api.taostats.io`) when an API key is provided;
//! otherwise history is empty (the on-chain RPC doesn't expose a transfer
//! index, so a third-party indexer is the only practical path).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaoBalance {
    /// Rao (1 TAO = 10^9 rao).
    pub rao: u128,
    pub tao_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaoHistoryEntry {
    pub txid: String,
    pub block_num: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_rao: u128,
    pub fee_rao: u128,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaoSendResult {
    pub txid: String,
    pub extrinsic_hex: String,
}

impl super::SignedSubmission for TaoSendResult {
    fn submission_id(&self) -> &str { &self.txid }
    fn signed_payload(&self) -> &str { &self.extrinsic_hex }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat { super::SignedPayloadFormat::Hex }
}

pub struct BittensorClient {
    pub(crate) rpc_endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) taostats_endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) taostats_api_key: Option<String>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl BittensorClient {
    pub fn new(
        rpc_endpoints: std::sync::Arc<Vec<String>>,
        taostats_endpoints: std::sync::Arc<Vec<String>>,
        taostats_api_key: Option<String>,
    ) -> Self {
        Self {
            rpc_endpoints,
            taostats_endpoints,
            taostats_api_key,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn rpc_call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = std::sync::Arc::new(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        );
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

    pub async fn fetch_balance(&self, address: &str) -> Result<TaoBalance, String> {
        // Subtensor exposes balance via system::account, encoded as SCALE.
        // Decoding the AccountInfo struct in Rust is non-trivial without a
        // codegen pipeline, so we lean on the Taostats API (which mirrors
        // Subscan's shape) when a key is configured.
        if let Some(_key) = &self.taostats_api_key {
            #[derive(Deserialize)]
            struct TaostatsAccount {
                balance_total: Option<String>,
            }
            let resp: Result<TaostatsAccount, _> = self
                .taostats_get(&format!("/api/account/v1?address={address}"))
                .await;
            if let Ok(acc) = resp {
                let raw = acc.balance_total.unwrap_or_default();
                let rao = raw.parse::<u128>().unwrap_or(0);
                return Ok(TaoBalance {
                    rao,
                    tao_display: format_tao(rao),
                });
            }
        }
        // Fallback: zero-balance default. Users without a Taostats API key
        // see only their own outgoing transfers reflected after RPC submit.
        Ok(TaoBalance {
            rao: 0,
            tao_display: "0".to_string(),
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

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<TaoHistoryEntry>, String> {
        if self.taostats_api_key.is_none() {
            return Ok(Vec::new());
        }
        #[derive(Deserialize, Default)]
        struct TaostatsTransfers {
            #[serde(default)]
            data: Vec<TaostatsTransfer>,
        }
        #[derive(Deserialize)]
        struct TaostatsTransfer {
            #[serde(default)]
            extrinsic_id: String,
            #[serde(default)]
            block_number: u64,
            #[serde(default)]
            timestamp: u64,
            #[serde(default)]
            from: String,
            #[serde(default)]
            to: String,
            #[serde(default)]
            amount: String,
            #[serde(default)]
            fee: String,
        }
        let transfers: TaostatsTransfers = self
            .taostats_get(&format!("/api/transfer/v1?address={address}&limit=50"))
            .await
            .unwrap_or_default();
        Ok(transfers
            .data
            .into_iter()
            .map(|t| TaoHistoryEntry {
                txid: t.extrinsic_id,
                block_num: t.block_number,
                timestamp: t.timestamp,
                from: t.from.clone(),
                to: t.to.clone(),
                amount_rao: t.amount.parse().unwrap_or(0),
                fee_rao: t.fee.parse().unwrap_or(0),
                is_incoming: t.to == address,
            })
            .collect())
    }

    async fn taostats_get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        let api_key = self.taostats_api_key.clone();
        with_fallback(&self.taostats_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let api_key = api_key.clone();
            async move {
                let mut headers = std::collections::HashMap::new();
                if let Some(key) = &api_key {
                    headers.insert("Authorization", key.as_str());
                }
                client
                    .get_json_with_headers(&url, &headers, RetryProfile::ChainRead)
                    .await
            }
        })
        .await
    }
}

pub(crate) fn format_tao(rao: u128) -> String {
    let whole = rao / 1_000_000_000;
    let frac = rao % 1_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:09}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

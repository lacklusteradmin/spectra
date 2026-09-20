//! Bittensor (subtensor) chain client.
//!
//! Bittensor is Substrate-based and exposes the standard Polkadot-style
//! JSON-RPC surface: `state_*`, `chain_*`, `system_*`, `author_*`. We use
//! the public OpenTensor entrypoint (`https://entrypoint-finney.opentensor.ai`).
//!
//! Balance and history both come from Taostats (`api.taostats.io`), which
//! needs an API key: the on-chain RPC returns `AccountInfo` as SCALE bytes and
//! exposes no transfer index, so a third-party indexer is the only practical
//! path to either number. Without a key this client refuses both reads rather
//! than reporting an empty wallet.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

/// What both Taostats reads answer when no key is configured. Neither can be
/// served from the RPC, so this is a missing setting, not a missing address.
const TAOSTATS_KEY_REQUIRED: &str =
    "Bittensor balance and history need a Taostats API key; none is configured";

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
    fn submission_id(&self) -> &str {
        &self.txid
    }
    fn signed_payload(&self) -> &str {
        &self.extrinsic_hex
    }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat {
        super::SignedPayloadFormat::Hex
    }
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

    /// Read the Taostats balance or return an error. Missing credentials,
    /// request failures, and malformed balances must not become zero holdings.
    pub async fn fetch_balance(&self, address: &str) -> Result<TaoBalance, String> {
        #[derive(Deserialize)]
        struct TaostatsAccount {
            balance_total: Option<String>,
        }
        if self.taostats_api_key.is_none() {
            return Err(TAOSTATS_KEY_REQUIRED.to_string());
        }
        let account: TaostatsAccount = self
            .taostats_get(&format!("/api/account/v1?address={address}"))
            .await?;
        let raw = account
            .balance_total
            .ok_or("taostats account: no balance_total")?;
        let rao = raw
            .parse::<u128>()
            .map_err(|e| format!("taostats balance_total {raw:?}: {e}"))?;
        Ok(TaoBalance {
            rao,
            tao_display: format_tao(rao),
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
            return Err(TAOSTATS_KEY_REQUIRED.to_string());
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
            .await?;
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

#[cfg(test)]
mod balance_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};

    fn client(taostats: &str, key: Option<&str>) -> BittensorClient {
        BittensorClient::new(
            Arc::new(vec![]),
            Arc::new(vec![taostats.to_string()]),
            key.map(str::to_string),
        )
    }

    /// Each of these answered `Ok(rao: 0)` once, and the first is the one the
    /// app hits by default: `api_keys` starts empty, so an unconfigured
    /// Bittensor wallet reported a balance of zero rather than saying it had
    /// no way to look.
    #[tokio::test]
    async fn a_balance_that_cannot_be_read_is_refused_rather_than_reported_as_zero() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"balance_total": null})))
            .mount(&server)
            .await;

        // No key: the read is impossible, not empty.
        assert!(client(&server.uri(), None)
            .fetch_balance("addr")
            .await
            .is_err());
        assert!(client(&server.uri(), None)
            .fetch_history("addr")
            .await
            .is_err());

        // Keyed, but the account carries no balance_total.
        let err = client(&server.uri(), Some("k"))
            .fetch_balance("addr")
            .await
            .unwrap_err();
        assert!(err.contains("balance_total"), "{err}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_balance_that_reads_is_returned_in_rao() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"balance_total": "2500000000"})),
            )
            .mount(&server)
            .await;
        let balance = client(&server.uri(), Some("k"))
            .fetch_balance("addr")
            .await
            .unwrap();
        assert_eq!(balance.rao, 2_500_000_000);
        assert_eq!(balance.tao_display, "2.5");
    }
}

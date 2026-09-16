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

// ── Public result types

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

impl super::SignedSubmission for DotSendResult {
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

// ── Client

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
                // Subscan refuses with `200` and `{"code":<non-zero>,
                // "message":...,"data":null}`. Decoding that `null` as `T`
                // fails with "invalid type: null", which names the shape
                // instead of the reason; the reason is in `message`.
                if resp
                    .get("code")
                    .and_then(Value::as_i64)
                    .is_some_and(|code| code != 0)
                {
                    let message = resp
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("no message");
                    return Err(format!("subscan refused: {message}"));
                }
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
    /// The balance Subscan reports, or why it could not be read.
    ///
    /// `system_account` returns `AccountInfo` as SCALE bytes and this client
    /// has no decoder for it, so Subscan is the only source of a number here.
    /// A failure used to fall through to `balance: "0"` — "return a default",
    /// the comment said — which turned every outage, rate limit and refusal
    /// into an account holding nothing.
    pub async fn fetch_balance(&self, address: &str) -> Result<DotBalance, String> {
        #[derive(Deserialize)]
        struct SubscanAccount {
            balance: String,
        }
        let resp: SubscanAccount = self
            .subscan_post("/api/v2/scan/search", &json!({"key": address}))
            .await?;

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
            .await?;

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

#[cfg(test)]
mod balance_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};

    fn client(subscan: &str) -> PolkadotClient {
        PolkadotClient::new(Arc::new(vec![]), Arc::new(vec![subscan.to_string()]), None)
    }

    /// Subscan refuses with `200` and a non-zero `code`, so the refusal has to
    /// be read out of the body. The balance used to `or_else` into `"0"` and
    /// the history into an empty list, which is how an outage rendered as an
    /// account holding nothing and having done nothing.
    #[tokio::test]
    async fn a_subscan_refusal_is_an_error_naming_its_reason() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"code": 10001, "message": "Record Not Found", "data": null}),
                ),
            )
            .mount(&server)
            .await;

        for err in [
            client(&server.uri())
                .fetch_balance("addr")
                .await
                .unwrap_err(),
            client(&server.uri())
                .fetch_history("addr")
                .await
                .unwrap_err(),
        ] {
            assert!(err.contains("Record Not Found"), "{err}");
            // Not the shape complaint decoding `data: null` used to produce.
            assert!(!err.contains("invalid type"), "{err}");
        }
    }

    #[tokio::test]
    async fn a_balance_that_reads_is_converted_to_planck() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"code": 0, "message": "Success", "data": {"balance": "12.5"}}),
            ))
            .mount(&server)
            .await;
        let balance = client(&server.uri()).fetch_balance("addr").await.unwrap();
        assert_eq!(balance.planck, 125_000_000_000);
        assert_eq!(balance.dot_display, "12.5");
    }
}

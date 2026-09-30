//! The Tron node HTTP API adapter (`/wallet/...`): balances, TRC-20 reads
//! through constant calls, block references and broadcast. Account history
//! and holdings come from `trongrid_v1`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile, race};
use sha2::{Digest, Sha256};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronBalance {
    /// SUN (1 TRX = 1_000_000 SUN).
    pub sun: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronSendResult {
    pub txid: String,
    /// Full signed transaction JSON for rebroadcast. Serialized as a JSON string.
    #[serde(default)]
    pub signed_tx_json: String,
}

/// TRC-20 balance payload. Mirrors `Erc20Balance` so the Swift-side decoder
/// can share a single response type if desired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Balance {
    pub contract: String,
    pub holder: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    pub symbol: String,
}

/// Lightweight TRC-20 metadata (symbol + decimals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Metadata {
    pub symbol: String,
    pub decimals: u8,
}

// ── Client

use crate::api::tron_metadata_cache::{self as metadata_cache, MetadataCache};

pub struct TronHttpClient {
    metadata_cache: Option<(crate::registry::Chain, std::sync::Arc<MetadataCache>)>,
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl TronHttpClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            metadata_cache: None,
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// Balance reads share metadata across wallets; signing uses `new` and fresh metadata.
    pub(crate) fn with_metadata_cache(
        endpoints: std::sync::Arc<Vec<String>>,
        chain: crate::registry::Chain,
        cache: std::sync::Arc<MetadataCache>,
    ) -> Self {
        Self {
            metadata_cache: Some((chain, cache)),
            ..Self::new(endpoints)
        }
    }

    async fn read_metadata(&self, contract: &str) -> Result<Trc20Metadata, String> {
        match &self.metadata_cache {
            Some((chain, cache)) => {
                cache
                    .get_or_fetch(
                        metadata_cache::Key {
                            chain: *chain,
                            endpoints: self.endpoints.clone(),
                            contract: contract.to_owned(),
                        },
                        self.fetch_trc20_metadata(contract),
                    )
                    .await
            }
            None => self.fetch_trc20_metadata(contract).await,
        }
    }

    pub(crate) async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let path = path.to_string();
        let body = std::sync::Arc::new(body.clone());
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = std::sync::Arc::clone(&body);
            async move {
                client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await
            }
        })
        .await
    }
}
// Tron fetch paths: balance, latest block, unified TRX+TRC-20 history,
// TRC-20 balance, TRC-20 metadata.

use serde_json::json;

use crate::derivation::tron::tron_base58_to_evm_hex;

impl TronHttpClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<TronBalance, String> {
        let resp = self
            .post(
                "/wallet/getaccount",
                &json!({"address": address, "visible": true}),
            )
            .await?;
        let sun = resp.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(TronBalance { sun })
    }

    /// Read the live balance and share cached symbol/decimals when this client
    /// belongs to a service read path. A standalone client reads all three.
    pub async fn fetch_trc20_balance(
        &self,
        contract_base58: &str,
        holder_base58: &str,
    ) -> Result<Trc20Balance, String> {
        let raw = self
            .fetch_trc20_balance_of(contract_base58, holder_base58)
            .await?;
        let metadata = self.read_metadata(contract_base58).await?;
        let balance_display = crate::decimal::from_units(raw, u32::from(metadata.decimals));
        Ok(Trc20Balance {
            contract: contract_base58.to_string(),
            holder: holder_base58.to_string(),
            balance_raw: raw.to_string(),
            balance_display,
            decimals: metadata.decimals,
            symbol: metadata.symbol,
        })
    }

    /// Raw `balanceOf(holder)` constant call.
    pub async fn fetch_trc20_balance_of(
        &self,
        contract_base58: &str,
        holder_base58: &str,
    ) -> Result<u128, String> {
        // TRC-20 uses the same 4-byte selector as ERC-20, but Tron addresses are
        // passed in their *hex* form (0x41... stripped to the last 20 bytes).
        let holder_hex = tron_base58_to_evm_hex(holder_base58)?;
        let parameter = format!("{:0>64}", holder_hex);

        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": holder_base58,
                    "contract_address": contract_base58,
                    "function_selector": "balanceOf(address)",
                    "parameter": parameter,
                    "visible": true
                }),
            )
            .await?;

        let hex_str = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("triggerconstantcontract balanceOf: missing result")?;

        // The result is a 32-byte big-endian integer hex string.
        parse_abi_u128(hex_str)
    }

    /// Name holdings another source enumerated: each contract's `decimals()`
    /// and `symbol()` are read concurrently, and one that will not answer is
    /// reported unnamed rather than dropped.
    pub async fn name_trc20_holdings(
        &self,
        held: Vec<(String, u128)>,
    ) -> Vec<crate::api::HeldToken> {
        let metadata = futures::future::join_all(
            held.iter()
                .map(|(contract, _)| self.read_metadata(contract)),
        )
        .await;
        held.into_iter()
            .zip(metadata)
            .map(|((contract, balance_raw), meta)| {
                let meta = meta.ok();
                crate::api::HeldToken {
                    contract,
                    balance_raw,
                    decimals: meta.as_ref().map(|m| m.decimals),
                    symbol: meta.map(|m| m.symbol),
                }
            })
            .collect()
    }

    /// Fetch token symbol + decimals.
    pub async fn fetch_trc20_metadata(
        &self,
        contract_base58: &str,
    ) -> Result<Trc20Metadata, String> {
        // decimals()
        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": contract_base58,
                    "contract_address": contract_base58,
                    "function_selector": "decimals()",
                    "parameter": "",
                    "visible": true
                }),
            )
            .await?;
        let decimals_hex = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("triggerconstantcontract decimals: missing result")?;
        let decimals = crate::api::checked_token_decimals(parse_abi_u128(decimals_hex)?)?;

        // symbol()
        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": contract_base58,
                    "contract_address": contract_base58,
                    "function_selector": "symbol()",
                    "parameter": "",
                    "visible": true
                }),
            )
            .await?;
        let symbol_hex = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("triggerconstantcontract symbol: missing result")?;
        let symbol = crate::api::evm_json_rpc::decode_abi_string_or_bytes32(symbol_hex)
            .ok_or("TRC20 symbol: malformed ABI string")?;

        Ok(Trc20Metadata { symbol, decimals })
    }
}

// ── TRC-20 helpers

/// A uint256 ABI word must be complete and fit the core's u128 amount type.
pub(crate) fn parse_abi_u128(hex_str: &str) -> Result<u128, String> {
    let word = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    if word.len() != 64 || !word.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("TRC20 integer: expected one 32-byte hex ABI word".into());
    }
    if !word[..32].bytes().all(|b| b == b'0') {
        return Err("TRC20 integer exceeds u128 range".into());
    }
    u128::from_str_radix(&word[32..], 16).map_err(|e| format!("TRC20 integer: {e}"))
}

/// Only the block reference is supplied by the node, never a transaction/hash.
pub(crate) struct BlockReference {
    pub number: u64,
    pub id: [u8; 32],
    pub timestamp_ms: u64,
}

impl TronHttpClient {
    /// The latest block, which a transfer references; the node supplies
    /// nothing else of it.
    pub(crate) async fn transfer_reference(&self) -> Result<BlockReference, String> {
        let block = self.post("/wallet/getnowblock", &json!({})).await?;
        let number = block
            .pointer("/block_header/raw_data/number")
            .and_then(Value::as_u64)
            .ok_or("missing Tron block number")?;
        let id = hex::decode(block["blockID"].as_str().ok_or("missing Tron block id")?)
            .map_err(|_| "invalid Tron block id")?
            .try_into()
            .map_err(|_| "Tron block id must be 32 bytes")?;
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "clock before epoch")?
            .as_millis()
            .try_into()
            .map_err(|_| "clock overflow")?;
        Ok(BlockReference {
            number,
            id,
            timestamp_ms,
        })
    }

    pub async fn broadcast_raw(&self, signed_tx_json: &str) -> Result<TronSendResult, String> {
        let body: Value = serde_json::from_str(signed_tx_json)
            .map_err(|e| format!("invalid signed Tron transaction: {e}"))?;
        let raw = hex::decode(
            body["raw_data_hex"]
                .as_str()
                .ok_or("missing Tron raw bytes")?,
        )
        .map_err(|_| "invalid Tron raw bytes")?;
        let txid = hex::encode(Sha256::digest(&raw));
        if body["txID"].as_str() != Some(txid.as_str()) {
            return Err("Tron transaction hash mismatch".into());
        }
        let result = self.post("/wallet/broadcasttransaction", &body).await?;
        if result["result"].as_bool() != Some(true) {
            return Err(format!("Tron broadcast refused: {result}"));
        }
        Ok(TronSendResult {
            txid,
            signed_tx_json: signed_tx_json.into(),
        })
    }
}

#[cfg(test)]
mod integer_tests {
    use super::*;
    #[test]
    fn abi_integers_never_truncate_or_accept_malformed_words() {
        assert_eq!(
            parse_abi_u128(&format!("{:064x}", u128::MAX)).unwrap(),
            u128::MAX
        );
        assert_eq!(parse_abi_u128(&"0".repeat(64)).unwrap(), 0);
        for bad in [
            "01".into(),
            "0".repeat(63),
            "0".repeat(65),
            format!("1{}", "0".repeat(63)),
            format!("{}z", "0".repeat(63)),
            format!("{}é", "0".repeat(62)),
        ] {
            assert!(parse_abi_u128(&bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn invalid_decimals_are_refused_before_reading_the_symbol() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};
        for result in [
            format!("{:064x}", 39),
            format!("{:064x}", 256),
            format!("1{}", "0".repeat(63)),
            "06".into(),
        ] {
            let server = MockServer::start().await;
            Mock::given(body_partial_json(json!({"function_selector":"decimals()"})))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"constant_result":[result]})),
                )
                .expect(1)
                .mount(&server)
                .await;
            assert!(
                TronHttpClient::new(std::sync::Arc::new(vec![server.uri()]))
                    .fetch_trc20_metadata("contract")
                    .await
                    .is_err()
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
}

#[cfg(test)]
mod metadata_cache_rpc_tests {
    use super::*;
    use serde_json::json;
    use std::{sync::Arc, time::Duration};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};

    #[tokio::test]
    async fn concurrent_balance_reads_share_metadata_but_sends_and_new_sources_read_fresh() {
        let server = MockServer::start().await;
        for (selector, result, expected) in [
            ("balanceOf(address)", format!("{:064x}", 1_000_000), 8),
            ("decimals()", format!("{:064x}", 6), 4),
            ("symbol()", format!("{:0<64}", hex::encode("TOKEN")), 4),
        ] {
            Mock::given(body_partial_json(json!({"function_selector":selector})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"constant_result":[result]}))
                        .set_delay(Duration::from_millis(20)),
                )
                .expect(expected)
                .mount(&server)
                .await;
        }
        let cache = Arc::new(MetadataCache::default());
        let endpoints = Arc::new(vec![server.uri()]);
        let contract = "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t";
        let holder = "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7";
        // Separate short-lived clients, just like separate wallet refreshes.
        let results = futures::future::join_all((0..8).map(|_| {
            let client = TronHttpClient::with_metadata_cache(
                endpoints.clone(),
                crate::registry::Chain::Tron,
                cache.clone(),
            );
            async move { client.fetch_trc20_balance(contract, holder).await.unwrap() }
        }))
        .await;
        for balance in results {
            assert_eq!(balance.decimals, 6);
            assert_eq!(balance.symbol, "TOKEN");
            assert_eq!(balance.balance_raw, "1000000");
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 10);
        // Even a cache-enabled client must bypass the cache for the explicit
        // metadata API used by the send builder.
        let reader = TronHttpClient::with_metadata_cache(
            endpoints.clone(),
            crate::registry::Chain::Tron,
            cache.clone(),
        );
        reader.fetch_trc20_metadata(contract).await.unwrap();
        TronHttpClient::with_metadata_cache(
            endpoints.clone(),
            crate::registry::Chain::TronNile,
            cache.clone(),
        )
        .read_metadata(contract)
        .await
        .unwrap();
        // A changed endpoint list is a different source, even for the same chain.
        let changed = Arc::new(vec![format!("{}/", server.uri())]);
        TronHttpClient::with_metadata_cache(changed, crate::registry::Chain::Tron, cache)
            .read_metadata(contract)
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 16);
    }
}

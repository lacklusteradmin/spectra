//! Tron chain client.
//!
//! Uses the TronGrid REST API: the node HTTP API and the v1 account API.
//! Transactions are built using a protobuf-like manual encoding (Tron uses
//! protobuf for its RawData but the on-wire format for transfers is simple).
//! Signing uses secp256k1 with keccak256 (same key derivation as Ethereum,
//! but Tron addresses use Base58Check with version byte 0x41).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fetch::http::{HttpClient, RetryProfile, with_fallback};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronBalance {
    /// SUN (1 TRX = 1_000_000 SUN).
    pub sun: u64,
    pub trx_display: String,
}

/// Unified history entry covering both native TRX and TRC-20 token transfers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronTransfer {
    pub contract: Option<String>,
    pub txid: String,
    /// Milliseconds since epoch, as TronGrid reports block times.
    pub timestamp_ms: u64,
    pub from: String,
    pub to: String,
    /// Human-readable amount string ("1.5", "10.0", …).
    pub amount_display: String,
    /// "TRX" for native, token abbreviation (e.g. "USDT") for TRC-20.
    pub symbol: String,
    pub is_incoming: bool,
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

use super::tron_metadata_cache::{self as metadata_cache, MetadataCache};

pub struct TronClient {
    metadata_cache: Option<(String, std::sync::Arc<MetadataCache>)>,
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl TronClient {
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
        chain: &str,
        cache: std::sync::Arc<MetadataCache>,
    ) -> Self {
        Self {
            metadata_cache: Some((chain.to_owned(), cache)),
            ..Self::new(endpoints)
        }
    }

    async fn read_metadata(&self, contract: &str) -> Result<Trc20Metadata, String> {
        match &self.metadata_cache {
            Some((chain, cache)) => {
                cache
                    .get_or_fetch(
                        metadata_cache::Key {
                            chain: chain.clone(),
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
        with_fallback(&self.endpoints, |base| {
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

impl TronClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<TronBalance, String> {
        let resp = self
            .post(
                "/wallet/getaccount",
                &json!({"address": address, "visible": true}),
            )
            .await?;
        let sun = resp.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(TronBalance {
            sun,
            trx_display: format_trx(sun),
        })
    }

    /// Up to `limit` recent confirmed transfers, native TRX and TRC-20,
    /// newest first, from TronGrid's v1 account API. Each entry of
    /// `account_endpoints` is a `…/v1/accounts` base; the first that answers
    /// both reads is used.
    pub async fn fetch_history(
        &self,
        address: &str,
        account_endpoints: &[String],
        limit: usize,
    ) -> Result<Vec<TronTransfer>, String> {
        let limit = limit.min(50);
        let (native, tokens): (Value, Value) =
            with_fallback(account_endpoints, |base| async move {
                let base = format!("{}/{address}", base.trim_end_matches('/'));
                let query = format!("limit={limit}&only_confirmed=true");
                let native = self
                    .client
                    .get_json(
                        &format!("{base}/transactions?{query}"),
                        RetryProfile::ChainRead,
                    )
                    .await?;
                let tokens = self
                    .client
                    .get_json(
                        &format!("{base}/transactions/trc20?{query}"),
                        RetryProfile::ChainRead,
                    )
                    .await?;
                Ok((native, tokens))
            })
            .await?;
        let mut entries = native_transfers(&native, address)?;
        entries.extend(token_transfers(&tokens, address)?);
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
        entries.truncate(limit);
        Ok(entries)
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
        let balance_display = crate::fetch::evm::format_token_amount(raw, metadata.decimals);
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

    /// Every TRC-20 the account holds, as TronGrid reports it.
    ///
    /// `/v1/accounts` returns contract addresses and raw balances but no
    /// decimals, so each holding needs its own `decimals()`/`symbol()` read;
    /// those run concurrently and a contract that will not answer is reported
    /// unnamed rather than dropped.
    pub async fn fetch_all_trc20_balances(
        &self,
        address: &str,
        account_endpoints: &[String],
    ) -> Result<Vec<super::HeldToken>, String> {
        let resp: Value = with_fallback(account_endpoints, |endpoint| async move {
            self.client
                .get_json(
                    &format!("{}/{address}", endpoint.trim_end_matches('/')),
                    RetryProfile::ChainRead,
                )
                .await
        })
        .await?;
        let mut held: Vec<(String, u128)> = Vec::new();
        for entry in resp
            .pointer("/data/0/trc20")
            .and_then(|v| v.as_array())
            .map(|v| v.as_slice())
            .unwrap_or_default()
        {
            let Some(map) = entry.as_object() else {
                continue;
            };
            for (contract, balance) in map {
                let Some(raw) = balance.as_str().and_then(|s| s.parse::<u128>().ok()) else {
                    continue;
                };
                if raw == 0 {
                    continue;
                }
                held.push((contract.clone(), raw));
            }
        }

        let metadata = futures::future::join_all(
            held.iter()
                .map(|(contract, _)| self.read_metadata(contract)),
        )
        .await;
        Ok(held
            .into_iter()
            .zip(metadata)
            .map(|((contract, balance_raw), meta)| {
                let meta = meta.ok();
                super::HeldToken {
                    contract,
                    balance_raw,
                    decimals: meta.as_ref().map(|m| m.decimals),
                    symbol: meta.map(|m| m.symbol),
                }
            })
            .collect())
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
        let decimals = super::checked_token_decimals(parse_abi_u128(decimals_hex)?)?;

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
        let symbol = crate::fetch::evm::decode_abi_string_or_bytes32(symbol_hex)
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

fn data(response: &Value) -> Result<&Vec<Value>, String> {
    response
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "TronGrid history: response has no data".to_string())
}

/// Successful TRX transfers. Other contract types (smart-contract calls,
/// staking, votes) move no TRX between accounts; TRC-20 movements come from
/// the token endpoint.
fn native_transfers(response: &Value, address: &str) -> Result<Vec<TronTransfer>, String> {
    let mut entries = Vec::new();
    for tx in data(response)? {
        let contract = tx.pointer("/raw_data/contract/0");
        if contract.and_then(|c| c.get("type")).and_then(Value::as_str) != Some("TransferContract")
            || tx.pointer("/ret/0/contractRet").and_then(Value::as_str) != Some("SUCCESS")
        {
            continue;
        }
        let txid = tx.get("txID").and_then(Value::as_str).unwrap_or_default();
        let value = contract
            .and_then(|c| c.pointer("/parameter/value"))
            .ok_or_else(|| format!("TronGrid history: transfer {txid} has no value"))?;
        let party = |field: &str| {
            value
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("TronGrid history: transfer {txid} has no {field}"))
                .and_then(crate::derivation::tron::tron_hex_to_base58)
        };
        let (from, to) = (party("owner_address")?, party("to_address")?);
        let sun = value
            .get("amount")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("TronGrid history: transfer {txid} has no amount"))?;
        entries.push(TronTransfer {
            contract: None,
            txid: txid.to_string(),
            timestamp_ms: super::confirmed_history_time(
                tx.get("block_timestamp").and_then(Value::as_u64),
                txid,
            )?,
            is_incoming: to == address,
            from,
            to,
            amount_display: format_trx(sun),
            symbol: "TRX".to_string(),
        });
    }
    Ok(entries)
}

/// TRC-20 `Transfer` events; approvals move nothing.
fn token_transfers(response: &Value, address: &str) -> Result<Vec<TronTransfer>, String> {
    let mut entries = Vec::new();
    for tx in data(response)? {
        if tx.get("type").and_then(Value::as_str) != Some("Transfer") {
            continue;
        }
        let txid = tx
            .get("transaction_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = |pointer: &str| {
            tx.pointer(pointer)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("TronGrid history: token transfer {txid} has no {pointer}"))
        };
        let raw: u128 = text("/value")?.parse().map_err(|_| {
            format!("TronGrid history: token transfer {txid} has a malformed value")
        })?;
        let decimals = tx
            .pointer("/token_info/decimals")
            .and_then(Value::as_u64)
            .and_then(|d| u32::try_from(d).ok())
            .filter(|d| *d <= 38)
            .ok_or_else(|| format!("TronGrid history: token transfer {txid} has no decimals"))?;
        let to = text("/to")?.to_string();
        entries.push(TronTransfer {
            contract: Some(text("/token_info/address")?.to_string()),
            txid: txid.to_string(),
            timestamp_ms: super::confirmed_history_time(
                tx.get("block_timestamp").and_then(Value::as_u64),
                txid,
            )?,
            from: text("/from")?.to_string(),
            is_incoming: to == address,
            to,
            amount_display: format_units(raw, decimals),
            symbol: text("/token_info/symbol").unwrap_or("?").to_string(),
        });
    }
    Ok(entries)
}

/// `raw` in whole units, exactly: no rounding, trailing zeros trimmed.
fn format_units(raw: u128, decimals: u32) -> String {
    let divisor = 10u128.pow(decimals);
    let (whole, frac) = (raw / divisor, raw % divisor);
    if frac == 0 {
        return whole.to_string();
    }
    let frac = format!("{frac:0>width$}", width = decimals as usize);
    format!("{whole}.{}", frac.trim_end_matches('0'))
}

fn format_trx(sun: u64) -> String {
    let whole = sun / 1_000_000;
    let frac = sun % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ME: &str = "TKHuVq1oKVruCGLvqVexFs6dawKv6fQgFs";

    #[tokio::test]
    async fn trongrid_history_merges_trx_and_trc20_transfers_newest_first() {
        let server = MockServer::start().await;
        let transfer = |id: &str, kind: &str, ret: &str, owner: &str, to: &str, time: u64| {
            serde_json::json!({
                "txID": id, "block_timestamp": time, "ret": [{"contractRet": ret}],
                "raw_data": {"contract": [{"type": kind, "parameter": {"value": {
                    "amount": 1_500_000, "owner_address": owner, "to_address": to,
                }}}]},
            })
        };
        let me = "4166426c7ac3d98b29191063833345b6bc540d7278";
        let them = "41add5246bd889365714a57579fc070ef81a8b6d81";
        Mock::given(method("GET"))
            .and(path(format!("/v1/accounts/{ME}/transactions")))
            .and(query_param("only_confirmed", "true"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [
                    transfer("in", "TransferContract", "SUCCESS", them, me, 3000),
                    transfer("call", "TriggerSmartContract", "SUCCESS", me, them, 2500),
                    transfer("failed", "TransferContract", "REVERT", me, them, 2000),
                ]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/accounts/{ME}/transactions/trc20")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [
                {"transaction_id": "usdt", "type": "Transfer", "block_timestamp": 4000,
                 "from": ME, "to": "TJ5usJLLwjwn7Pw3TPbdzreG7dvgKzfQ5y", "value": "1234500",
                 "token_info": {"symbol": "USDT", "address": "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", "decimals": 6}},
                {"transaction_id": "approve", "type": "Approval", "block_timestamp": 5000,
                 "from": ME, "to": "TJ5usJLLwjwn7Pw3TPbdzreG7dvgKzfQ5y", "value": "1",
                 "token_info": {"symbol": "USDT", "address": "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", "decimals": 6}},
            ]})))
            .mount(&server)
            .await;

        let history = TronClient::new(std::sync::Arc::new(vec![]))
            .fetch_history(ME, &[format!("{}/v1/accounts", server.uri())], 50)
            .await
            .unwrap();
        let ids: Vec<_> = history.iter().map(|t| t.txid.as_str()).collect();
        assert_eq!(ids, ["usdt", "in"]);
        assert_eq!(history[0].amount_display, "1.2345");
        assert_eq!(
            history[0].contract.as_deref(),
            Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")
        );
        assert!(!history[0].is_incoming);
        assert_eq!(history[1].amount_display, "1.5");
        assert_eq!(history[1].to, ME, "hex addresses come back in base58check");
        assert!(history[1].is_incoming);
    }

    #[tokio::test]
    async fn a_failed_read_is_an_error_rather_than_an_empty_history() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        assert!(
            TronClient::new(std::sync::Arc::new(vec![]))
                .fetch_history(ME, &[format!("{}/v1/accounts", server.uri())], 50)
                .await
                .is_err()
        );
    }

    #[test]
    fn token_amounts_are_exact() {
        assert_eq!(
            format_units(1_000_000_000_000_000_001, 18),
            "1.000000000000000001"
        );
        assert_eq!(format_units(5, 0), "5");
        assert_eq!(format_units(2_000_000, 6), "2");
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
                TronClient::new(std::sync::Arc::new(vec![server.uri()]))
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
            let client = TronClient::with_metadata_cache(endpoints.clone(), "tron", cache.clone());
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
        let reader = TronClient::with_metadata_cache(endpoints.clone(), "tron", cache.clone());
        reader.fetch_trc20_metadata(contract).await.unwrap();
        TronClient::with_metadata_cache(endpoints.clone(), "tron-nile", cache.clone())
            .read_metadata(contract)
            .await
            .unwrap();
        // A changed endpoint list is a different source, even for the same chain.
        let changed = Arc::new(vec![format!("{}/", server.uri())]);
        TronClient::with_metadata_cache(changed, "tron", cache)
            .read_metadata(contract)
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 16);
    }
}

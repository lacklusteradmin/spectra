//! The TronGrid v1 account API adapter (`…/v1/accounts`): an account's
//! transfers and TRC-20 holdings, which the node HTTP API does not index.
//! Native transactions also carry TRC-10 TransferAssetContract transfers.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile, race};

/// Unified history entry covering native TRX, TRC-10 and TRC-20 transfers.
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
    pub is_incoming: bool,
}

/// One or more `…/v1/accounts` bases.
pub struct TrongridClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
    trc10_node: Option<(
        crate::registry::Chain,
        std::sync::Arc<crate::api::tron_http::TronHttpClient>,
    )>,
}

impl TrongridClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
            trc10_node: None,
        }
    }

    pub(crate) fn with_trc10(
        endpoints: std::sync::Arc<Vec<String>>,
        chain: crate::registry::Chain,
        node: crate::api::tron_http::TronHttpClient,
    ) -> Self {
        Self {
            trc10_node: Some((chain, std::sync::Arc::new(node))),
            ..Self::new(endpoints)
        }
    }

    /// A page of up to `limit` confirmed transfers, native TRX, TRC-10 and
    /// TRC-20, newest first. One endpoint answers both reads.
    pub async fn fetch_history_page(
        &self,
        address: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<TronTransfer>, ApiError> {
        #[derive(Default, serde::Serialize, serde::Deserialize)]
        struct Cursor {
            native: Option<String>,
            tokens: Option<String>,
            native_done: bool,
            tokens_done: bool,
        }
        let mut position: Cursor = cursor
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        let limit = limit.clamp(1, 50);
        let (native, tokens): (Value, Value) = race(&self.endpoints, |base_url| {
            let position = &position;
            async move {
                let base = format!("/{address}");
                let fetch = |token: bool| {
                    let path = if token { "/transactions/trc20" } else { "/transactions" };
                    let (fingerprint, done) = if token { (&position.tokens, position.tokens_done) } else { (&position.native, position.native_done) };
                    let continuation = fingerprint.as_ref().map(|value| format!("&fingerprint={}", crate::api::history_page::query_value(value))).unwrap_or_default();
                    let url = format!("{}{base}{path}?limit={limit}&only_confirmed=true&order_by=block_timestamp,desc{continuation}", base_url.trim_end_matches('/'));
                    async move { if done { Ok(serde_json::json!({"data": []})) } else { self.client.get_json(&url, RetryProfile::ChainRead).await } }
                };
                let (native, tokens) = tokio::try_join!(fetch(false), fetch(true))?;
                Ok::<_, ApiError>((native, tokens))
            }
        }).await?;
        for (response, fingerprint, done) in [
            (&native, &mut position.native, &mut position.native_done),
            (&tokens, &mut position.tokens, &mut position.tokens_done),
        ] {
            let count = data(response)?.len();
            *fingerprint = response
                .pointer("/meta/fingerprint")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            *done = fingerprint.is_none();
            if count >= limit && *done {
                return Err(ApiError::Decode(
                    "TronGrid: full page has no continuation fingerprint".into(),
                ));
            }
        }
        let mut entries = native_transfers(&native, address)?;
        entries.extend(self.trc10_transfers(&native, address).await?);
        entries.extend(token_transfers(&tokens, address)?);
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
        let next_cursor = (!(position.native_done && position.tokens_done))
            .then(|| serde_json::to_string(&position))
            .transpose()?;
        Ok(crate::api::HistoryPage {
            items: entries,
            next_cursor,
        })
    }

    async fn trc10_transfers(
        &self,
        response: &Value,
        address: &str,
    ) -> Result<Vec<TronTransfer>, ApiError> {
        let mut entries = Vec::new();
        let mut metadata = std::collections::HashMap::new();
        for tx in data(response)? {
            if tx
                .pointer("/raw_data/contract/0/type")
                .and_then(Value::as_str)
                != Some("TransferAssetContract")
                || tx.pointer("/ret/0/contractRet").and_then(Value::as_str) != Some("SUCCESS")
            {
                continue;
            }
            let (chain, node) = self
                .trc10_node
                .as_ref()
                .or_decode("TRC-10 history requires a network-bound node metadata reader")?;
            let txid = tx
                .get("txID")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_decode("TRC-10 transfer has no transaction hash")?;
            let value = tx
                .pointer("/raw_data/contract/0/parameter/value")
                .or_decode("TRC-10 transfer has no contract value")?;
            let timestamp_ms = crate::api::time::confirmed_history_time(
                tx.get("block_timestamp").and_then(Value::as_u64),
                txid,
            )?;
            let name_end = chain
                .tron_trc10_name_end_ms()
                .or_decode("TRC-10 history token identity activation boundary is unverified")?;
            let uses_name = timestamp_ms <= name_end;
            let token = String::from_utf8(
                hex::decode(
                    value
                        .get("asset_name")
                        .and_then(Value::as_str)
                        .or_decode("TRC-10 transfer has no asset ID")?,
                )
                .map_err(ApiError::decode)?,
            )
            .map_err(ApiError::decode)?;
            let asset = match metadata.entry((uses_name, token.clone())) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let asset = if uses_name {
                        node.fetch_legacy_trc10_metadata(*chain, &token).await
                    } else {
                        node.fetch_trc10_metadata(*chain, &token).await
                    }
                    .map_err(|error| {
                        ApiError::decode(format!(
                            "TRC-10 history transaction {txid} has unverifiable asset identity: {error}"
                        ))
                    })?;
                    entry.insert(asset)
                }
            };
            let party = |field: &str| {
                value
                    .get(field)
                    .and_then(Value::as_str)
                    .or_decode("TRC-10 transfer party missing")
                    .and_then(|hex| {
                        crate::derivation::tron::tron_hex_to_base58(hex).map_err(ApiError::decode)
                    })
            };
            let (from, to) = (party("owner_address")?, party("to_address")?);
            if from != address && to != address {
                return Err(ApiError::decode(
                    "TRC-10 transfer is unrelated to the requested owner",
                ));
            }
            let raw = value
                .get("amount")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                .or_decode("TRC-10 transfer amount is invalid")?;
            entries.push(TronTransfer {
                contract: Some(asset.asset_id.clone()),
                txid: txid.into(),
                timestamp_ms,
                is_incoming: to == address,
                from,
                to,
                amount_display: crate::decimal::from_units(
                    u128::from(raw),
                    u32::from(asset.decimals),
                ),
            });
        }
        Ok(entries)
    }

    /// Every TRC-20 the account holds, as contract and raw balance.
    /// `/v1/accounts` reports no decimals or symbols.
    pub async fn fetch_trc20_holdings(
        &self,
        address: &str,
    ) -> Result<Vec<(String, u128)>, ApiError> {
        let resp: Value = race(&self.endpoints, |endpoint| async move {
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
        Ok(held)
    }
}

fn data(response: &Value) -> Result<&Vec<Value>, ApiError> {
    response
        .get("data")
        .and_then(Value::as_array)
        .or_decode("TronGrid history: response has no data")
}

/// Successful TRX transfers. Other contract types (smart-contract calls,
/// staking, votes) move no TRX between accounts; TRC-20 movements come from
/// the token endpoint.
fn native_transfers(response: &Value, address: &str) -> Result<Vec<TronTransfer>, ApiError> {
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
            .ok_or_else(|| {
                ApiError::Decode(format!("TronGrid history: transfer {txid} has no value"))
            })?;
        let party = |field: &str| {
            value
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ApiError::Decode(format!("TronGrid history: transfer {txid} has no {field}"))
                })
                .and_then(|hex| {
                    crate::derivation::tron::tron_hex_to_base58(hex).map_err(ApiError::decode)
                })
        };
        let (from, to) = (party("owner_address")?, party("to_address")?);
        let sun = value.get("amount").and_then(Value::as_u64).ok_or_else(|| {
            ApiError::Decode(format!("TronGrid history: transfer {txid} has no amount"))
        })?;
        entries.push(TronTransfer {
            contract: None,
            txid: txid.to_string(),
            timestamp_ms: crate::api::time::confirmed_history_time(
                tx.get("block_timestamp").and_then(Value::as_u64),
                txid,
            )?,
            is_incoming: to == address,
            from,
            to,
            amount_display: crate::decimal::from_units(u128::from(sun), 6),
        });
    }
    Ok(entries)
}

/// TRC-20 `Transfer` events; approvals move nothing.
fn token_transfers(response: &Value, address: &str) -> Result<Vec<TronTransfer>, ApiError> {
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
                .ok_or_else(|| {
                    ApiError::Decode(format!(
                        "TronGrid history: token transfer {txid} has no {pointer}"
                    ))
                })
        };
        let raw: u128 = text("/value")?.parse().map_err(|_| {
            ApiError::decode(format!(
                "TronGrid history: token transfer {txid} has a malformed value"
            ))
        })?;
        let decimals = tx
            .pointer("/token_info/decimals")
            .and_then(Value::as_u64)
            .and_then(|d| u32::try_from(d).ok())
            .filter(|d| *d <= 38)
            .ok_or_else(|| {
                ApiError::Decode(format!(
                    "TronGrid history: token transfer {txid} has no decimals"
                ))
            })?;
        let to = text("/to")?.to_string();
        entries.push(TronTransfer {
            contract: Some(text("/token_info/address")?.to_string()),
            txid: txid.to_string(),
            timestamp_ms: crate::api::time::confirmed_history_time(
                tx.get("block_timestamp").and_then(Value::as_u64),
                txid,
            )?,
            from: text("/from")?.to_string(),
            is_incoming: to == address,
            to,
            amount_display: crate::decimal::from_units(raw, decimals),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ME: &str = "TKHuVq1oKVruCGLvqVexFs6dawKv6fQgFs";

    #[tokio::test]
    async fn legacy_trc10_numeric_names_follow_each_networks_verified_activation() {
        use wiremock::matchers::body_json;
        for chain in [
            crate::registry::Chain::Tron,
            crate::registry::Chain::TronNile,
        ] {
            let server = MockServer::start().await;
            let end = chain.tron_trc10_name_end_ms().unwrap();
            let row = |hash: &str, asset: &str, time: u64| {
                serde_json::json!({
                    "txID":hash,"block_timestamp":time,"ret":[{"contractRet":"SUCCESS"}],
                    "raw_data":{"contract":[{"type":"TransferAssetContract","parameter":{"value":{
                        "asset_name":hex::encode(asset),"owner_address":"41add5246bd889365714a57579fc070ef81a8b6d81",
                        "to_address":"4166426c7ac3d98b29191063833345b6bc540d7278","amount":123,
                    }}}]},
                })
            };
            Mock::given(method("POST"))
                .and(path("/wallet/getblockbynum"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"blockID":chain.tron_genesis_block_id().unwrap()}),
                ))
                .expect(3)
                .mount(&server)
                .await;
            for (name, id) in [("Legacy", "1000002"), ("1009999", "1000001")] {
                let asset = serde_json::json!({"id":id,"name":hex::encode(name)});
                Mock::given(method("POST"))
                    .and(path("/wallet/getassetissuebyname"))
                    .and(body_json(serde_json::json!({"value":hex::encode(name)})))
                    .respond_with(ResponseTemplate::new(200).set_body_json(asset.clone()))
                    .expect(1)
                    .mount(&server)
                    .await;
                Mock::given(method("POST"))
                    .and(path("/wallet/getassetissuebyid"))
                    .and(body_json(serde_json::json!({"value":id})))
                    .respond_with(ResponseTemplate::new(200).set_body_json(asset))
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            Mock::given(method("POST"))
                .and(path("/wallet/getassetissuebyid"))
                .and(body_json(serde_json::json!({"value":"1009999"})))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"id":"1009999","name":hex::encode("Modern"),"precision":2}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            let client = TrongridClient::with_trc10(
                std::sync::Arc::new(vec![]),
                chain,
                crate::api::tron_http::TronHttpClient::new(std::sync::Arc::new(vec![server.uri()])),
            );
            let rows=client.trc10_transfers(&serde_json::json!({"data":[
                row("old-name","Legacy",end-3000),row("numeric-name","1009999",end),row("modern-id","1009999",end+9000),
            ]}),ME).await.unwrap();
            assert_eq!(
                rows.iter()
                    .map(|r| r.contract.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                ["1000002", "1000001", "1009999"]
            );
            assert_eq!(
                rows.iter()
                    .map(|r| r.amount_display.as_str())
                    .collect::<Vec<_>>(),
                ["123", "123", "1.23"]
            );
        }
    }

    #[tokio::test]
    async fn native_feed_trc10_rows_use_node_precision_and_one_metadata_read_per_id() {
        let server = MockServer::start().await;
        let row = |id: &str, amount: u64| {
            serde_json::json!({
                "txID":id,"block_timestamp":1700000000000u64,"ret":[{"contractRet":"SUCCESS"}],
                "raw_data":{"contract":[{"type":"TransferAssetContract","parameter":{"value":{
                    "asset_name":hex::encode("1002000"),"owner_address":"41add5246bd889365714a57579fc070ef81a8b6d81",
                    "to_address":"4166426c7ac3d98b29191063833345b6bc540d7278","amount":amount,
                }}}]},
            })
        };
        Mock::given(method("GET"))
            .and(path(format!("/v1/accounts/{ME}/transactions")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":[row("a",123),row("b",250)]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/accounts/{ME}/transactions/trc20")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/wallet/getblockbynum"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"blockID":crate::registry::Chain::Tron.tron_genesis_block_id().unwrap()})))
            .expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/wallet/getassetissuebyid"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id":"1002000","name":hex::encode("Legacy"),"precision":2}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let client = TrongridClient::with_trc10(
            std::sync::Arc::new(vec![format!("{}/v1/accounts", server.uri())]),
            crate::registry::Chain::Tron,
            crate::api::tron_http::TronHttpClient::new(std::sync::Arc::new(vec![server.uri()])),
        );
        let page = client.fetch_history_page(ME, 50, None).await.unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.next_cursor.is_none());
        assert_eq!(page.items[0].contract.as_deref(), Some("1002000"));
        assert_eq!(page.items[0].amount_display, "1.23");
        assert_eq!(page.items[1].amount_display, "2.5");
        assert!(page.items.iter().all(|row| row.is_incoming));
    }

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

        let history = TrongridClient::new(std::sync::Arc::new(vec![format!(
            "{}/v1/accounts",
            server.uri()
        )]))
        .fetch_history_page(ME, 50, None)
        .await
        .unwrap()
        .items;
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
            TrongridClient::new(std::sync::Arc::new(vec![format!(
                "{}/v1/accounts",
                server.uri()
            )]))
            .fetch_history_page(ME, 50, None)
            .await
            .is_err()
        );
    }
}

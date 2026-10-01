//! The TronGrid v1 account API adapter (`…/v1/accounts`): an account's
//! transfers and TRC-20 holdings, which the node HTTP API does not index.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile, race};

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

/// One or more `…/v1/accounts` bases.
pub struct TrongridClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl TrongridClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// Up to `limit` recent confirmed transfers, native TRX and TRC-20,
    /// newest first. One endpoint answers both reads.
    pub async fn fetch_history(
        &self,
        address: &str,
        limit: usize,
    ) -> Result<Vec<TronTransfer>, ApiError> {
        let limit = limit.min(50);
        let (native, tokens): (Value, Value) = race(&self.endpoints, |base| async move {
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
            Ok::<_, ApiError>((native, tokens))
        })
        .await?;
        let mut entries = native_transfers(&native, address)?;
        entries.extend(token_transfers(&tokens, address)?);
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
        entries.truncate(limit);
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
            symbol: "TRX".to_string(),
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
            symbol: text("/token_info/symbol").unwrap_or("?").to_string(),
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
        .fetch_history(ME, 50)
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
            TrongridClient::new(std::sync::Arc::new(vec![format!(
                "{}/v1/accounts",
                server.uri()
            )]))
            .fetch_history(ME, 50)
            .await
            .is_err()
        );
    }
}

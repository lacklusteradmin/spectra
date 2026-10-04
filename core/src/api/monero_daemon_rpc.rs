//! The Monero daemon RPC adapter: the HTTP transport `monero-daemon-rpc`
//! speaks over, and a daemon checked to be on the expected network, synced
//! and on a supported hard fork. Scanning and signing stay on the device, in
//! `send::monero_local`.

use crate::api::error::{ApiError, OrDecode};
use crate::{api::http::HttpClient, registry::Chain};
use monero_daemon_rpc::{HttpTransport, MoneroDaemon};
use monero_wallet::interface::InterfaceError;

#[derive(Clone)]
pub(crate) struct DaemonTransport {
    endpoint: String,
}
impl HttpTransport for DaemonTransport {
    async fn post(
        &self,
        route: &str,
        body: Vec<u8>,
        limit: Option<usize>,
    ) -> Result<Vec<u8>, InterfaceError> {
        let error = |e: String| InterfaceError::InterfaceError(e);
        let url = format!("{}/{}", self.endpoint.trim_end_matches('/'), route);
        let mut response = HttpClient::shared()
            .reqwest_client()
            .post(url)
            .body(body)
            .send()
            .await
            .map_err(|e| error(e.to_string()))?
            .error_for_status()
            .map_err(|e| error(e.to_string()))?;
        let limit = limit.unwrap_or(100 * 1024 * 1024).min(100 * 1024 * 1024);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| error(e.to_string()))? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(error("Monero response exceeds size limit".into()));
            }
            bytes.extend(chunk);
        }
        Ok(bytes)
    }
}
pub(crate) type Daemon = MoneroDaemon<DaemonTransport>;
pub(crate) async fn daemon(endpoint: &str, chain: Chain) -> Result<Daemon, ApiError> {
    let transport = DaemonTransport {
        endpoint: endpoint.into(),
    };
    let info: serde_json::Value = serde_json::from_slice(
        &transport
            .post("get_info", b"{}".to_vec(), Some(1024 * 1024))
            .await
            .map_err(ApiError::decode)?,
    )?;
    if info["nettype"].as_str() != Some(chain.monero_network_name()?)
        || info["synchronized"].as_bool() != Some(true)
    {
        return Err(ApiError::Decode(
            "Monero daemon is on the wrong network or is not synchronized".into(),
        ));
    }
    let fork: serde_json::Value = serde_json::from_slice(
        &transport
            .post(
                "json_rpc",
                br#"{"jsonrpc":"2.0","id":"0","method":"hard_fork_info"}"#.to_vec(),
                Some(1024 * 1024),
            )
            .await
            .map_err(ApiError::decode)?,
    )?;
    if fork["result"]["version"].as_u64() != Some(16) {
        return Err(ApiError::Decode(
            "Unsupported Monero hard fork; update before sending".into(),
        ));
    }
    MoneroDaemon::new(transport).await.map_err(ApiError::decode)
}

pub(crate) async fn fetch_transaction_status(
    endpoint: &str,
    chain: Chain,
    hash: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    crate::api::transaction_status::validate_hex_hash(hash)?;
    daemon(endpoint, chain).await?;
    let transport = DaemonTransport {
        endpoint: endpoint.into(),
    };
    let response: serde_json::Value = serde_json::from_slice(
        &transport
            .post(
                "get_transactions",
                serde_json::to_vec(
                    &serde_json::json!({"txs_hashes":[hash],"decode_as_json":false,"prune":true}),
                )?,
                Some(4 * 1024 * 1024),
            )
            .await
            .map_err(ApiError::decode)?,
    )?;
    monero_transaction_status(&response, hash)
}

fn monero_transaction_status(
    response: &serde_json::Value,
    hash: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    if response["status"].as_str() != Some("OK") || response["untrusted"].as_bool() == Some(true) {
        return Err(ApiError::decode(
            "Monero status: daemon did not supply a trusted result",
        ));
    }
    if response["missed_tx"].as_array().is_some_and(|rows| {
        rows.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(hash))
        })
    }) {
        return Ok(TransactionStatus::Pending);
    }
    let rows = response["txs"]
        .as_array()
        .filter(|rows| rows.len() == 1)
        .or_decode("Monero status: expected one transaction")?;
    let row = &rows[0];
    if !row["tx_hash"]
        .as_str()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(hash))
    {
        return Err(ApiError::decode("Monero status: transaction hash mismatch"));
    }
    if row["in_pool"]
        .as_bool()
        .or_decode("Monero status: missing pool membership")?
    {
        return Ok(TransactionStatus::Pending);
    }
    Ok(TransactionStatus::Confirmed {
        succeeded: true,
        block: Some(
            row["block_height"]
                .as_u64()
                .or_decode("Monero status: missing block height")?,
        ),
    })
}

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;
    use serde_json::json;

    #[test]
    fn missing_or_pool_transaction_cannot_become_a_failed_execution() {
        assert_eq!(
            monero_transaction_status(&json!({"status":"OK","missed_tx":["h"]}), "h").unwrap(),
            TransactionStatus::Pending
        );
        let mut response = json!({"status":"OK","untrusted":false,"txs":[{"tx_hash":"h","in_pool":true,"block_height":123}]});
        assert_eq!(
            monero_transaction_status(&response, "h").unwrap(),
            TransactionStatus::Pending
        );
        response["txs"][0]["in_pool"] = json!(false);
        assert_eq!(
            monero_transaction_status(&response, "h").unwrap(),
            TransactionStatus::Confirmed {
                succeeded: true,
                block: Some(123)
            }
        );
        assert!(monero_transaction_status(&response, "other").is_err());
        response["untrusted"] = json!(true);
        assert!(monero_transaction_status(&response, "h").is_err());
    }
}

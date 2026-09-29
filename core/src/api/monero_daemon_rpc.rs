//! The Monero daemon RPC adapter: the HTTP transport `monero-daemon-rpc`
//! speaks over, and a daemon checked to be on the expected network, synced
//! and on a supported hard fork. Scanning and signing stay on the device, in
//! `send::monero_local`.

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
pub(crate) async fn daemon(endpoint: &str, chain: Chain) -> Result<Daemon, String> {
    let transport = DaemonTransport {
        endpoint: endpoint.into(),
    };
    let info: serde_json::Value = serde_json::from_slice(
        &transport
            .post("get_info", b"{}".to_vec(), Some(1024 * 1024))
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if info["nettype"].as_str() != Some(chain.monero_network_name()?)
        || info["synchronized"].as_bool() != Some(true)
    {
        return Err("Monero daemon is on the wrong network or is not synchronized".into());
    }
    let fork: serde_json::Value = serde_json::from_slice(
        &transport
            .post(
                "json_rpc",
                br#"{"jsonrpc":"2.0","id":"0","method":"hard_fork_info"}"#.to_vec(),
                Some(1024 * 1024),
            )
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if fork["result"]["version"].as_u64() != Some(16) {
        return Err("Unsupported Monero hard fork; update before sending".into());
    }
    MoneroDaemon::new(transport)
        .await
        .map_err(|e| e.to_string())
}

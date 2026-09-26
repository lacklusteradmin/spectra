//! Bittensor RPC client. Balance and history have no configured keyless source.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::fetch::http::HttpClient;

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
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl BittensorClient {
    pub fn new(rpc_endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            rpc_endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn rpc_call(&self, method: &str, params: Value) -> Result<Value, String> {
        crate::fetch::json_rpc::call(
            crate::EndpointApi::SubstrateJsonRpc,
            &self.client,
            &self.rpc_endpoints,
            method,
            params,
        )
        .await
    }

    pub async fn fetch_balance(&self, _address: &str) -> Result<TaoBalance, String> {
        Err("Bittensor: no keyless balance source configured".into())
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

    pub async fn fetch_history(&self, _address: &str) -> Result<Vec<TaoHistoryEntry>, String> {
        Err("Bittensor: no keyless history source configured".into())
    }
}

#[cfg(test)]
mod unavailable_reads {
    use super::*;
    #[tokio::test]
    async fn missing_sources_are_errors_not_empty_wallets() {
        let client = BittensorClient::new(std::sync::Arc::new(vec![]));
        assert!(
            client
                .fetch_balance("address")
                .await
                .unwrap_err()
                .contains("no keyless balance source")
        );
        assert!(
            client
                .fetch_history("address")
                .await
                .unwrap_err()
                .contains("no keyless history source")
        );
    }
}

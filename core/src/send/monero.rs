//! Monero send: relay transfers via wallet-rpc + dry-run fee estimate.

use serde_json::json;

use crate::fetch::monero::{MoneroClient, MoneroSendResult};

impl MoneroClient {
    pub(crate) async fn relay_prepared(&self, payload: &str) -> Result<MoneroSendResult, String> {
        let body: serde_json::Value = serde_json::from_str(payload).map_err(|e| e.to_string())?;
        let metadata = body["hex"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("missing Monero relay metadata")?;
        let result = self.call("relay_tx", json!({"hex":metadata})).await?;
        let txid = result["tx_hash"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("relay_tx: missing tx_hash")?
            .to_owned();
        let fee = body["fee"].as_u64().unwrap_or(0);
        let amount = body["amount"].as_u64().unwrap_or(0);
        Ok(MoneroSendResult {
            txid,
            fee_piconeros: fee,
            amount_piconeros: amount,
        })
    }
}

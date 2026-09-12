//! Monero send: relay transfers via wallet-rpc + dry-run fee estimate.

use serde_json::json;

use crate::fetch::chains::monero::{MoneroClient, MoneroSendResult};

impl MoneroClient {
    /// Send XMR via wallet-rpc `transfer`.
    pub async fn send(
        &self,
        to_address: &str,
        piconeros: u64,
        account_index: u32,
        priority: u32, // 0=default, 1=unimportant, 2=normal, 3=elevated, 4=priority
    ) -> Result<MoneroSendResult, String> {
        let result = self
            .call(
                "transfer",
                json!({
                    "destinations": [{"amount": piconeros, "address": to_address}],
                    "account_index": account_index,
                    "subaddr_indices": [],
                    "priority": priority,
                    "get_tx_metadata": true,
                    "do_not_relay": true
                }),
            )
            .await?;
        let txid = result
            .get("tx_hash")
            .and_then(|v| v.as_str())
            .ok_or("transfer: missing tx_hash")?
            .to_string();
        let fee = result.get("fee").and_then(|v| v.as_u64()).unwrap_or(0);
        let amount = result.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
        let metadata = result["tx_metadata"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("transfer: missing tx_metadata")?;
        let payload = json!({"hex":metadata, "fee":fee, "amount":amount});
        crate::send::payload::before_submission(
            payload.to_string(),
            "txid",
            Some(txid.clone()),
            None,
        )
        .await?;
        self.relay_prepared(&payload.to_string()).await
    }

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

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
                    "get_tx_key": true,
                    "do_not_relay": false
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
        Ok(MoneroSendResult {
            txid,
            fee_piconeros: fee,
            amount_piconeros: amount,
        })
    }
}

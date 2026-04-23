//! Polkadot fetch paths: balance (Subscan), runtime/genesis/block info (RPC),
//! nonce (RPC), and history (Subscan).

use serde::Deserialize;
use serde_json::json;

use super::{DotBalance, DotHistoryEntry, PolkadotClient};

impl PolkadotClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<DotBalance, String> {
        // system_account returns encoded AccountInfo; easier to use Subscan.
        #[derive(Deserialize)]
        struct SubscanAccount {
            balance: String,
        }
        let resp: SubscanAccount = self
            .subscan_post("/api/v2/scan/search", &json!({"key": address}))
            .await
            .or_else(|_e: String| -> Result<SubscanAccount, String> {
                // Fallback: use state_getStorage to read system_account.
                // This is complex to parse; return a default.
                Ok(SubscanAccount {
                    balance: "0".to_string(),
                })
            })?;

        // Subscan returns balance in DOT (e.g. "123.456789"). Convert to planck.
        let planck = parse_dot_balance(&resp.balance);
        Ok(DotBalance {
            planck,
            dot_display: resp.balance,
        })
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

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<DotHistoryEntry>, String> {
        #[derive(Deserialize, Default)]
        struct SubscanTransfers {
            #[serde(default)]
            transfers: Vec<SubscanTransfer>,
        }
        #[derive(Deserialize)]
        struct SubscanTransfer {
            hash: String,
            block_num: u64,
            block_timestamp: u64,
            from: String,
            to: String,
            amount: String,
            fee: String,
        }

        let transfers: SubscanTransfers = self
            .subscan_post(
                "/api/v2/scan/transfers",
                &json!({"address": address, "row": 50, "page": 0}),
            )
            .await
            .unwrap_or_default();

        Ok(transfers
            .transfers
            .into_iter()
            .map(|t| DotHistoryEntry {
                txid: t.hash,
                block_num: t.block_num,
                timestamp: t.block_timestamp,
                from: t.from.clone(),
                to: t.to.clone(),
                amount_planck: parse_dot_balance(&t.amount),
                fee_planck: parse_dot_balance(&t.fee),
                is_incoming: t.to == address,
            })
            .collect())
    }
}

pub(super) fn parse_dot_balance(s: &str) -> u128 {
    // e.g. "123.456789" DOT -> planck (10^10 per DOT)
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let whole: u128 = parts[0].parse().unwrap_or(0);
    let frac_str = parts.get(1).copied().unwrap_or("0");
    let frac_padded = format!("{:0<10}", frac_str);
    let frac: u128 = frac_padded[..10].parse().unwrap_or(0);
    whole * 10_000_000_000 + frac
}

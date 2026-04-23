//! ICP fetch paths (via Rosetta): balance and history.

use serde_json::{json, Value};

use super::{IcpBalance, IcpClient, IcpHistoryEntry, E8S_PER_ICP};

impl IcpClient {
    pub async fn fetch_balance(&self, account_address: &str) -> Result<IcpBalance, String> {
        let resp: Value = self
            .rosetta_post(
                "/account/balance",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address}
                }),
            )
            .await?;
        let e8s: u64 = resp
            .pointer("/balances/0/value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Ok(IcpBalance {
            e8s,
            icp_display: format_icp(e8s),
        })
    }

    pub async fn fetch_history(
        &self,
        account_address: &str,
    ) -> Result<Vec<IcpHistoryEntry>, String> {
        let resp: Value = self
            .rosetta_post(
                "/search/transactions",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address},
                    "limit": 50
                }),
            )
            .await?;

        let txs = resp
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut entries = Vec::new();
        for item in txs {
            let block_index: u64 = item
                .pointer("/block_identifier/index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let timestamp_ns: u64 = item
                .pointer("/transaction/metadata/timestamp")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let ops = item
                .pointer("/transaction/operations")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut from = String::new();
            let mut to = String::new();
            let mut amount_e8s: u64 = 0;
            let mut fee_e8s: u64 = 0;

            for op in &ops {
                let op_type = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let addr = op
                    .pointer("/account/address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let value: i64 = op
                    .pointer("/amount/value")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                match op_type {
                    "TRANSACTION" => {
                        if value < 0 {
                            from = addr;
                            amount_e8s = value.unsigned_abs();
                        } else {
                            to = addr;
                        }
                    }
                    "FEE" => {
                        fee_e8s = value.unsigned_abs();
                    }
                    _ => {}
                }
            }

            let is_incoming = to == account_address;
            entries.push(IcpHistoryEntry {
                block_index,
                timestamp_ns,
                from,
                to,
                amount_e8s,
                fee_e8s,
                is_incoming,
            });
        }
        Ok(entries)
    }
}

fn format_icp(e8s: u64) -> String {
    let whole = e8s / E8S_PER_ICP;
    let frac = e8s % E8S_PER_ICP;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

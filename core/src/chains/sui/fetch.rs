//! Sui fetch paths: native balance, per-coin balance, history.

use serde_json::json;

use super::{SuiBalance, SuiClient, SuiHistoryEntry};

impl SuiClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<SuiBalance, String> {
        let result = self
            .call("suix_getBalance", json!([address, "0x2::sui::SUI"]))
            .await?;
        let mist: u64 = result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or("suix_getBalance: missing totalBalance")?;
        Ok(SuiBalance {
            mist,
            sui_display: format_sui(mist),
        })
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<SuiHistoryEntry>, String> {
        let result = self
            .call(
                "suix_queryTransactionBlocks",
                json!([
                    {"ToAddress": address},
                    null,
                    20,
                    true
                ]),
            )
            .await
            .unwrap_or(json!({"data": []}));

        let data = result
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(data
            .into_iter()
            .map(|item| {
                let digest = item.get("digest").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let timestamp_ms = item
                    .get("timestampMs")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                SuiHistoryEntry {
                    digest,
                    timestamp_ms,
                    is_incoming: true,
                    amount_mist: 0,
                    gas_mist: 0,
                }
            })
            .collect())
    }

    /// Fetch the balance for a specific coin type (e.g. `0x5d4b...::coin::COIN`).
    /// Returns the raw balance in the coin's smallest unit.
    pub async fn fetch_coin_balance(&self, address: &str, coin_type: &str) -> Result<u64, String> {
        let result = self
            .call("suix_getBalance", json!([address, coin_type]))
            .await?;
        result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("suix_getBalance: missing totalBalance for {coin_type}"))
    }
}

fn format_sui(mist: u64) -> String {
    let whole = mist / 1_000_000_000;
    let frac = mist % 1_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:09}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

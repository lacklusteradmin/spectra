//! Aptos fetch paths: balance, per-coin balance, account info, ledger info,
//! gas price, history.

use serde_json::Value;

use super::{AptosBalance, AptosClient, AptosHistoryEntry};

impl AptosClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<AptosBalance, String> {
        // The APT coin is stored in 0x1::coin::CoinStore<0x1::aptos_coin::AptosCoin>
        let path = format!(
            "/accounts/{address}/resource/0x1::coin::CoinStore%3C0x1::aptos_coin::AptosCoin%3E"
        );
        let resp: Value = self.get(&path).await?;
        let octas: u64 = resp
            .pointer("/data/coin/value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or("balance: missing coin value")?;
        Ok(AptosBalance {
            octas,
            apt_display: format_apt(octas),
        })
    }

    /// Fetch the balance for a specific coin type stored in
    /// `0x1::coin::CoinStore<{coin_type}>` (the legacy Aptos coin standard).
    /// Returns the raw balance in octas (or smallest unit).
    pub async fn fetch_coin_balance(&self, address: &str, coin_type: &str) -> Result<u64, String> {
        // Encode '<' and '>' so they survive as a URL path segment.
        let encoded = coin_type.replace('<', "%3C").replace('>', "%3E");
        let path = format!("/accounts/{address}/resource/0x1::coin::CoinStore%3C{encoded}%3E");
        let resp: Value = self.get(&path).await?;
        resp.pointer("/data/coin/value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("aptos: missing coin value for {coin_type}"))
    }

    pub async fn fetch_account_info(&self, address: &str) -> Result<(u64, u64), String> {
        let resp: Value = self.get(&format!("/accounts/{address}")).await?;
        let sequence: u64 = resp
            .get("sequence_number")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or("account: missing sequence_number")?;
        Ok((sequence, 0))
    }

    pub async fn fetch_ledger_info(&self) -> Result<(u64, String), String> {
        let resp: Value = self.get("/").await?;
        let chain_id: u64 = resp
            .get("chain_id")
            .and_then(|v| v.as_u64())
            .ok_or("ledger: missing chain_id")?;
        let ledger_version: String = resp
            .get("ledger_version")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        Ok((chain_id, ledger_version))
    }

    pub async fn fetch_gas_price(&self) -> Result<u64, String> {
        let resp: Value = self.get("/estimate_gas_price").await?;
        resp.get("gas_estimate")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "estimate_gas_price: missing gas_estimate".to_string())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<AptosHistoryEntry>, String> {
        let txs: Vec<Value> = self
            .get(&format!("/accounts/{address}/transactions?limit=50"))
            .await?;

        let mut entries = Vec::new();
        for tx in txs {
            let txtype = tx.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if txtype != "user_transaction" {
                continue;
            }
            let txid = tx.get("hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let version: u64 = tx
                .get("version")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let timestamp_us: u64 = tx
                .get("timestamp")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let from = tx
                .get("sender")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let gas_used: u64 = tx
                .get("gas_used")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let gas_unit_price: u64 = tx
                .get("gas_unit_price")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            // Extract coin transfer payload.
            let func = tx
                .pointer("/payload/function")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (to, amount_octas) = if func.contains("transfer") {
                let args = tx
                    .pointer("/payload/arguments")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let to = args.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
                let amount: u64 = args
                    .get(1)
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                (to, amount)
            } else {
                continue;
            };

            let is_incoming = to.eq_ignore_ascii_case(address);
            entries.push(AptosHistoryEntry {
                txid,
                version,
                timestamp_us,
                from,
                to,
                amount_octas,
                gas_used,
                gas_unit_price,
                is_incoming,
            });
        }
        Ok(entries)
    }
}

fn format_apt(octas: u64) -> String {
    let whole = octas / 100_000_000;
    let frac = octas % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

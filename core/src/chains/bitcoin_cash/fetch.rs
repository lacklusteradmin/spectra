//! BCH fetch paths (Blockbook REST): balance, UTXOs, fee estimate, history,
//! tx status.

use crate::http::{with_fallback, RetryProfile};

use super::derive::normalize_bch_address;
use super::{
    BchBalance, BchHistoryEntry, BchUtxo, BitcoinCashClient, BlockbookAddress,
    BlockbookFeeEstimate, BlockbookTx, BlockbookTxList, BlockbookUtxo,
};

impl BitcoinCashClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<BchBalance, String> {
        // Blockbook accepts both cashaddr and legacy.
        let norm = normalize_bch_address(address);
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{norm}?details=basic"))
            .await?;
        let sat: u64 = info.balance.parse().unwrap_or(0);
        Ok(BchBalance {
            balance_sat: sat,
            balance_display: format_bch(sat),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<BchUtxo>, String> {
        let norm = normalize_bch_address(address);
        let utxos: Vec<BlockbookUtxo> = self.get(&format!("/api/v2/utxo/{norm}")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| BchUtxo {
                txid: u.txid,
                vout: u.vout,
                value_sat: u.value.parse().unwrap_or(0),
                confirmations: u.confirmations,
            })
            .collect())
    }

    /// Fetch recommended fee rate for `blocks` confirmation target.
    /// Returns satoshis per vbyte. Falls back to 1 sat/vB on failure.
    pub async fn fetch_fee_rate(&self, blocks: u32) -> u64 {
        let estimate: Result<BlockbookFeeEstimate, _> =
            self.get(&format!("/api/v2/estimatefee/{blocks}")).await;
        estimate
            .ok()
            .and_then(|e| e.result.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|bch_per_kb| ((bch_per_kb * 1e8 / 1000.0).ceil() as u64).max(1))
            .unwrap_or(1)
    }

    /// Fetch the most recent 50 transactions for `address` via Blockbook's
    /// `details=txs` pagination. Blockbook normalizes BCH CashAddr inputs
    /// internally but we pass through `normalize_bch_address` as a safety
    /// check. Direction is detected from vin addresses.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<BchHistoryEntry>, String> {
        let norm = normalize_bch_address(address);
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{norm}?details=txs&page=1&pageSize=50"
            ))
            .await?;

        Ok(list
            .transactions
            .into_iter()
            .map(|tx| {
                let is_incoming = !tx.vin.iter().any(|i| {
                    i.addresses
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .any(|a| a == &norm || a == address)
                });
                let amount_sat: i64 = tx.value.parse().unwrap_or(0);
                let fee_sat: u64 = tx.fees.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
                BchHistoryEntry {
                    txid: tx.txid,
                    block_height: tx.block_height.unwrap_or(0),
                    timestamp: tx.block_time.unwrap_or(0),
                    amount_sat: if is_incoming { amount_sat } else { -amount_sat },
                    fee_sat,
                    is_incoming,
                }
            })
            .collect())
    }

    /// Fetch confirmation status for a single txid via Blockbook `/api/v2/tx/{txid}`.
    pub async fn fetch_tx_status(
        &self,
        txid: &str,
    ) -> Result<crate::chains::bitcoin::UtxoTxStatus, String> {
        let txid = txid.to_string();
        with_fallback(&self.endpoints, |base| {
            let txid = txid.clone();
            let client = self.client.clone();
            async move {
                let url = format!("{base}/api/v2/tx/{txid}");
                let tx: BlockbookTx = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
                Ok(crate::chains::bitcoin::UtxoTxStatus {
                    txid: tx.txid,
                    confirmed,
                    block_height: tx.block_height,
                    block_time: tx.block_time,
                    confirmations: None,
                })
            }
        })
        .await
    }
}

fn format_bch(sat: u64) -> String {
    let whole = sat / 100_000_000;
    let frac = sat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

//! Dogecoin fetch paths (Blockbook REST): balance, UTXOs, history, tx status.

use super::{
    BlockbookAddress, BlockbookTx, BlockbookTxList, BlockbookUtxo, DogeBalance,
    DogeHistoryEntry, DogeUtxo, DogecoinClient,
};

impl DogecoinClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<DogeBalance, String> {
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        let koin: u64 = info.balance.parse().unwrap_or(0);
        Ok(DogeBalance {
            balance_koin: koin,
            balance_display: format_doge(koin),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<DogeUtxo>, String> {
        let utxos: Vec<BlockbookUtxo> = self.get(&format!("/api/v2/utxo/{address}")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| DogeUtxo {
                txid: u.txid,
                vout: u.vout,
                value_koin: u.value.parse().unwrap_or(0),
                confirmations: u.confirmations,
            })
            .collect())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<DogeHistoryEntry>, String> {
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{address}?details=txs&page=1&pageSize=50"
            ))
            .await?;

        Ok(list
            .transactions
            .into_iter()
            .map(|tx| {
                let is_incoming = tx.vout.iter().any(|o| {
                    o.addresses
                        .as_deref()
                        .unwrap_or_default()
                        .contains(&address.to_string())
                });
                let amount_koin: i64 = tx.value.parse().unwrap_or(0);
                let fee_koin: u64 = tx.fees.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
                DogeHistoryEntry {
                    txid: tx.txid,
                    block_height: tx.block_height.unwrap_or(0),
                    timestamp: tx.block_time.unwrap_or(0),
                    amount_koin: if is_incoming { amount_koin } else { -amount_koin },
                    fee_koin,
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
        let tx: BlockbookTx = self.get(&format!("/api/v2/tx/{txid}")).await?;
        let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
        Ok(crate::chains::bitcoin::UtxoTxStatus {
            txid: tx.txid,
            confirmed,
            block_height: tx.block_height,
            block_time: tx.block_time,
            confirmations: Some(tx.confirmations),
        })
    }
}

fn format_doge(koin: u64) -> String {
    let whole = koin / 100_000_000;
    let frac = koin % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

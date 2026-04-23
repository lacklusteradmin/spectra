//! Cardano fetch paths (Blockfrost REST): balance, UTXOs, history, latest slot.

use serde::Deserialize;

use super::{
    BfAddress, BfTx, BfUtxo, CardanoBalance, CardanoClient, CardanoHistoryEntry, CardanoUtxo,
};

impl CardanoClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<CardanoBalance, String> {
        let info: BfAddress = self.get(&format!("/addresses/{address}")).await?;
        let lovelace: u64 = info
            .amount
            .iter()
            .find(|a| a.unit == "lovelace")
            .and_then(|a| a.quantity.parse().ok())
            .unwrap_or(0);
        Ok(CardanoBalance {
            lovelace,
            ada_display: format_ada(lovelace),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<CardanoUtxo>, String> {
        let utxos: Vec<BfUtxo> = self.get(&format!("/addresses/{address}/utxos")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| {
                let lovelace = u
                    .amount
                    .iter()
                    .find(|a| a.unit == "lovelace")
                    .and_then(|a| a.quantity.parse().ok())
                    .unwrap_or(0);
                CardanoUtxo {
                    tx_hash: u.tx_hash,
                    tx_index: u.tx_index,
                    lovelace,
                }
            })
            .collect())
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<CardanoHistoryEntry>, String> {
        #[derive(Deserialize)]
        struct BfTxRef {
            tx_hash: String,
        }
        let tx_refs: Vec<BfTxRef> = self
            .get(&format!("/addresses/{address}/transactions?count=50&order=desc"))
            .await?;

        let mut entries = Vec::new();
        for tx_ref in tx_refs {
            let tx: BfTx = match self.get(&format!("/txs/{}", tx_ref.tx_hash)).await {
                Ok(t) => t,
                Err(_) => continue,
            };
            let amount_lovelace: i64 = tx
                .output_amount
                .iter()
                .find(|a| a.unit == "lovelace")
                .and_then(|a| a.quantity.parse().ok())
                .unwrap_or(0i64);
            let fee_lovelace: u64 = tx.fees.parse().unwrap_or(0);
            entries.push(CardanoHistoryEntry {
                txid: tx.hash,
                block: tx.block,
                block_time: tx.block_time,
                is_incoming: amount_lovelace > 0,
                amount_lovelace,
                fee_lovelace,
            });
        }
        Ok(entries)
    }

    /// Fetch current slot from the latest block.
    pub async fn fetch_latest_slot(&self) -> Result<u64, String> {
        #[derive(Deserialize)]
        struct LatestBlock {
            slot: u64,
        }
        let block: LatestBlock = self.get("/blocks/latest").await?;
        Ok(block.slot)
    }
}

fn format_ada(lovelace: u64) -> String {
    let whole = lovelace / 1_000_000;
    let frac = lovelace % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

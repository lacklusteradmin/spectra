//! Bitcoin fetch paths (Esplora REST): balance, UTXOs, history, fee estimates,
//! and tx status.

use crate::http::{with_fallback, RetryProfile};

use super::{
    BitcoinBalance, BitcoinClient, BitcoinHistoryEntry, EsploraAddressStats, EsploraFeeEstimates,
    EsploraTx, EsploraTxStatus, EsploraUtxo, FeeRate, UtxoTxStatus,
};

impl BitcoinClient {
    // ----------------------------------------------------------------
    // Fetch: balance
    // ----------------------------------------------------------------

    pub async fn fetch_balance(&self, address: &str) -> Result<BitcoinBalance, String> {
        let addr = address.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
            let addr = addr.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/address/{addr}");
                let stats: EsploraAddressStats =
                    http.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed_sats = stats
                    .chain_stats
                    .funded_txo_sum
                    .saturating_sub(stats.chain_stats.spent_txo_sum);
                let unconfirmed_sats = stats.mempool_stats.funded_txo_sum as i64
                    - stats.mempool_stats.spent_txo_sum as i64;
                Ok(BitcoinBalance {
                    confirmed_sats,
                    unconfirmed_sats,
                    utxo_count: stats.chain_stats.tx_count as usize,
                })
            }
        })
        .await
    }

    // ----------------------------------------------------------------
    // Fetch: UTXOs
    // ----------------------------------------------------------------

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<EsploraUtxo>, String> {
        let addr = address.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
            let addr = addr.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/address/{addr}/utxo");
                http.get_json(&url, RetryProfile::ChainRead).await
            }
        })
        .await
    }

    // ----------------------------------------------------------------
    // Fetch: transaction history
    // ----------------------------------------------------------------

    pub async fn fetch_history(
        &self,
        address: &str,
        after_txid: Option<&str>,
    ) -> Result<Vec<BitcoinHistoryEntry>, String> {
        let addr = address.to_string();
        let cursor = after_txid.map(str::to_string);
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
            let addr = addr.clone();
            let cursor = cursor.clone();
            let http = http.clone();
            async move {
                let url = match &cursor {
                    Some(txid) => format!("{base}/address/{addr}/txs/chain/{txid}"),
                    None => format!("{base}/address/{addr}/txs"),
                };
                let txs: Vec<EsploraTx> = http.get_json(&url, RetryProfile::ChainRead).await?;

                Ok(txs
                    .into_iter()
                    .map(|tx| {
                        // Net change = sum of outputs to this address - sum of inputs from this address
                        let received: u64 = tx
                            .vout
                            .iter()
                            .filter(|o| o.scriptpubkey_address.as_deref() == Some(&addr))
                            .map(|o| o.value)
                            .sum();
                        let spent: u64 = tx
                            .vin
                            .iter()
                            .filter_map(|i| i.prevout.as_ref())
                            .filter(|o| o.scriptpubkey_address.as_deref() == Some(&addr))
                            .map(|o| o.value)
                            .sum();
                        BitcoinHistoryEntry {
                            txid: tx.txid,
                            confirmed: tx.status.confirmed,
                            block_height: tx.status.block_height,
                            block_time: tx.status.block_time,
                            net_sats: received as i64 - spent as i64,
                            fee_sats: tx.fee,
                        }
                    })
                    .collect())
            }
        })
        .await
    }

    // ----------------------------------------------------------------
    // Fetch: fee estimates
    // ----------------------------------------------------------------

    /// Returns the fee rate for `confirmation_target` blocks (typically
    /// 1, 6, or 144). Falls back to a conservative 10 sat/vB if the
    /// estimate is unavailable.
    pub async fn fetch_fee_rate(&self, confirmation_target: u32) -> Result<FeeRate, String> {
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        let estimates: EsploraFeeEstimates = with_fallback(&endpoints, |base| {
            let http = http.clone();
            async move {
                let url = format!("{base}/fee-estimates");
                http.get_json(&url, RetryProfile::ChainRead).await
            }
        })
        .await?;

        let key = confirmation_target.to_string();
        let sats_per_vbyte = estimates
            .targets
            .get(&key)
            // Fallback: take the next available target above.
            .or_else(|| {
                estimates
                    .targets
                    .iter()
                    .filter(|(k, _)| k.parse::<u32>().unwrap_or(u32::MAX) >= confirmation_target)
                    .min_by_key(|(k, _)| k.parse::<u32>().unwrap_or(u32::MAX))
                    .map(|(_, v)| v)
            })
            .copied()
            .unwrap_or(10.0);

        Ok(FeeRate { sats_per_vbyte })
    }

    // ----------------------------------------------------------------
    // Fetch: tx status (confirmation lookup)
    // ----------------------------------------------------------------

    /// Fetch the confirmation status for a single txid.
    /// Esplora `GET /tx/{txid}/status` returns `EsploraTxStatus` directly.
    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, String> {
        let txid = txid.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();
        with_fallback(&endpoints, |base| {
            let txid = txid.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/tx/{txid}/status");
                let s: EsploraTxStatus = http.get_json(&url, RetryProfile::ChainRead).await?;
                Ok(UtxoTxStatus {
                    txid: txid.clone(),
                    confirmed: s.confirmed,
                    block_height: s.block_height,
                    block_time: s.block_time,
                    confirmations: None,
                })
            }
        })
        .await
    }
}

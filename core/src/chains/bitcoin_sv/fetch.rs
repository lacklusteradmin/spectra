//! BSV fetch paths (WhatsOnChain REST): balance, UTXOs, history (with per-tx
//! enrichment), and tx status.

use crate::http::{with_fallback, RetryProfile};

use super::{
    BitcoinSvClient, BsvBalance, BsvHistoryEntry, BsvUtxo, WocBalance, WocHistoryItem,
    WocTxDetail, WocUtxo,
};

impl BitcoinSvClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<BsvBalance, String> {
        let bal: WocBalance = self.get(&format!("/address/{address}/balance")).await?;
        let confirmed = bal.confirmed.max(0) as u64;
        let unconfirmed = bal.unconfirmed.max(0) as u64;
        let total = confirmed.saturating_add(unconfirmed);
        Ok(BsvBalance {
            balance_sat: total,
            balance_display: format_bsv(total),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<BsvUtxo>, String> {
        let utxos: Vec<WocUtxo> = self.get(&format!("/address/{address}/unspent")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| BsvUtxo {
                txid: u.tx_hash,
                vout: u.tx_pos,
                value_sat: u.value,
                confirmations: if u.height > 0 { 1 } else { 0 },
            })
            .collect())
    }

    /// Fetch recent transactions for `address` via WhatsOnChain.
    ///
    /// WoC exposes `/address/{addr}/history` as a flat list of
    /// `{tx_hash, height}` entries. To populate amounts and timestamps we
    /// issue a sequential `/tx/hash/{hash}` fetch per entry.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<BsvHistoryEntry>, String> {
        let list: Vec<WocHistoryItem> = self.get(&format!("/address/{address}/history")).await?;

        let mut out: Vec<BsvHistoryEntry> = Vec::with_capacity(list.len());
        for item in list.into_iter() {
            let tx: WocTxDetail = match self.get(&format!("/tx/hash/{}", item.tx_hash)).await {
                Ok(t) => t,
                Err(_) => {
                    // Fall back to a bare entry so the user still sees the txid.
                    out.push(BsvHistoryEntry {
                        txid: item.tx_hash,
                        block_height: item.height.max(0) as u64,
                        timestamp: 0,
                        amount_sat: 0,
                        is_incoming: false,
                    });
                    continue;
                }
            };

            let (amount_sat, is_incoming) = bsv_compute_delta(&tx, address);
            let block_height = tx.blockheight.unwrap_or(item.height).max(0) as u64;
            let timestamp = tx.blocktime.or(tx.time).unwrap_or(0);

            out.push(BsvHistoryEntry {
                txid: item.tx_hash,
                block_height,
                timestamp,
                amount_sat,
                is_incoming,
            });
        }

        Ok(out)
    }

    /// Fetch confirmation status for a single txid via WoC `/tx/hash/{txid}`.
    pub async fn fetch_tx_status(
        &self,
        txid: &str,
    ) -> Result<crate::chains::bitcoin::UtxoTxStatus, String> {
        let txid = txid.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let txid = txid.clone();
            async move {
                let url = format!("{}/tx/hash/{}", base.trim_end_matches('/'), txid);
                let tx: WocTxDetail = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.blockheight.map(|h| h >= 0).unwrap_or(false);
                let block_height = tx.blockheight.filter(|&h| h >= 0).map(|h| h as u64);
                let block_time = tx.blocktime.or(tx.time);
                Ok(crate::chains::bitcoin::UtxoTxStatus {
                    txid: txid.clone(),
                    confirmed,
                    block_height,
                    block_time,
                    confirmations: None,
                })
            }
        })
        .await
    }
}

fn format_bsv(sat: u64) -> String {
    let whole = sat / 100_000_000;
    let frac = sat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

/// Best-effort amount/direction decoding from a WoC tx detail object.
///
/// We don't have the previous-output values for inputs without extra
/// round-trips, so "outgoing" is returned with a zero amount when we can
/// only prove the address appeared on the input side. For incoming txs we
/// sum the vout values destined for the queried address and convert BSV
/// floats to satoshis.
fn bsv_compute_delta(tx: &WocTxDetail, address: &str) -> (i64, bool) {
    // Outgoing detection: any vin whose `addr` matches us.
    let is_outgoing = tx.vin.iter().any(|v| v.addr.as_deref() == Some(address));

    // Incoming amount: sum vout values paid to us.
    let mut incoming_sats: u64 = 0;
    for v in &tx.vout {
        let pays_us = v
            .script_pub_key
            .as_ref()
            .and_then(|spk| spk.addresses.as_ref())
            .map(|addrs| addrs.iter().any(|a| a == address))
            .unwrap_or(false);
        if pays_us {
            // BSV float → satoshis; clamp negatives and NaN.
            let sats = (v.value * 100_000_000.0).round();
            if sats.is_finite() && sats >= 0.0 {
                incoming_sats = incoming_sats.saturating_add(sats as u64);
            }
        }
    }

    if is_outgoing {
        // Outgoing wins: amount is best-effort as the *negative* incoming
        // change (wallets that send to themselves will show a small delta).
        // When we can't attribute any value we return 0 rather than lie.
        let signed: i64 = incoming_sats.min(i64::MAX as u64) as i64;
        (-signed, false)
    } else if incoming_sats > 0 {
        (incoming_sats.min(i64::MAX as u64) as i64, true)
    } else {
        (0, false)
    }
}

//! Bitcoin Cash chain client.
//!
//! BCH uses the CashAddr address format (prefix "bitcoincash:") but can also
//! accept legacy P2PKH addresses (version 0x00, same as BTC). Signing is
//! SIGHASH_ALL with replay protection (BIP143 SegWit-style digest for BCH
//! is NOT used; BCH uses its own SIGHASH_FORKID = 0x40).
//!
//! We use Blockbook for balance/UTXO/broadcast.

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Blockbook types
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct BlockbookUtxo {
    pub(crate) txid: String,
    pub(crate) vout: u32,
    pub(crate) value: String,
    #[serde(default)]
    pub(crate) confirmations: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BlockbookAddress {
    pub(crate) balance: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BlockbookFeeEstimate {
    pub(crate) result: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlockbookTxList {
    #[serde(default)]
    pub(crate) transactions: Vec<BlockbookTx>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlockbookTx {
    pub(crate) txid: String,
    pub(crate) block_time: Option<u64>,
    pub(crate) block_height: Option<u64>,
    #[serde(default)]
    pub(crate) value: String,
    pub(crate) fees: Option<String>,
    #[serde(default)]
    pub(crate) vin: Vec<BlockbookVin>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BlockbookVin {
    pub(crate) addresses: Option<Vec<String>>,
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Net value change for the queried address. Negative = outgoing.
    pub amount_sat: i64,
    pub fee_sat: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct BitcoinCashClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl BitcoinCashClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}
// BCH fetch paths (Blockbook REST): balance, UTXOs, fee estimate, history,
// tx status.


use crate::derivation::chains::bitcoin_cash::normalize_bch_address;

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
    ) -> Result<crate::fetch::chains::bitcoin::UtxoTxStatus, String> {
        let txid = txid.to_string();
        with_fallback(&self.endpoints, |base| {
            let txid = txid.clone();
            let client = self.client.clone();
            async move {
                let url = format!("{base}/api/v2/tx/{txid}");
                let tx: BlockbookTx = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
                Ok(crate::fetch::chains::bitcoin::UtxoTxStatus {
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

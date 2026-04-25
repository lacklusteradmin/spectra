//! Dogecoin chain client.
//!
//! Uses Blockbook-compatible REST API (same as most UTXO explorers).
//! Signing uses secp256k1 / P2PKH (Dogecoin does not support SegWit).
//! Network params: version byte 0x1e (addresses start with 'D').

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Blockbook response types
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
    pub(crate) confirmations: u64,
    #[serde(default)]
    pub(crate) value: String,
    pub(crate) fees: Option<String>,
    pub(crate) vout: Vec<BlockbookVout>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BlockbookVout {
    pub(crate) addresses: Option<Vec<String>>,
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeBalance {
    /// Confirmed balance in koinus (1 DOGE = 100_000_000 koinus).
    pub balance_koin: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    pub amount_koin: i64, // negative = outgoing
    pub fee_koin: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_koin: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct DogecoinClient {
    pub(crate) endpoints: Vec<String>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl DogecoinClient {
    pub fn new(endpoints: Vec<String>) -> Self {
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
// Dogecoin fetch paths (Blockbook REST): balance, UTXOs, history, tx status.


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
    ) -> Result<crate::fetch::chains::bitcoin::UtxoTxStatus, String> {
        let txid = txid.to_string();
        let tx: BlockbookTx = self.get(&format!("/api/v2/tx/{txid}")).await?;
        let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
        Ok(crate::fetch::chains::bitcoin::UtxoTxStatus {
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

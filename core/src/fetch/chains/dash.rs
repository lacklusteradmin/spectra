//! Dash chain client.
//!
//! Trezor Blockbook REST surface (`https://dash1.trezor.io`). Same shape as
//! the BTG/LTC clients: balance, UTXOs, history, fee, broadcast, tx-status.

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

#[derive(Debug, Deserialize)]
pub(crate) struct BlockbookUtxo {
    pub(crate) txid: String,
    pub(crate) vout: u32,
    pub(crate) value: String,
    #[serde(default)]
    pub(crate) confirmations: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    pub amount_sat: i64,
    pub fee_sat: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

impl super::SignedSubmission for DashSendResult {
    fn submission_id(&self) -> &str { &self.txid }
    fn signed_payload(&self) -> &str { &self.raw_tx_hex }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat { super::SignedPayloadFormat::Hex }
}

pub struct DashClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl DashClient {
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

    pub async fn fetch_balance(&self, address: &str) -> Result<DashBalance, String> {
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        let sat: u64 = info.balance.parse().unwrap_or(0);
        Ok(DashBalance {
            balance_sat: sat,
            balance_display: format_dash(sat),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<DashUtxo>, String> {
        let utxos: Vec<BlockbookUtxo> = self.get(&format!("/api/v2/utxo/{address}")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| DashUtxo {
                txid: u.txid,
                vout: u.vout,
                value_sat: u.value.parse().unwrap_or(0),
                confirmations: u.confirmations,
            })
            .collect())
    }

    pub async fn fetch_fee_rate(&self, blocks: u32) -> u64 {
        let estimate: Result<BlockbookFeeEstimate, _> =
            self.get(&format!("/api/v2/estimatefee/{blocks}")).await;
        estimate
            .ok()
            .and_then(|e| e.result.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|dash_per_kb| ((dash_per_kb * 1e8 / 1000.0).ceil() as u64).max(1))
            .unwrap_or(1)
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<DashHistoryEntry>, String> {
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{address}?details=txs&page=1&pageSize=50"
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
                        .any(|a| a == address)
                });
                let amount_sat: i64 = tx.value.parse().unwrap_or(0);
                let fee_sat: u64 = tx.fees.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
                DashHistoryEntry {
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

fn format_dash(sat: u64) -> String {
    let whole = sat / 100_000_000;
    let frac = sat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

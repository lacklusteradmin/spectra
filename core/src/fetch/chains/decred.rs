//! Decred chain client.
//!
//! Backed by the dcrdata Insight-compatible REST surface
//! (`https://dcrdata.decred.org/insight/api`). Insight returns DCR amounts as
//! decimal strings (e.g. `"1.23456789"`), which we convert to atoms (1e-8
//! DCR) for storage parity with Bitcoin family chains.

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

#[derive(Debug, Deserialize)]
struct InsightAddress {
    #[serde(rename = "balanceSat")]
    balance_sat: Option<u64>,
    #[serde(default)]
    balance: f64,
}

#[derive(Debug, Deserialize)]
struct InsightUtxo {
    txid: String,
    vout: u32,
    #[serde(default)]
    satoshis: u64,
    #[serde(default)]
    amount: f64,
    #[serde(default)]
    confirmations: u32,
}

#[derive(Debug, Deserialize)]
struct InsightTxList {
    #[serde(default)]
    txs: Vec<InsightTx>,
}

#[derive(Debug, Deserialize)]
struct InsightTx {
    txid: String,
    #[serde(default)]
    blockheight: i64,
    #[serde(default)]
    time: u64,
    #[serde(default)]
    fees: f64,
    #[serde(default)]
    vin: Vec<InsightVin>,
    #[serde(default)]
    vout: Vec<InsightVout>,
}

#[derive(Debug, Deserialize)]
struct InsightVin {
    addr: Option<String>,
    #[serde(default, rename = "valueSat")]
    value_sat: u64,
    #[serde(default)]
    value: f64,
}

#[derive(Debug, Deserialize)]
struct InsightVout {
    #[serde(default)]
    value: String,
    #[serde(rename = "scriptPubKey")]
    script_pub_key: Option<InsightScriptPubKey>,
}

#[derive(Debug, Deserialize)]
struct InsightScriptPubKey {
    #[serde(default)]
    addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcrBalance {
    pub balance_atoms: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcrUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_atoms: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcrHistoryEntry {
    pub txid: String,
    pub block_height: i64,
    pub timestamp: u64,
    pub amount_atoms: i64,
    pub fee_atoms: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcrSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

impl super::SignedSubmission for DcrSendResult {
    fn submission_id(&self) -> &str { &self.txid }
    fn signed_payload(&self) -> &str { &self.raw_tx_hex }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat { super::SignedPayloadFormat::Hex }
}

#[derive(Debug, Deserialize)]
struct InsightBroadcastResponse {
    txid: String,
}

#[derive(Debug, Deserialize)]
struct InsightStatus {
    #[serde(default)]
    info: InsightStatusInfo,
}

#[derive(Debug, Deserialize, Default)]
struct InsightStatusInfo {
    #[serde(default)]
    blocks: u64,
}

pub struct DecredClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl DecredClient {
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

    pub async fn fetch_balance(&self, address: &str) -> Result<DcrBalance, String> {
        let info: InsightAddress = self.get(&format!("/addr/{address}?noTxList=1")).await?;
        let atoms = info
            .balance_sat
            .unwrap_or_else(|| (info.balance * 1e8).round() as u64);
        Ok(DcrBalance {
            balance_atoms: atoms,
            balance_display: format_dcr(atoms),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<DcrUtxo>, String> {
        let utxos: Vec<InsightUtxo> = self.get(&format!("/addr/{address}/utxo")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| {
                let atoms = if u.satoshis > 0 {
                    u.satoshis
                } else {
                    (u.amount * 1e8).round() as u64
                };
                DcrUtxo {
                    txid: u.txid,
                    vout: u.vout,
                    value_atoms: atoms,
                    confirmations: u.confirmations,
                }
            })
            .collect())
    }

    /// Decred fees vary with size; dcrdata exposes a recommended atoms/byte
    /// rate. We hardcode 10 atoms/byte (≈ standard wallet default) and let
    /// callers override via `fee_sat` on the send request if needed.
    pub async fn fetch_fee_rate(&self, _blocks: u32) -> u64 {
        10
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<DcrHistoryEntry>, String> {
        let list: InsightTxList = self.get(&format!("/txs?address={address}")).await?;
        Ok(list
            .txs
            .into_iter()
            .map(|tx| {
                let owned_in: i64 = tx
                    .vin
                    .iter()
                    .filter(|v| v.addr.as_deref().map(|a| a == address).unwrap_or(false))
                    .map(|v| {
                        if v.value_sat > 0 {
                            v.value_sat as i64
                        } else {
                            (v.value * 1e8).round() as i64
                        }
                    })
                    .sum();
                let owned_out: i64 = tx
                    .vout
                    .iter()
                    .filter(|o| {
                        o.script_pub_key
                            .as_ref()
                            .map(|s| s.addresses.iter().any(|a| a == address))
                            .unwrap_or(false)
                    })
                    .map(|o| {
                        o.value
                            .parse::<f64>()
                            .ok()
                            .map(|v| (v * 1e8).round() as i64)
                            .unwrap_or(0)
                    })
                    .sum();
                let net = owned_out - owned_in;
                let is_incoming = net >= 0;
                let fee_atoms = (tx.fees * 1e8).round() as u64;
                DcrHistoryEntry {
                    txid: tx.txid,
                    block_height: tx.blockheight,
                    timestamp: tx.time,
                    amount_atoms: net,
                    fee_atoms,
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
                let url = format!("{base}/tx/{txid}");
                let tx: InsightTx = client.get_json(&url, RetryProfile::ChainRead).await?;
                let height = if tx.blockheight > 0 {
                    Some(tx.blockheight as u64)
                } else {
                    None
                };
                Ok(crate::fetch::chains::bitcoin::UtxoTxStatus {
                    txid: tx.txid,
                    confirmed: height.is_some(),
                    block_height: height,
                    block_time: if tx.time > 0 { Some(tx.time) } else { None },
                    confirmations: None,
                })
            }
        })
        .await
    }

    pub async fn fetch_chain_tip_height(&self) -> Result<u64, String> {
        let status: InsightStatus = self.get("/status?q=getInfo").await?;
        Ok(status.info.blocks)
    }

    pub async fn broadcast_raw_tx(&self, raw_tx_hex: &str) -> Result<DcrSendResult, String> {
        let raw_hex = raw_tx_hex.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let raw_hex = raw_hex.clone();
            let url = format!("{}/tx/send", base.trim_end_matches('/'));
            async move {
                let body = serde_json::json!({ "rawtx": raw_hex.clone() });
                let resp: InsightBroadcastResponse = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                Ok(DcrSendResult {
                    txid: resp.txid,
                    raw_tx_hex: raw_hex,
                })
            }
        })
        .await
    }
}

fn format_dcr(atoms: u64) -> String {
    let whole = atoms / 100_000_000;
    let frac = atoms % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

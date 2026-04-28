//! Kaspa chain client.
//!
//! Backed by the public REST surface at `https://api.kaspa.org`. Balances
//! and outpoint values are denominated in `sompi` (1e-8 KAS).

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

#[derive(Debug, Deserialize)]
struct ApiBalance {
    #[serde(default)]
    balance: u64,
}

#[derive(Debug, Deserialize)]
struct ApiUtxo {
    address: String,
    outpoint: ApiOutpoint,
    #[serde(rename = "utxoEntry")]
    utxo_entry: ApiUtxoEntry,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiOutpoint {
    transaction_id: String,
    index: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiUtxoEntry {
    amount: String,
    script_public_key: ApiScriptPublicKey,
    block_daa_score: Option<String>,
    is_coinbase: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ApiScriptPublicKey {
    version: u32,
    script_public_key: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApiTxEntry {
    #[serde(default)]
    transaction_id: String,
    #[serde(default)]
    block_time: u64,
    #[serde(default)]
    accepting_block_blue_score: Option<u64>,
    #[serde(default)]
    inputs: Vec<ApiTxInput>,
    #[serde(default)]
    outputs: Vec<ApiTxOutput>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApiTxInput {
    #[serde(default)]
    previous_outpoint_address: Option<String>,
    #[serde(default)]
    previous_outpoint_amount: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApiTxOutput {
    #[serde(default)]
    amount: u64,
    #[serde(default)]
    script_public_key_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiNetworkInfo {
    #[serde(default)]
    virtual_daa_score: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasBalance {
    pub balance_sompi: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sompi: u64,
    pub script_version: u32,
    pub script_pubkey_hex: String,
    pub block_daa_score: u64,
    pub is_coinbase: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasHistoryEntry {
    pub txid: String,
    pub block_daa_score: u64,
    pub timestamp: u64,
    pub amount_sompi: i64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

impl super::SignedSubmission for KasSendResult {
    fn submission_id(&self) -> &str { &self.txid }
    fn signed_payload(&self) -> &str { &self.raw_tx_hex }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat { super::SignedPayloadFormat::Hex }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiBroadcastResponse {
    #[serde(default)]
    transaction_id: String,
    #[serde(default)]
    error: Option<String>,
}

pub struct KaspaClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl KaspaClient {
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

    pub async fn fetch_balance(&self, address: &str) -> Result<KasBalance, String> {
        let info: ApiBalance = self
            .get(&format!("/addresses/{address}/balance"))
            .await?;
        Ok(KasBalance {
            balance_sompi: info.balance,
            balance_display: format_kas(info.balance),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<KasUtxo>, String> {
        let utxos: Vec<ApiUtxo> = self.get(&format!("/addresses/{address}/utxos")).await?;
        Ok(utxos
            .into_iter()
            .filter(|u| u.address == address)
            .map(|u| {
                let amount = u.utxo_entry.amount.parse::<u64>().unwrap_or(0);
                let block_daa_score = u
                    .utxo_entry
                    .block_daa_score
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                KasUtxo {
                    txid: u.outpoint.transaction_id,
                    vout: u.outpoint.index,
                    value_sompi: amount,
                    script_version: u.utxo_entry.script_public_key.version,
                    script_pubkey_hex: u.utxo_entry.script_public_key.script_public_key,
                    block_daa_score,
                    is_coinbase: u.utxo_entry.is_coinbase.unwrap_or(false),
                }
            })
            .collect())
    }

    /// Kaspa fees are paid as a flat additive `sompi` amount and are quite
    /// small. The current network default is ~1000 sompi for a typical
    /// 2-input/2-output Schnorr P2PK send. This is a fixed-rate stub; callers
    /// can override via the `fee_sat` request field.
    pub async fn fetch_fee_rate(&self, _blocks: u32) -> u64 {
        1
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<KasHistoryEntry>, String> {
        let txs: Vec<ApiTxEntry> = self
            .get(&format!(
                "/addresses/{address}/full-transactions-page?limit=50"
            ))
            .await
            .unwrap_or_default();
        Ok(txs
            .into_iter()
            .map(|tx| {
                let owned_in: i64 = tx
                    .inputs
                    .iter()
                    .filter(|i| {
                        i.previous_outpoint_address
                            .as_deref()
                            .map(|a| a == address)
                            .unwrap_or(false)
                    })
                    .map(|i| i.previous_outpoint_amount.unwrap_or(0) as i64)
                    .sum();
                let owned_out: i64 = tx
                    .outputs
                    .iter()
                    .filter(|o| {
                        o.script_public_key_address
                            .as_deref()
                            .map(|a| a == address)
                            .unwrap_or(false)
                    })
                    .map(|o| o.amount as i64)
                    .sum();
                let net = owned_out - owned_in;
                KasHistoryEntry {
                    txid: tx.transaction_id,
                    block_daa_score: tx.accepting_block_blue_score.unwrap_or(0),
                    timestamp: tx.block_time,
                    amount_sompi: net,
                    is_incoming: net >= 0,
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
                let url = format!("{base}/transactions/{txid}");
                let tx: ApiTxEntry = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.accepting_block_blue_score.is_some();
                Ok(crate::fetch::chains::bitcoin::UtxoTxStatus {
                    txid: tx.transaction_id,
                    confirmed,
                    block_height: tx.accepting_block_blue_score,
                    block_time: if tx.block_time > 0 { Some(tx.block_time) } else { None },
                    confirmations: None,
                })
            }
        })
        .await
    }

    pub async fn fetch_chain_tip_daa_score(&self) -> Result<u64, String> {
        let info: ApiNetworkInfo = self.get("/info/network").await?;
        Ok(info.virtual_daa_score.unwrap_or(0))
    }

    /// POST a constructed transaction body to `/transactions`. Body must be
    /// the JSON shape api.kaspa.org expects: `{"transaction": {...}}`.
    pub async fn broadcast_tx_body(
        &self,
        body: serde_json::Value,
    ) -> Result<KasSendResult, String> {
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: ApiBroadcastResponse = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                if let Some(err) = resp.error {
                    return Err(format!("kaspa broadcast rejected: {err}"));
                }
                Ok(KasSendResult {
                    txid: resp.transaction_id,
                    raw_tx_hex: String::new(),
                })
            }
        })
        .await
    }
}

fn format_kas(sompi: u64) -> String {
    let whole = sompi / 100_000_000;
    let frac = sompi % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

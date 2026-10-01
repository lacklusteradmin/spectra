//! The Kaspa REST adapter (`api.kaspa.org`): balances, UTXOs, history,
//! transaction status and submission, in sompi (1e-8 KAS).

use crate::api::error::ApiError;
use serde::{Deserialize, Serialize};

use crate::api::http::{HttpClient, RetryProfile, race};

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

/// api.kaspa.org answers in snake_case. These were read as camelCase, so
/// every field fell to its default: no transaction id, no block, and no
/// address on any input or output.
#[derive(Debug, Deserialize, Default)]
struct ApiTxEntry {
    #[serde(default)]
    transaction_id: String,
    /// Unix milliseconds.
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
struct ApiTxInput {
    #[serde(default)]
    previous_outpoint_address: Option<String>,
    #[serde(default)]
    previous_outpoint_amount: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct ApiTxOutput {
    #[serde(default)]
    amount: u64,
    #[serde(default)]
    script_public_key_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasBalance {
    pub balance_sompi: u64,
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

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<KasBalance, ApiError> {
        let info: ApiBalance = self.get(&format!("/addresses/{address}/balance")).await?;
        Ok(KasBalance {
            balance_sompi: info.balance,
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<KasUtxo>, ApiError> {
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

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<KasHistoryEntry>, ApiError> {
        let txs: Vec<ApiTxEntry> = self
            .get(&format!(
                // Without resolving previous outpoints the inputs name no
                // address, so a send read as receiving its own change.
                "/addresses/{address}/full-transactions-page?limit=50&resolve_previous_outpoints=light"
            ))
            .await?;
        // Every listed transaction is in a block, so each has a time.
        let entries: Result<Vec<Option<KasHistoryEntry>>, ApiError> = txs
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
                let timestamp = crate::api::time::confirmed_history_time(
                    Some(tx.block_time),
                    &tx.transaction_id,
                )?;
                Ok((net != 0).then_some(KasHistoryEntry {
                    txid: tx.transaction_id,
                    block_daa_score: tx.accepting_block_blue_score.unwrap_or(0),
                    timestamp,
                    amount_sompi: net,
                    is_incoming: net > 0,
                }))
            })
            .collect();
        Ok(entries?.into_iter().flatten().collect())
    }

    pub async fn fetch_tx_status(
        &self,
        txid: &str,
    ) -> Result<crate::api::utxo::UtxoTxStatus, ApiError> {
        let txid = txid.to_string();
        race(&self.endpoints, |base| {
            let txid = txid.clone();
            let client = self.client.clone();
            async move {
                let url = format!("{base}/transactions/{txid}");
                let tx: ApiTxEntry = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.accepting_block_blue_score.is_some();
                Ok(crate::api::utxo::UtxoTxStatus {
                    txid: tx.transaction_id,
                    confirmed,
                    block_height: tx.accepting_block_blue_score,
                    block_time: (tx.block_time > 0).then_some(tx.block_time / 1000),
                    confirmations: None,
                })
            }
        })
        .await
    }

    /// POST a constructed transaction body to `/transactions`. Body must be
    /// the JSON shape api.kaspa.org expects: `{"transaction": {...}}`.
    pub async fn broadcast_tx_body(
        &self,
        body: serde_json::Value,
    ) -> Result<KasSendResult, ApiError> {
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: ApiBroadcastResponse = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                if let Some(err) = resp.error {
                    return Err(ApiError::Rejected(format!(
                        "kaspa broadcast rejected: {err}"
                    )));
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

#[cfg(test)]
mod history_tests {
    use super::*;

    /// The snake_case shape api.kaspa.org's `full-transactions-page` returns
    /// with `resolve_previous_outpoints=light`.
    #[test]
    fn transactions_decode_from_the_apis_own_field_names() {
        let tx: ApiTxEntry = serde_json::from_value(serde_json::json!({
            "transaction_id": "2de3", "block_time": 1_772_543_921_441u64,
            "accepting_block_blue_score": 369_401_244u64,
            "inputs": [{"previous_outpoint_address": "kaspa:me", "previous_outpoint_amount": 50_960_108_648u64}],
            "outputs": [
                {"amount": 39_800_000_000u64, "script_public_key_address": "kaspa:them"},
                {"amount": 11_160_106_612u64, "script_public_key_address": "kaspa:me"}
            ]
        }))
        .unwrap();
        assert_eq!(tx.transaction_id, "2de3");
        assert_eq!(tx.accepting_block_blue_score, Some(369_401_244));
        assert_eq!(
            tx.inputs[0].previous_outpoint_address.as_deref(),
            Some("kaspa:me")
        );
        assert_eq!(
            tx.outputs[1].script_public_key_address.as_deref(),
            Some("kaspa:me")
        );
    }
}

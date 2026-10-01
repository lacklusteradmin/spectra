//! The WhatsOnChain adapter for Bitcoin SV. A base URL is rooted at
//! `/v1/bsv/main` (or `/v1/bsv/test`); the paths below it:
//!
//! - `GET /address/{addr}/balance` → `{confirmed, unconfirmed}`
//! - `GET /address/{addr}/unspent`  → `[{tx_hash, tx_pos, value, height}]`
//! - `POST /tx/raw`                 → body `{"txhex": "..."}` returning a txid string
//!
//! `api::utxo` decides which adapter serves a request.

use crate::api::error::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::utxo::{Utxo, UtxoBalance, UtxoHistoryEntry, UtxoStatus, UtxoTxStatus};

// ── WhatsOnChain response types

#[derive(Debug, Deserialize)]
pub(crate) struct WocBalance {
    #[serde(default)]
    pub(crate) confirmed: i64,
    #[serde(default)]
    pub(crate) unconfirmed: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WocUtxo {
    pub(crate) tx_hash: String,
    pub(crate) tx_pos: u32,
    pub(crate) value: u64,
    #[serde(default)]
    pub(crate) height: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WocHistoryItem {
    pub(crate) tx_hash: String,
    #[serde(default)]
    pub(crate) height: i64,
}

/// Full tx JSON returned by WoC `/tx/hash/{hash}`. Only the fields we
/// actually use are modeled — `#[serde(default)]` lets unknown/missing
/// fields fall through cleanly.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WocTxDetail {
    #[serde(default)]
    pub(crate) txid: String,
    #[serde(default)]
    pub(crate) time: Option<u64>,
    #[serde(default)]
    pub(crate) blocktime: Option<u64>,
    #[serde(default)]
    pub(crate) blockheight: Option<i64>,
    #[serde(default)]
    pub(crate) vin: Vec<WocTxVin>,
    #[serde(default)]
    pub(crate) vout: Vec<WocTxVout>,
}

/// An input names the output it spends; WoC gives neither its address nor
/// its value.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WocTxVin {
    #[serde(default)]
    pub(crate) txid: String,
    #[serde(default)]
    pub(crate) vout: u32,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct WocTxVout {
    /// BSV amount as a float (WoC convention). Convert ×1e8 for sats.
    #[serde(default)]
    pub(crate) value: f64,
    #[serde(default)]
    pub(crate) n: u32,
    #[serde(default)]
    #[serde(rename = "scriptPubKey")]
    pub(crate) script_pub_key: Option<WocTxVoutScriptPubKey>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct WocTxVoutScriptPubKey {
    #[serde(default)]
    pub(crate) addresses: Option<Vec<String>>,
}

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsonchainSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ── Client

pub struct WhatsonchainClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl WhatsonchainClient {
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
}
// BSV fetch paths (WhatsOnChain REST): balance, UTXOs, history (with per-tx
// enrichment), and tx status.

impl WhatsonchainClient {
    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, ApiError> {
        let balance = self.fetch_balance(address).await?;
        if balance.confirmed_sats > 0 || balance.unconfirmed_sats != 0 {
            return Ok(true);
        }
        // Only the history index is needed; never enrich every transaction.
        let list: Vec<WocHistoryItem> = self.get(&format!("/address/{address}/history")).await?;
        Ok(!list.is_empty())
    }

    /// The confirmed balance and the mempool's net change to it.
    pub async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, ApiError> {
        let bal: WocBalance = self.get(&format!("/address/{address}/balance")).await?;
        Ok(UtxoBalance {
            confirmed_sats: bal.confirmed.max(0) as u64,
            unconfirmed_sats: bal.unconfirmed,
        })
    }

    /// Unspent outputs, the mempool's included.
    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, ApiError> {
        let utxos: Vec<WocUtxo> = self.get(&format!("/address/{address}/unspent")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| Utxo {
                txid: u.tx_hash,
                vout: u.tx_pos,
                value: u.value,
                status: UtxoStatus {
                    confirmed: u.height > 0,
                    block_height: (u.height > 0).then_some(u.height as u64),
                },
            })
            .collect())
    }

    /// Fetch recent transactions for `address` via WhatsOnChain.
    ///
    /// WoC exposes `/address/{addr}/history` as a flat list of
    /// `{tx_hash, height}` entries. To populate amounts and timestamps we
    /// issue a sequential `/tx/hash/{hash}` fetch per entry.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<UtxoHistoryEntry>, ApiError> {
        let list: Vec<WocHistoryItem> = self.get(&format!("/address/{address}/history")).await?;

        let mut details = Vec::with_capacity(list.len());
        for item in list {
            // Every transaction is needed to know which outputs are the
            // address's, so one that cannot be read fails the whole history
            // rather than turning a later spend of its outputs into a receipt.
            let tx: WocTxDetail = self.get(&format!("/tx/hash/{}", item.tx_hash)).await?;
            details.push((item, tx));
        }
        bsv_history_from_details(details, address)
    }

    /// Fetch confirmation status for a single txid via WoC `/tx/hash/{txid}`.
    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, ApiError> {
        let txid = txid.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let txid = txid.clone();
            async move {
                let url = format!("{}/tx/hash/{}", base.trim_end_matches('/'), txid);
                let tx: WocTxDetail = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.blockheight.map(|h| h >= 0).unwrap_or(false);
                let block_height = tx.blockheight.filter(|&h| h >= 0).map(|h| h as u64);
                let block_time = tx.blocktime.or(tx.time);
                Ok(UtxoTxStatus {
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

/// Each transaction's net effect on `address`: the outputs paying it less
/// the outputs of its own that the transaction spends.
///
/// WoC's inputs carry no address or value, only the output they spend, so an
/// input is the address's when it spends an output the address received in
/// this same history. Without that, no input was ever recognized: a send
/// with change read as receiving the change, and one without read as 0.
fn bsv_history_from_details(
    details: Vec<(WocHistoryItem, WocTxDetail)>,
    address: &str,
) -> Result<Vec<UtxoHistoryEntry>, ApiError> {
    // WoC answers in floating-point BTC. Its shortest spelling is what it
    // said; more than eight places is not a satoshi amount and reads as 0.
    let sats = |value: f64| {
        crate::decimal::from_f64(value)
            .and_then(|btc| crate::decimal::to_units(&btc, 8))
            .and_then(|sats| i64::try_from(sats).ok())
            .unwrap_or(0)
    };
    let pays_address = |vout: &WocTxVout| {
        vout.script_pub_key
            .as_ref()
            .and_then(|spk| spk.addresses.as_ref())
            .is_some_and(|addrs| addrs.iter().any(|a| a == address))
    };
    let owned: std::collections::HashMap<(String, u32), i64> = details
        .iter()
        .flat_map(|(item, tx)| {
            let txid = if tx.txid.is_empty() {
                &item.tx_hash
            } else {
                &tx.txid
            };
            tx.vout
                .iter()
                .filter(|vout| pays_address(vout))
                .map(move |vout| ((txid.clone(), vout.n), sats(vout.value)))
        })
        .collect();
    let entries: Result<Vec<Option<UtxoHistoryEntry>>, ApiError> = details
        .into_iter()
        .map(|(item, tx)| {
            let received: i64 = tx
                .vout
                .iter()
                .filter(|v| pays_address(v))
                .map(|v| sats(v.value))
                .sum();
            let spent: i64 = tx
                .vin
                .iter()
                .filter_map(|vin| owned.get(&(vin.txid.clone(), vin.vout)))
                .sum();
            let net_sats = received - spent;
            let block_height = Some(tx.blockheight.unwrap_or(item.height))
                .filter(|height| *height > 0)
                .map(|height| height as u64);
            let block_time = crate::api::time::history_time(
                block_height.is_some(),
                tx.blocktime.or(tx.time),
                &item.tx_hash,
            )?;
            Ok((net_sats != 0).then_some(UtxoHistoryEntry {
                txid: item.tx_hash,
                confirmed: block_height.is_some(),
                block_height,
                block_time,
                net_sats,
                fee_sats: None,
            }))
        })
        .collect();
    Ok(entries?.into_iter().flatten().collect())
}

impl WhatsonchainClient {
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<WhatsonchainSendResult, ApiError> {
        let hex = hex_tx.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let hex = hex.clone();
            let url = format!("{}/tx/raw", base.trim_end_matches('/'));
            async move {
                // WhatsOnChain /tx/raw expects `{"txhex": "<hex>"}` and
                // responds with a bare JSON string containing the txid.
                let raw_tx_hex = hex.clone();
                let body = json!({ "txhex": hex });
                let txid: String = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                Ok(WhatsonchainSendResult {
                    txid: txid.trim().trim_matches('"').to_string(),
                    raw_tx_hex,
                })
            }
        })
        .await
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "1KGHhLTQaPr4LErrvbAuGE62yPpDoRwrob";
    const THEM: &str = "14oJKtCNjEM7Sx4eFReaiHfqRFVVLDNBMG";

    fn detail(
        txid: &str,
        vin: serde_json::Value,
        vout: serde_json::Value,
    ) -> (WocHistoryItem, WocTxDetail) {
        let tx: WocTxDetail = serde_json::from_value(
            serde_json::json!({"txid": txid, "vin": vin, "vout": vout, "blocktime": 1}),
        )
        .unwrap();
        (
            WocHistoryItem {
                tx_hash: txid.into(),
                height: 1,
            },
            tx,
        )
    }
    fn out(n: u32, value: f64, to: &str) -> serde_json::Value {
        serde_json::json!({"value": value, "n": n, "scriptPubKey": {"addresses": [to]}})
    }

    /// WoC `/tx/hash` shapes: inputs name only the output they spend.
    #[test]
    fn a_send_spends_the_addresses_own_outputs() {
        let details = vec![
            detail(
                "fund",
                serde_json::json!([{"txid": "x", "vout": 0}]),
                serde_json::json!([out(0, 1.0, ME)]),
            ),
            detail(
                "send",
                serde_json::json!([{"txid": "fund", "vout": 0}]),
                serde_json::json!([out(0, 0.6, THEM), out(1, 0.3999, ME)]),
            ),
            detail(
                "sweep",
                serde_json::json!([{"txid": "send", "vout": 1}]),
                serde_json::json!([out(0, 0.3998, THEM)]),
            ),
        ];
        let entries = bsv_history_from_details(details, ME).unwrap();
        let got: Vec<(&str, i64, bool)> = entries
            .iter()
            .map(|e| (e.txid.as_str(), e.net_sats, e.net_sats > 0))
            .collect();
        assert_eq!(
            got,
            [
                ("fund", 100_000_000, true),
                ("send", -60_010_000, false),
                ("sweep", -39_990_000, false)
            ]
        );
    }
}

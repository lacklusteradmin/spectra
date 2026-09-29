//! The BlockCypher REST adapter. A base URL names its coin and network, as in
//! `https://api.blockcypher.com/v1/doge/main`; the paths below it are the
//! same for every coin. `api::utxo` decides which adapter serves a request.

use serde::Deserialize;

use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::utxo::{FeeRate, Utxo, UtxoBalance, UtxoHistoryEntry, UtxoStatus, UtxoTxStatus};

// ── BlockCypher response types

/// Response from GET /addrs/{address}/balance
#[derive(Debug, Deserialize)]
struct BlockcypherBalance {
    /// Confirmed balance in the coin's smallest unit.
    balance: u64,
    #[serde(default)]
    unconfirmed_balance: i64,
}

/// Response from GET /addrs/{address}?unspentOnly=true
#[derive(Debug, Deserialize)]
struct BlockcypherAddress {
    #[serde(default)]
    txrefs: Vec<BlockcypherTxref>,
    /// Refs of transactions still in the mempool: no block and no time.
    #[serde(default)]
    unconfirmed_txrefs: Vec<BlockcypherTxref>,
}

#[derive(Debug, Deserialize)]
struct BlockcypherTxref {
    tx_hash: String,
    #[serde(default)]
    tx_output_n: i32,
    #[serde(default)]
    tx_input_n: i32,
    value: i64,
    #[serde(default)]
    block_height: i64,
    #[serde(default)]
    spent: bool,
    confirmed: Option<String>,
}

/// Response from GET {base}: the chain's fee levels, per 1000 bytes.
#[derive(Debug, Deserialize)]
struct BlockcypherChain {
    high_fee_per_kb: f64,
    medium_fee_per_kb: f64,
    low_fee_per_kb: f64,
}

// ── Client

pub struct BlockcypherClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl BlockcypherClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, String> {
        self.client.get_path(&self.endpoints, path).await
    }
}

impl BlockcypherClient {
    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, String> {
        #[derive(Deserialize)]
        struct Activity {
            n_tx: u64,
            unconfirmed_n_tx: u64,
        }
        let info: Activity = self.get(&format!("/addrs/{address}/balance")).await?;
        Ok(info.n_tx > 0 || info.unconfirmed_n_tx > 0)
    }

    /// The confirmed balance and the mempool's net change to it.
    pub async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, String> {
        let info: BlockcypherBalance = self.get(&format!("/addrs/{address}/balance")).await?;
        Ok(UtxoBalance {
            confirmed_sats: info.balance,
            unconfirmed_sats: info.unconfirmed_balance,
        })
    }

    /// Unspent outputs, the mempool's included.
    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, String> {
        let info: BlockcypherAddress = self
            .get(&format!("/addrs/{address}?unspentOnly=true"))
            .await?;
        Ok(info
            .txrefs
            .into_iter()
            .chain(info.unconfirmed_txrefs)
            .filter(|r| r.tx_output_n >= 0 && !r.spent && r.value >= 0)
            .map(|r| {
                let block_height = (r.block_height > 0).then_some(r.block_height as u64);
                Utxo {
                    txid: r.tx_hash,
                    vout: r.tx_output_n as u32,
                    value: r.value as u64,
                    status: UtxoStatus {
                        confirmed: block_height.is_some(),
                        block_height,
                    },
                }
            })
            .collect())
    }

    /// The most recent 50 transactions touching `address`, newest first.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<UtxoHistoryEntry>, String> {
        let info: BlockcypherAddress = self.get(&format!("/addrs/{address}?limit=50")).await?;
        history_from_txrefs(info.unconfirmed_txrefs.into_iter().chain(info.txrefs))
    }

    /// The fee rate for a `confirmation_target`, in sat/vB: BlockCypher's
    /// high level within two blocks, medium within six, low beyond.
    pub async fn fetch_fee_rate(&self, confirmation_target: u32) -> Result<FeeRate, String> {
        let chain: BlockcypherChain = self.get("").await?;
        let per_kb = match confirmation_target {
            0..=2 => chain.high_fee_per_kb,
            3..=6 => chain.medium_fee_per_kb,
            _ => chain.low_fee_per_kb,
        };
        if !per_kb.is_finite() || per_kb <= 0.0 {
            return Err("BlockCypher has no fee estimate".into());
        }
        Ok(FeeRate {
            sats_per_vbyte: per_kb / 1000.0,
        })
    }

    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, String> {
        #[derive(Deserialize)]
        struct BlockcypherTx {
            hash: String,
            block_height: Option<i64>,
            confirmations: Option<u64>,
            confirmed: Option<String>,
        }
        let tx: BlockcypherTx = self.get(&format!("/txs/{txid}")).await?;
        let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
        Ok(UtxoTxStatus {
            txid: tx.hash,
            confirmed,
            block_height: tx.block_height.map(|h| if h > 0 { h as u64 } else { 0 }),
            block_time: blockcypher_time(tx.confirmed.as_deref()),
            confirmations: tx.confirmations,
        })
    }

    /// Submit a signed transaction; BlockCypher answers with its hash.
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Pushed {
            tx: PushedTx,
        }
        #[derive(Deserialize)]
        struct PushedTx {
            hash: String,
        }
        let body = serde_json::json!({ "tx": hex_tx });
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let url = format!("{}/txs/push", base.trim_end_matches('/'));
            async move {
                let pushed: Pushed = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                Ok(pushed.tx.hash)
            }
        })
        .await
    }
}

/// One entry per transaction, netting its refs.
///
/// BlockCypher lists a ref per input the address funded (`tx_input_n` ≥ 0)
/// and per output paying it (`tx_input_n` = -1), so a transaction appears once
/// per leg.
fn history_from_txrefs(
    refs: impl IntoIterator<Item = BlockcypherTxref>,
) -> Result<Vec<UtxoHistoryEntry>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut legs: std::collections::HashMap<String, Vec<BlockcypherTxref>> =
        std::collections::HashMap::new();
    for r in refs {
        if !legs.contains_key(&r.tx_hash) {
            order.push(r.tx_hash.clone());
        }
        legs.entry(r.tx_hash.clone()).or_default().push(r);
    }
    let mut entries = Vec::new();
    for hash in order {
        let refs = &legs[&hash];
        let net: i64 = refs
            .iter()
            .map(|r| if r.tx_input_n < 0 { r.value } else { -r.value })
            .sum();
        if net == 0 {
            continue;
        }
        let block_height = refs
            .iter()
            .map(|r| r.block_height)
            .max()
            .filter(|height| *height > 0)
            .map(|height| height as u64);
        let block_time = crate::api::time::history_time(
            block_height.is_some(),
            refs.iter()
                .find_map(|r| blockcypher_time(r.confirmed.as_deref())),
            &hash,
        )?;
        entries.push(UtxoHistoryEntry {
            txid: hash,
            confirmed: block_height.is_some(),
            block_height,
            block_time,
            net_sats: net,
            fee_sats: None,
        });
    }
    // Unconfirmed first, then newest block first.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.block_height.unwrap_or(u64::MAX)));
    Ok(entries)
}

/// A BlockCypher RFC 3339 time as Unix seconds, or `None` when there is none
/// or it does not parse. This had its own parser, which answered 0 — the
/// Unix epoch — for anything it could not read.
fn blockcypher_time(s: Option<&str>) -> Option<u64> {
    crate::api::time::parse_iso8601_timestamp(s?)
        .filter(|t| *t > 0.0)
        .map(|t| t as u64)
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn leg(
        hash: &str,
        input: i32,
        value: i64,
        height: i64,
        time: Option<&str>,
    ) -> BlockcypherTxref {
        serde_json::from_value(serde_json::json!({
            "tx_hash": hash, "tx_input_n": input, "tx_output_n": if input < 0 { 0 } else { -1 },
            "value": value, "block_height": height, "confirmed": time
        }))
        .unwrap()
    }

    /// BlockCypher `/addrs` refs: one per leg, unconfirmed ones without a time.
    #[test]
    fn a_transactions_legs_net_into_one_entry() {
        let t = Some("2026-09-22T19:10:21Z");
        let entries = history_from_txrefs([
            leg("pending", -1, 500, -1, None),
            leg("send", 0, 1_000_000_000, 6_385_234, t),
            leg("send", -1, 400_000_000, 6_385_234, t),
            leg("receive", -1, 951_727_163, 6_385_200, t),
        ])
        .unwrap();
        let got: Vec<(&str, i64, Option<u64>)> = entries
            .iter()
            .map(|e| (e.txid.as_str(), e.net_sats, e.block_time))
            .collect();
        assert_eq!(
            got,
            [
                ("pending", 500, None),
                ("send", -600_000_000, Some(1_790_104_221)),
                ("receive", 951_727_163, Some(1_790_104_221)),
            ]
        );
        assert!(history_from_txrefs([leg("bad", -1, 1, 5, None)]).is_err());
    }
}

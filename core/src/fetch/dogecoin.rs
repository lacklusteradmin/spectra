//! Dogecoin chain client.
//!
//! Uses BlockCypher REST API (the only configured endpoint).
//! Endpoint base: https://api.blockcypher.com/v1/doge/main
//! Signing uses secp256k1 / P2PKH (Dogecoin does not support SegWit).
//! Network params: version byte 0x1e (addresses start with 'D').

use serde::{Deserialize, Serialize};

use crate::fetch::http::HttpClient;

// ── BlockCypher response types

/// Response from GET /addrs/{address}/balance
#[derive(Debug, Deserialize)]
struct BlockcypherBalance {
    /// Confirmed balance in koinus (1 DOGE = 100_000_000 koinus).
    balance: u64,
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
    confirmations: u32,
    #[serde(default)]
    block_height: i64,
    #[serde(default)]
    spent: bool,
    confirmed: Option<String>,
}

// ── Public result types

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
    /// `None` while the transaction is unconfirmed.
    pub timestamp: Option<u64>,
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

// ── Client

pub struct DogecoinClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl DogecoinClient {
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

impl DogecoinClient {
    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, String> {
        #[derive(Deserialize)]
        struct Activity {
            n_tx: u64,
            unconfirmed_n_tx: u64,
        }
        let info: Activity = self.get(&format!("/addrs/{address}/balance")).await?;
        Ok(info.n_tx > 0 || info.unconfirmed_n_tx > 0)
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<DogeBalance, String> {
        let info: BlockcypherBalance = self.get(&format!("/addrs/{address}/balance")).await?;
        Ok(DogeBalance {
            balance_koin: info.balance,
            balance_display: format_doge(info.balance),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<DogeUtxo>, String> {
        let info: BlockcypherAddress = self
            .get(&format!("/addrs/{address}?unspentOnly=true"))
            .await?;
        Ok(info
            .txrefs
            .into_iter()
            .filter(|r| r.tx_output_n >= 0 && !r.spent && r.value >= 0)
            .map(|r| DogeUtxo {
                txid: r.tx_hash,
                vout: r.tx_output_n as u32,
                value_koin: r.value as u64,
                confirmations: r.confirmations,
            })
            .collect())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<DogeHistoryEntry>, String> {
        let info: BlockcypherAddress = self.get(&format!("/addrs/{address}?limit=50")).await?;
        doge_history_from_txrefs(info.unconfirmed_txrefs.into_iter().chain(info.txrefs))
    }

    pub async fn fetch_tx_status(
        &self,
        txid: &str,
    ) -> Result<crate::fetch::bitcoin::UtxoTxStatus, String> {
        #[derive(Deserialize)]
        struct BlockcypherTx {
            hash: String,
            block_height: Option<i64>,
            confirmations: Option<u64>,
            confirmed: Option<String>,
        }
        let tx: BlockcypherTx = self.get(&format!("/txs/{txid}")).await?;
        let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
        Ok(crate::fetch::bitcoin::UtxoTxStatus {
            txid: tx.hash,
            confirmed,
            block_height: tx.block_height.map(|h| if h > 0 { h as u64 } else { 0 }),
            block_time: blockcypher_time(tx.confirmed.as_deref()),
            confirmations: tx.confirmations,
        })
    }
}

/// One entry per transaction, netting its refs.
///
/// BlockCypher lists a ref per input the address funded (`tx_input_n` ≥ 0)
/// and per output paying it (`tx_input_n` = -1), so a transaction appears once
/// per leg.
fn doge_history_from_txrefs(
    refs: impl IntoIterator<Item = BlockcypherTxref>,
) -> Result<Vec<DogeHistoryEntry>, String> {
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
            .unwrap_or(0)
            .max(0) as u64;
        let timestamp = super::history_time(
            block_height > 0,
            refs.iter()
                .find_map(|r| blockcypher_time(r.confirmed.as_deref())),
            &hash,
        )?;
        entries.push(DogeHistoryEntry {
            txid: hash,
            block_height,
            timestamp,
            amount_koin: net,
            fee_koin: 0,
            is_incoming: net > 0,
        });
    }
    // Unconfirmed first, then newest block first.
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(if entry.block_height == 0 {
            u64::MAX
        } else {
            entry.block_height
        })
    });
    Ok(entries)
}

/// A BlockCypher RFC 3339 time as Unix seconds, or `None` when there is none
/// or it does not parse. This had its own parser, which answered 0 — the
/// Unix epoch — for anything it could not read.
fn blockcypher_time(s: Option<&str>) -> Option<u64> {
    super::history::parse_iso8601_timestamp(s?)
        .filter(|t| *t > 0.0)
        .map(|t| t as u64)
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
        let entries = doge_history_from_txrefs([
            leg("pending", -1, 500, -1, None),
            leg("send", 0, 1_000_000_000, 6_385_234, t),
            leg("send", -1, 400_000_000, 6_385_234, t),
            leg("receive", -1, 951_727_163, 6_385_200, t),
        ])
        .unwrap();
        let got: Vec<(&str, i64, Option<u64>)> = entries
            .iter()
            .map(|e| (e.txid.as_str(), e.amount_koin, e.timestamp))
            .collect();
        assert_eq!(
            got,
            [
                ("pending", 500, None),
                ("send", -600_000_000, Some(1_790_104_221)),
                ("receive", 951_727_163, Some(1_790_104_221)),
            ]
        );
        assert!(doge_history_from_txrefs([leg("bad", -1, 1, 5, None)]).is_err());
    }
}

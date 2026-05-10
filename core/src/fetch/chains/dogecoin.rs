//! Dogecoin chain client.
//!
//! Uses BlockCypher REST API (the only configured endpoint).
//! Endpoint base: https://api.blockcypher.com/v1/doge/main
//! Signing uses secp256k1 / P2PKH (Dogecoin does not support SegWit).
//! Network params: version byte 0x1e (addresses start with 'D').

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// BlockCypher response types
// ----------------------------------------------------------------

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

impl super::SignedSubmission for DogeSendResult {
    fn submission_id(&self) -> &str { &self.txid }
    fn signed_payload(&self) -> &str { &self.raw_tx_hex }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat { super::SignedPayloadFormat::Hex }
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

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

impl DogecoinClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<DogeBalance, String> {
        let info: BlockcypherBalance = self
            .get(&format!("/addrs/{address}/balance"))
            .await?;
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
        let info: BlockcypherAddress = self
            .get(&format!("/addrs/{address}?limit=50"))
            .await?;
        let mut seen = std::collections::HashSet::new();
        let mut entries: Vec<DogeHistoryEntry> = info
            .txrefs
            .into_iter()
            .filter(|r| seen.insert(r.tx_hash.clone()))
            .map(|r| {
                let is_incoming = r.tx_input_n < 0;
                DogeHistoryEntry {
                    txid: r.tx_hash,
                    block_height: if r.block_height > 0 { r.block_height as u64 } else { 0 },
                    timestamp: parse_blockcypher_time(r.confirmed.as_deref()),
                    amount_koin: if is_incoming { r.value } else { -r.value.abs() },
                    fee_koin: 0,
                    is_incoming,
                }
            })
            .collect();
        entries.sort_by(|a, b| b.block_height.cmp(&a.block_height));
        Ok(entries)
    }

    pub async fn fetch_tx_status(
        &self,
        txid: &str,
    ) -> Result<crate::fetch::chains::bitcoin::UtxoTxStatus, String> {
        #[derive(Deserialize)]
        struct BlockcypherTx {
            hash: String,
            block_height: Option<i64>,
            confirmations: Option<u64>,
            confirmed: Option<String>,
        }
        let tx: BlockcypherTx = self.get(&format!("/txs/{txid}")).await?;
        let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
        Ok(crate::fetch::chains::bitcoin::UtxoTxStatus {
            txid: tx.hash,
            confirmed,
            block_height: tx.block_height.map(|h| if h > 0 { h as u64 } else { 0 }),
            block_time: Some(parse_blockcypher_time(tx.confirmed.as_deref())),
            confirmations: tx.confirmations,
        })
    }
}

/// Parse a BlockCypher RFC 3339 timestamp string to a Unix timestamp.
/// Returns 0 on parse failure — timestamps are display-only.
fn parse_blockcypher_time(s: Option<&str>) -> u64 {
    fn inner(s: &str) -> Option<u64> {
        let b = s.as_bytes();
        if b.len() < 19 { return None; }
        let year:  i64 = std::str::from_utf8(&b[0..4]).ok()?.parse().ok()?;
        let month: i64 = std::str::from_utf8(&b[5..7]).ok()?.parse().ok()?;
        let day:   i64 = std::str::from_utf8(&b[8..10]).ok()?.parse().ok()?;
        let hour:  i64 = std::str::from_utf8(&b[11..13]).ok()?.parse().ok()?;
        let min:   i64 = std::str::from_utf8(&b[14..16]).ok()?.parse().ok()?;
        let sec:   i64 = std::str::from_utf8(&b[17..19]).ok()?.parse().ok()?;
        // Days since Unix epoch using civil calendar (Euclidean algorithm).
        let m_adj = if month <= 2 { month + 9 } else { month - 3 };
        let y_adj = if month <= 2 { year - 1 } else { year };
        let era = y_adj.div_euclid(400);
        let yoe = y_adj - era * 400;
        let doy = (153 * m_adj + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe - 719468;
        let ts = days * 86400 + hour * 3600 + min * 60 + sec;
        if ts < 0 { None } else { Some(ts as u64) }
    }
    s.and_then(|s| if s.is_empty() { None } else { inner(s) }).unwrap_or(0)
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

//! Cardano chain client.
//!
//! Uses the Koios REST API (api.koios.rest/api/v1) for balance,
//! history, UTXOs, and protocol params.
//! Cardano transactions are encoded in CBOR (cardano-multiplatform-lib
//! is too heavy; we use a minimal handwritten CBOR encoder for simple
//! ADA-only transfers).
//! Signing uses Ed25519 (ed25519-dalek).

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoBalance {
    /// Lovelace (1 ADA = 1_000_000 lovelace).
    pub lovelace: u64,
    pub ada_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoUtxo {
    pub tx_hash: String,
    pub tx_index: u32,
    pub lovelace: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoHistoryEntry {
    pub txid: String,
    pub block: String,
    pub block_time: u64,
    pub is_incoming: bool,
    pub amount_lovelace: i64,
    pub fee_lovelace: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoSendResult {
    pub txid: String,
    /// CBOR hex of the signed transaction — stored for rebroadcast.
    pub cbor_hex: String,
}

impl super::SignedSubmission for CardanoSendResult {
    fn submission_id(&self) -> &str { &self.txid }
    fn signed_payload(&self) -> &str { &self.cbor_hex }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat { super::SignedPayloadFormat::Hex }
}

// ----------------------------------------------------------------
// Koios response types (shared within the chain module)
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosAddressInfo {
    pub(crate) balance: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosUtxo {
    pub(crate) tx_hash: String,
    pub(crate) tx_index: u32,
    pub(crate) value: String,
    #[serde(default)]
    pub(crate) is_spent: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosTxRef {
    pub(crate) tx_hash: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosTxInfo {
    pub(crate) tx_hash: String,
    #[serde(default)]
    pub(crate) block_height: u64,
    #[serde(default)]
    pub(crate) tx_timestamp: u64,
    #[serde(default)]
    pub(crate) total_output: String,
    #[serde(default)]
    pub(crate) fee: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct CardanoClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    #[allow(dead_code)]
    pub(crate) api_key: String,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl CardanoClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>, api_key: String) -> Self {
        Self {
            endpoints,
            api_key,
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

    pub(crate) async fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body_val = serde_json::to_value(body).map_err(|e| e.to_string())?;
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body_val = body_val.clone();
            async move { client.post_json(&url, &body_val, RetryProfile::ChainRead).await }
        })
        .await
    }
}

impl CardanoClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<CardanoBalance, String> {
        #[derive(Serialize)]
        struct Req<'a> { #[serde(rename = "_addresses")] addresses: &'a [&'a str] }
        let resp: Vec<KoiosAddressInfo> = self
            .post("/address_info", &Req { addresses: &[address] })
            .await?;
        let lovelace: u64 = resp
            .into_iter()
            .next()
            .and_then(|r| r.balance.parse().ok())
            .unwrap_or(0);
        Ok(CardanoBalance {
            lovelace,
            ada_display: format_ada(lovelace),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<CardanoUtxo>, String> {
        #[derive(Serialize)]
        struct Req<'a> { #[serde(rename = "_addresses")] addresses: &'a [&'a str] }
        let utxos: Vec<KoiosUtxo> = self
            .post("/address_utxos", &Req { addresses: &[address] })
            .await?;
        Ok(utxos
            .into_iter()
            .filter(|u| !u.is_spent)
            .map(|u| CardanoUtxo {
                tx_hash: u.tx_hash,
                tx_index: u.tx_index,
                lovelace: u.value.parse().unwrap_or(0),
            })
            .collect())
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<CardanoHistoryEntry>, String> {
        #[derive(Serialize)]
        struct AddrReq<'a> { #[serde(rename = "_addresses")] addresses: &'a [&'a str] }
        #[derive(Serialize)]
        struct TxReq { #[serde(rename = "_tx_hashes")] tx_hashes: Vec<String> }

        let tx_refs: Vec<KoiosTxRef> = self
            .post("/address_txs", &AddrReq { addresses: &[address] })
            .await?;

        let hashes: Vec<String> = tx_refs.iter().take(20).map(|r| r.tx_hash.clone()).collect();
        if hashes.is_empty() { return Ok(vec![]); }

        let tx_infos: Vec<KoiosTxInfo> = self
            .post("/tx_info", &TxReq { tx_hashes: hashes })
            .await
            .unwrap_or_default();

        let mut entries: Vec<CardanoHistoryEntry> = tx_infos
            .into_iter()
            .map(|tx| {
                let total: i64 = tx.total_output.parse().unwrap_or(0);
                let fee: u64 = tx.fee.parse().unwrap_or(0);
                CardanoHistoryEntry {
                    txid: tx.tx_hash,
                    block: tx.block_height.to_string(),
                    block_time: tx.tx_timestamp,
                    is_incoming: total > 0,
                    amount_lovelace: total,
                    fee_lovelace: fee,
                }
            })
            .collect();
        entries.sort_by(|a, b| b.block_time.cmp(&a.block_time));
        Ok(entries)
    }

    /// Fetch current slot from the latest block.
    pub async fn fetch_latest_slot(&self) -> Result<u64, String> {
        #[derive(Deserialize)]
        struct Tip { abs_slot: u64 }
        let tips: Vec<Tip> = self.get("/tip").await?;
        tips.into_iter().next().map(|t| t.abs_slot).ok_or_else(|| "tip: empty response".to_string())
    }
}

fn format_ada(lovelace: u64) -> String {
    let whole = lovelace / 1_000_000;
    let frac = lovelace % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

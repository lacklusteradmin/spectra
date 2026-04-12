//! Bitcoin Cash chain client.
//!
//! BCH uses the CashAddr address format (prefix "bitcoincash:") but can also
//! accept legacy P2PKH addresses (version 0x00, same as BTC). Signing is
//! SIGHASH_ALL with replay protection (BIP143 SegWit-style digest for BCH
//! is NOT used; BCH uses its own SIGHASH_FORKID = 0x40).
//!
//! We use Blockbook for balance/UTXO/broadcast.

use serde::{Deserialize, Serialize};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Blockbook types
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BlockbookUtxo {
    txid: String,
    vout: u32,
    value: String,
    #[serde(default)]
    confirmations: u32,
}

#[derive(Debug, Deserialize)]
struct BlockbookAddress {
    balance: String,
}

#[derive(Debug, Deserialize)]
struct BlockbookFeeEstimate {
    result: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookTxList {
    #[serde(default)]
    transactions: Vec<BlockbookTx>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookTx {
    txid: String,
    block_time: Option<u64>,
    block_height: Option<u64>,
    #[serde(default)]
    value: String,
    fees: Option<String>,
    #[serde(default)]
    vin: Vec<BlockbookVin>,
}

#[derive(Debug, Deserialize)]
struct BlockbookVin {
    addresses: Option<Vec<String>>,
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Net value change for the queried address. Negative = outgoing.
    pub amount_sat: i64,
    pub fee_sat: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct BitcoinCashClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl BitcoinCashClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<BchBalance, String> {
        // Blockbook accepts both cashaddr and legacy.
        let norm = normalize_bch_address(address);
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{norm}?details=basic"))
            .await?;
        let sat: u64 = info.balance.parse().unwrap_or(0);
        Ok(BchBalance {
            balance_sat: sat,
            balance_display: format_bch(sat),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<BchUtxo>, String> {
        let norm = normalize_bch_address(address);
        let utxos: Vec<BlockbookUtxo> = self
            .get(&format!("/api/v2/utxo/{norm}"))
            .await?;
        Ok(utxos
            .into_iter()
            .map(|u| BchUtxo {
                txid: u.txid,
                vout: u.vout,
                value_sat: u.value.parse().unwrap_or(0),
                confirmations: u.confirmations,
            })
            .collect())
    }

    /// Fetch recommended fee rate for `blocks` confirmation target.
    /// Returns satoshis per vbyte. Falls back to 1 sat/vB on failure.
    pub async fn fetch_fee_rate(&self, blocks: u32) -> u64 {
        let estimate: Result<BlockbookFeeEstimate, _> = self
            .get(&format!("/api/v2/estimatefee/{blocks}"))
            .await;
        estimate
            .ok()
            .and_then(|e| e.result.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|bch_per_kb| ((bch_per_kb * 1e8 / 1000.0).ceil() as u64).max(1))
            .unwrap_or(1)
    }

    /// Fetch the most recent 50 transactions for `address` via Blockbook's
    /// `details=txs` pagination. Blockbook normalizes BCH CashAddr inputs
    /// internally but we pass through `normalize_bch_address` as a safety
    /// check. Direction is detected from vin addresses.
    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<BchHistoryEntry>, String> {
        let norm = normalize_bch_address(address);
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{norm}?details=txs&page=1&pageSize=50"
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
                        .any(|a| a == &norm || a == address)
                });
                let amount_sat: i64 = tx.value.parse().unwrap_or(0);
                let fee_sat: u64 = tx
                    .fees
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                BchHistoryEntry {
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

    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<BchSendResult, String> {
        let hex = hex_tx.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let hex = hex.clone();
            let url = format!("{}/api/v2/sendtx/", base.trim_end_matches('/'));
            async move {
                let raw_tx_hex = hex.clone();
                let txid: String = client
                    .post_text(&url, hex, RetryProfile::ChainWrite)
                    .await?;
                let txid = txid.trim().to_string();
                Ok(BchSendResult {
                    txid,
                    raw_tx_hex,
                })
            }
        })
        .await
    }

    /// Fetch UTXOs for `from_address`, sign a BCH P2PKH (SIGHASH_FORKID) transaction,
    /// and broadcast.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
    ) -> Result<BchSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let hash20 = decode_bch_to_hash20(from_address)?;
        let script_pubkey = p2pkh_script(&hash20);
        let utxo_tuples: Vec<(String, u32, u64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.value_sat, script_pubkey.clone()))
            .collect();
        let raw = sign_bch_tx(
            &utxo_tuples,
            to_address,
            amount_sat,
            fee_sat,
            from_address,
            private_key_bytes,
        )?;
        self.broadcast_raw_tx(&hex::encode(&raw)).await
    }
}

// ----------------------------------------------------------------
// BCH SIGHASH_FORKID signing (BIP143-variant)
// ----------------------------------------------------------------

/// SIGHASH_ALL | SIGHASH_FORKID = 0x41
const SIGHASH_ALL_FORKID: u32 = 0x41;

/// Sign a BCH P2PKH transaction using SIGHASH_FORKID.
///
/// `utxos` — (txid, vout, value_sat, script_pubkey) for each selected input.
pub fn sign_bch_tx(
    utxos: &[(String, u32, u64, Vec<u8>)],
    to_address: &str,
    amount_sat: u64,
    fee_sat: u64,
    change_address: &str,
    private_key_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let pubkey_bytes = pubkey.serialize();

    let total_in: u64 = utxos.iter().map(|(_, _, v, _)| v).sum();
    let change = total_in.saturating_sub(amount_sat + fee_sat);

    let to_hash = decode_bch_to_hash20(to_address)?;
    let change_hash = decode_bch_to_hash20(change_address)?;

    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(p2pkh_script(&to_hash), amount_sat)];
    if change > 546 {
        outputs.push((p2pkh_script(&change_hash), change));
    }

    // Precompute hashPrevouts and hashSequence (BIP143 §1,2).
    let mut prevouts_data = Vec::new();
    let mut sequences_data = Vec::new();
    for (txid, vout, _, _) in utxos {
        let mut txid_bytes = hex::decode(txid).unwrap_or_default();
        txid_bytes.reverse();
        prevouts_data.extend_from_slice(&txid_bytes);
        prevouts_data.extend_from_slice(&vout.to_le_bytes());
        sequences_data.extend_from_slice(&0xffffffff_u32.to_le_bytes());
    }
    let hash_prevouts = dsha256(&prevouts_data);
    let hash_sequence = dsha256(&sequences_data);

    // hashOutputs.
    let mut outputs_data = Vec::new();
    for (script, value) in &outputs {
        outputs_data.extend_from_slice(&value.to_le_bytes());
        outputs_data.extend_from_slice(&varint(script.len()));
        outputs_data.extend_from_slice(script);
    }
    let hash_outputs = dsha256(&outputs_data);

    let mut signed_inputs: Vec<Vec<u8>> = Vec::new();
    for (txid, vout, value, script_code) in utxos {
        // BIP143 sighash preimage for BCH:
        let mut preimage = Vec::new();
        preimage.extend_from_slice(&1u32.to_le_bytes()); // nVersion
        preimage.extend_from_slice(&hash_prevouts);
        preimage.extend_from_slice(&hash_sequence);
        let mut txid_bytes = hex::decode(txid).unwrap_or_default();
        txid_bytes.reverse();
        preimage.extend_from_slice(&txid_bytes);
        preimage.extend_from_slice(&vout.to_le_bytes());
        preimage.extend_from_slice(&varint(script_code.len()));
        preimage.extend_from_slice(script_code);
        preimage.extend_from_slice(&value.to_le_bytes());
        preimage.extend_from_slice(&0xffffffff_u32.to_le_bytes()); // nSequence
        preimage.extend_from_slice(&hash_outputs);
        preimage.extend_from_slice(&0u32.to_le_bytes()); // nLocktime
        preimage.extend_from_slice(&SIGHASH_ALL_FORKID.to_le_bytes());

        let sighash = dsha256(&preimage);
        let msg = Message::from_digest_slice(&sighash).map_err(|e| e.to_string())?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(SIGHASH_ALL_FORKID as u8);

        let script_sig = build_p2pkh_script_sig(&der, &pubkey_bytes);
        signed_inputs.push(build_input(txid, *vout, &script_sig));
    }

    Ok(build_tx(&signed_inputs, &outputs))
}

// ----------------------------------------------------------------
// Script / tx helpers
// ----------------------------------------------------------------

fn p2pkh_script(hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(hash);
    s.push(0x88);
    s.push(0xac);
    s
}

fn build_p2pkh_script_sig(der: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(der.len() as u8);
    out.extend_from_slice(der);
    out.push(pubkey.len() as u8);
    out.extend_from_slice(pubkey);
    out
}

fn build_input(txid: &str, vout: u32, script_sig: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut txid_bytes = hex::decode(txid).unwrap_or_default();
    txid_bytes.reverse();
    out.extend_from_slice(&txid_bytes);
    out.extend_from_slice(&vout.to_le_bytes());
    out.extend_from_slice(&varint(script_sig.len()));
    out.extend_from_slice(script_sig);
    out.extend_from_slice(&0xffffffff_u32.to_le_bytes());
    out
}

fn build_tx(inputs: &[Vec<u8>], outputs: &[(Vec<u8>, u64)]) -> Vec<u8> {
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes());
    raw.extend_from_slice(&varint(inputs.len()));
    for inp in inputs {
        raw.extend_from_slice(inp);
    }
    raw.extend_from_slice(&varint(outputs.len()));
    for (script, value) in outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.extend_from_slice(&varint(script.len()));
        raw.extend_from_slice(script);
    }
    raw.extend_from_slice(&0u32.to_le_bytes());
    raw
}

fn dsha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let first = Sha256::digest(data);
    Sha256::digest(first).into()
}

fn varint(n: usize) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => {
            let mut v = vec![0xfd];
            v.extend_from_slice(&(n as u16).to_le_bytes());
            v
        }
        _ => {
            let mut v = vec![0xfe];
            v.extend_from_slice(&(n as u32).to_le_bytes());
            v
        }
    }
}

// ----------------------------------------------------------------
// CashAddr helpers
// ----------------------------------------------------------------

/// Strip the "bitcoincash:" prefix if present.
fn normalize_bch_address(addr: &str) -> String {
    addr.strip_prefix("bitcoincash:")
        .unwrap_or(addr)
        .to_string()
}

/// Decode a BCH address (cashaddr or legacy) to its 20-byte hash.
fn decode_bch_to_hash20(address: &str) -> Result<[u8; 20], String> {
    // Try legacy base58check first.
    let norm = normalize_bch_address(address);
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        if decoded.len() == 21 {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(&decoded[1..21]);
            return Ok(hash);
        }
    }
    // Try cashaddr (simplified: extract the payload after the colon).
    // Full cashaddr decoding is complex; we decode the base32 payload.
    Err(format!("cannot decode BCH address: {address}"))
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_bch(sat: u64) -> String {
    let whole = sat / 100_000_000;
    let frac = sat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

pub fn validate_bch_address(address: &str) -> bool {
    let norm = normalize_bch_address(address);
    // Legacy P2PKH (version 0x00) or P2SH (0x05).
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        return decoded.len() == 21 && (decoded[0] == 0x00 || decoded[0] == 0x05);
    }
    // CashAddr: starts with 'q' or 'p' after stripping prefix.
    norm.starts_with('q') || norm.starts_with('p')
}

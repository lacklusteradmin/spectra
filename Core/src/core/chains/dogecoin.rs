//! Dogecoin chain client.
//!
//! Uses Blockbook-compatible REST API (same as most UTXO explorers).
//! Signing uses secp256k1 / P2PKH (Dogecoin does not support SegWit).
//! Network params: version byte 0x1e (addresses start with 'D').

use serde::{Deserialize, Serialize};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// API response types
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
#[serde(rename_all = "camelCase")]
struct BlockbookAddress {
    balance: String,
    #[serde(default)]
    #[allow(dead_code)]
    unconfirmed_balance: String,
    #[serde(default)]
    #[allow(dead_code)]
    txs: u64,
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
    #[allow(dead_code)]
    value: String,
    fees: Option<String>,
    #[allow(dead_code)]
    vin: Vec<BlockbookVin>,
    vout: Vec<BlockbookVout>,
}

#[derive(Debug, Deserialize)]
struct BlockbookVin {
    #[allow(dead_code)]
    addresses: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct BlockbookVout {
    addresses: Option<Vec<String>>,
    #[allow(dead_code)]
    value: Option<String>,
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

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct DogecoinClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl DogecoinClient {
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

    pub async fn fetch_balance(&self, address: &str) -> Result<DogeBalance, String> {
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        let koin: u64 = info.balance.parse().unwrap_or(0);
        Ok(DogeBalance {
            balance_koin: koin,
            balance_display: format_doge(koin),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<DogeUtxo>, String> {
        let utxos: Vec<BlockbookUtxo> = self
            .get(&format!("/api/v2/utxo/{address}"))
            .await?;
        Ok(utxos
            .into_iter()
            .map(|u| DogeUtxo {
                txid: u.txid,
                vout: u.vout,
                value_koin: u.value.parse().unwrap_or(0),
                confirmations: u.confirmations,
            })
            .collect())
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<DogeHistoryEntry>, String> {
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{address}?details=txs&page=1&pageSize=50"
            ))
            .await?;

        Ok(list
            .transactions
            .into_iter()
            .map(|tx| {
                let is_incoming = tx.vout.iter().any(|o| {
                    o.addresses
                        .as_deref()
                        .unwrap_or_default()
                        .contains(&address.to_string())
                });
                let amount_koin: i64 = tx.value.parse().unwrap_or(0);
                let fee_koin: u64 = tx
                    .fees
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                DogeHistoryEntry {
                    txid: tx.txid,
                    block_height: tx.block_height.unwrap_or(0),
                    timestamp: tx.block_time.unwrap_or(0),
                    amount_koin: if is_incoming { amount_koin } else { -amount_koin },
                    fee_koin,
                    is_incoming,
                }
            })
            .collect())
    }

    /// Broadcast a raw hex transaction.
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<DogeSendResult, String> {
        let hex = hex_tx.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let hex = hex.clone();
            let url = format!("{}/api/v2/sendtx/", base.trim_end_matches('/'));
            async move {
                let txid: String = client
                    .post_text(&url, hex.clone(), RetryProfile::ChainWrite)
                    .await?;
                Ok(DogeSendResult {
                    txid: txid.trim().to_string(),
                    raw_tx_hex: hex.clone(),
                })
            }
        })
        .await
    }

    /// Fetch UTXOs for `from_address`, sign a P2PKH transaction, and broadcast.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
    ) -> Result<DogeSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let script_pubkey = p2pkh_script(&decode_doge_address(from_address)?)?;
        let utxo_tuples: Vec<(String, u32, u64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.value_koin, script_pubkey.clone()))
            .collect();
        let raw = sign_doge_p2pkh(
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
// Dogecoin P2PKH signing
// ----------------------------------------------------------------

/// Sign and serialize a Dogecoin P2PKH transaction.
///
/// `utxos` — selected UTXOs with their redeeming scripts (the previous P2PKH
/// scriptPubKey for each input).
/// Returns raw transaction bytes ready for broadcast.
pub fn sign_doge_p2pkh(
    utxos: &[(String, u32, u64, Vec<u8>)], // (txid, vout, value_koin, script_pubkey)
    to_address: &str,
    amount_koin: u64,
    fee_koin: u64,
    change_address: &str,
    private_key_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let pubkey_bytes = pubkey.serialize(); // compressed

    let total_in: u64 = utxos.iter().map(|(_, _, v, _)| v).sum();
    let change = total_in.saturating_sub(amount_koin + fee_koin);

    // Build outputs.
    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(
        p2pkh_script(&decode_doge_address(to_address)?)?,
        amount_koin,
    )];
    if change > 546 {
        outputs.push((
            p2pkh_script(&decode_doge_address(change_address)?)?,
            change,
        ));
    }

    // Sign each input.
    let mut signed_inputs: Vec<Vec<u8>> = Vec::new();
    for (txid, vout, _, script_pubkey) in utxos {
        // SIGHASH_ALL preimage.
        let preimage = build_sighash_preimage(utxos, *vout, txid, script_pubkey, &outputs, 1)?;
        let hash = dsha256(&preimage);
        let msg = Message::from_digest_slice(&hash).map_err(|e| e.to_string())?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(0x01); // SIGHASH_ALL

        let script_sig = build_p2pkh_script_sig(&der, &pubkey_bytes);
        signed_inputs.push(build_input(txid, *vout, &script_sig, 0xffffffff));
    }

    Ok(build_tx(&signed_inputs, &outputs))
}

fn build_sighash_preimage(
    utxos: &[(String, u32, u64, Vec<u8>)],
    signing_vout: u32,
    signing_txid: &str,
    _script_pubkey: &[u8],
    outputs: &[(Vec<u8>, u64)],
    sighash_type: u32,
) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    // version
    raw.extend_from_slice(&1u32.to_le_bytes());
    // inputs
    raw.extend_from_slice(&varint(utxos.len()));
    for (txid, vout, _, spk) in utxos {
        let txid_bytes = decode_txid(txid)?;
        raw.extend_from_slice(&txid_bytes);
        raw.extend_from_slice(&vout.to_le_bytes());
        if vout == &signing_vout && txid == signing_txid {
            raw.extend_from_slice(&varint(spk.len()));
            raw.extend_from_slice(spk);
        } else {
            raw.push(0x00); // empty script for other inputs
        }
        raw.extend_from_slice(&0xffffffffu32.to_le_bytes());
    }
    // outputs
    raw.extend_from_slice(&varint(outputs.len()));
    for (script, value) in outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.extend_from_slice(&varint(script.len()));
        raw.extend_from_slice(script);
    }
    // locktime
    raw.extend_from_slice(&0u32.to_le_bytes());
    // sighash type
    raw.extend_from_slice(&sighash_type.to_le_bytes());
    Ok(raw)
}

fn build_input(txid: &str, vout: u32, script_sig: &[u8], sequence: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let txid_bytes = decode_txid(txid).unwrap_or_default();
    out.extend_from_slice(&txid_bytes);
    out.extend_from_slice(&vout.to_le_bytes());
    out.extend_from_slice(&varint(script_sig.len()));
    out.extend_from_slice(script_sig);
    out.extend_from_slice(&sequence.to_le_bytes());
    out
}

fn build_p2pkh_script_sig(der_sig: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(der_sig.len() as u8);
    out.extend_from_slice(der_sig);
    out.push(pubkey.len() as u8);
    out.extend_from_slice(pubkey);
    out
}

fn build_tx(inputs: &[Vec<u8>], outputs: &[(Vec<u8>, u64)]) -> Vec<u8> {
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes()); // version
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
    raw.extend_from_slice(&0u32.to_le_bytes()); // locktime
    raw
}

fn p2pkh_script(pubkey_hash: &[u8; 20]) -> Result<Vec<u8>, String> {
    Ok(vec![
        0x76, // OP_DUP
        0xa9, // OP_HASH160
        0x14, // push 20 bytes
        pubkey_hash[0], pubkey_hash[1], pubkey_hash[2], pubkey_hash[3],
        pubkey_hash[4], pubkey_hash[5], pubkey_hash[6], pubkey_hash[7],
        pubkey_hash[8], pubkey_hash[9], pubkey_hash[10], pubkey_hash[11],
        pubkey_hash[12], pubkey_hash[13], pubkey_hash[14], pubkey_hash[15],
        pubkey_hash[16], pubkey_hash[17], pubkey_hash[18], pubkey_hash[19],
        0x88, // OP_EQUALVERIFY
        0xac, // OP_CHECKSIG
    ])
}

/// Decode a Dogecoin base58check address to its 20-byte hash.
fn decode_doge_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid doge address: {e}"))?;
    // Decoded = [version_byte(1)] + [hash160(20)]
    if decoded.len() < 21 {
        return Err("address too short".to_string());
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

fn decode_txid(txid: &str) -> Result<Vec<u8>, String> {
    let mut bytes = hex::decode(txid).map_err(|e| format!("txid decode: {e}"))?;
    bytes.reverse(); // little-endian
    Ok(bytes)
}

fn dsha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);
    second.into()
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
// Formatting / validation
// ----------------------------------------------------------------

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

pub fn validate_dogecoin_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x1e || b[0] == 0x16))
        .unwrap_or(false)
}

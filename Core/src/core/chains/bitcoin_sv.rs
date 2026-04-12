//! Bitcoin SV chain client.
//!
//! BSV uses legacy P2PKH addresses (base58check, version byte 0x00 on mainnet)
//! and inherits the BIP143-variant SIGHASH_FORKID = 0x41 signing rules from
//! the BCH fork. There is no SegWit, no CashAddr, and no Taproot.
//!
//! ## Endpoints
//!
//! The canonical BSV indexer is WhatsOnChain. The endpoints vector is
//! expected to contain one or more base URLs rooted at `/v1/bsv/main`
//! (or `/v1/bsv/test` for testnet). Paths appended below:
//!
//! - `GET /address/{addr}/balance` → `{confirmed, unconfirmed}`
//! - `GET /address/{addr}/unspent`  → `[{tx_hash, tx_pos, value, height}]`
//! - `POST /tx/raw`                 → body `{"txhex": "..."}` returning a txid string
//!
//! Failures fall through to the next endpoint via `with_fallback`.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// WhatsOnChain response types
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WocBalance {
    #[serde(default)]
    confirmed: i64,
    #[serde(default)]
    unconfirmed: i64,
}

#[derive(Debug, Deserialize)]
struct WocUtxo {
    tx_hash: String,
    tx_pos: u32,
    value: u64,
    #[serde(default)]
    height: i64,
}

#[derive(Debug, Deserialize)]
struct WocHistoryItem {
    tx_hash: String,
    #[serde(default)]
    height: i64,
}

/// Full tx JSON returned by WoC `/tx/hash/{hash}`. Only the fields we
/// actually use are modeled — `#[serde(default)]` lets unknown/missing
/// fields fall through cleanly.
#[derive(Debug, Default, Deserialize)]
struct WocTxDetail {
    #[serde(default)]
    time: Option<u64>,
    #[serde(default)]
    blocktime: Option<u64>,
    #[serde(default)]
    blockheight: Option<i64>,
    #[serde(default)]
    vin: Vec<WocTxVin>,
    #[serde(default)]
    vout: Vec<WocTxVout>,
}

#[derive(Debug, Default, Deserialize)]
struct WocTxVin {
    #[serde(default)]
    addr: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct WocTxVout {
    /// BSV amount as a float (WoC convention). Convert ×1e8 for sats.
    #[serde(default)]
    value: f64,
    #[serde(default)]
    #[serde(rename = "scriptPubKey")]
    script_pub_key: Option<WocTxVoutScriptPubKey>,
}

#[derive(Debug, Default, Deserialize)]
struct WocTxVoutScriptPubKey {
    #[serde(default)]
    addresses: Option<Vec<String>>,
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Best-effort net value change for the queried address in sats.
    /// Positive = incoming (sum of vout values paid to this address).
    /// Negative = outgoing (vin addresses include this address).
    /// Zero = indeterminate (no direct match on either side).
    pub amount_sat: i64,
    pub is_incoming: bool,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct BitcoinSvClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl BitcoinSvClient {
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

    pub async fn fetch_balance(&self, address: &str) -> Result<BsvBalance, String> {
        let bal: WocBalance = self
            .get(&format!("/address/{address}/balance"))
            .await?;
        let confirmed = bal.confirmed.max(0) as u64;
        let unconfirmed = bal.unconfirmed.max(0) as u64;
        let total = confirmed.saturating_add(unconfirmed);
        Ok(BsvBalance {
            balance_sat: total,
            balance_display: format_bsv(total),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<BsvUtxo>, String> {
        let utxos: Vec<WocUtxo> = self
            .get(&format!("/address/{address}/unspent"))
            .await?;
        Ok(utxos
            .into_iter()
            .map(|u| BsvUtxo {
                txid: u.tx_hash,
                vout: u.tx_pos,
                value_sat: u.value,
                confirmations: if u.height > 0 { 1 } else { 0 },
            })
            .collect())
    }

    /// Fetch up to `limit` recent transactions for `address` via WhatsOnChain.
    ///
    /// WoC exposes `/address/{addr}/history` as a flat list of
    /// `{tx_hash, height}` entries. To populate amounts and timestamps we
    /// issue a sequential `/tx/hash/{hash}` fetch per entry — WoC free tier
    /// caps at ~3 req/sec so we deliberately cap `limit` at 25 to keep the
    /// total wall time reasonable and to stay inside the rate window.
    ///
    /// Per-entry decoding is best effort: outgoing vs incoming is inferred
    /// from vin `addr` fields (when present) and vout `scriptPubKey.addresses`
    /// lists. Fees are not computed here — BIP143 would require parent-tx
    /// fetches to know the previous output values.
    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<BsvHistoryEntry>, String> {
        let list: Vec<WocHistoryItem> = self
            .get(&format!("/address/{address}/history"))
            .await?;

        let limit = 25usize;
        let mut out: Vec<BsvHistoryEntry> = Vec::with_capacity(list.len().min(limit));
        for item in list.into_iter().take(limit) {
            let tx: WocTxDetail = match self
                .get(&format!("/tx/hash/{}", item.tx_hash))
                .await
            {
                Ok(t) => t,
                Err(_) => {
                    // Fall back to a bare entry so the user still sees the txid.
                    out.push(BsvHistoryEntry {
                        txid: item.tx_hash,
                        block_height: item.height.max(0) as u64,
                        timestamp: 0,
                        amount_sat: 0,
                        is_incoming: false,
                    });
                    continue;
                }
            };

            let (amount_sat, is_incoming) = bsv_compute_delta(&tx, address);
            let block_height = tx
                .blockheight
                .unwrap_or(item.height)
                .max(0) as u64;
            let timestamp = tx.blocktime.or(tx.time).unwrap_or(0);

            out.push(BsvHistoryEntry {
                txid: item.tx_hash,
                block_height,
                timestamp,
                amount_sat,
                is_incoming,
            });
        }

        Ok(out)
    }

    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<BsvSendResult, String> {
        let hex = hex_tx.to_string();
        with_fallback(&self.endpoints, |base| {
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
                Ok(BsvSendResult {
                    txid: txid.trim().trim_matches('"').to_string(),
                    raw_tx_hex,
                })
            }
        })
        .await
    }

    /// Fetch UTXOs for `from_address`, sign a BSV P2PKH (SIGHASH_FORKID)
    /// transaction, and broadcast.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
    ) -> Result<BsvSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let hash20 = decode_bsv_address(from_address)?;
        let script_pubkey = p2pkh_script(&hash20);
        let utxo_tuples: Vec<(String, u32, u64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.value_sat, script_pubkey.clone()))
            .collect();
        let raw = sign_bsv_tx(
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
// BSV SIGHASH_FORKID signing (BIP143-variant, inherited from BCH fork)
// ----------------------------------------------------------------

/// SIGHASH_ALL | SIGHASH_FORKID = 0x41
const SIGHASH_ALL_FORKID: u32 = 0x41;

/// Sign a BSV P2PKH transaction using SIGHASH_FORKID.
///
/// `utxos` — (txid, vout, value_sat, script_pubkey) for each selected input.
pub fn sign_bsv_tx(
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

    let to_hash = decode_bsv_address(to_address)?;
    let change_hash = decode_bsv_address(change_address)?;

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
        // BIP143 sighash preimage for BSV (same as BCH):
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
// Address helpers
// ----------------------------------------------------------------

/// Decode a BSV legacy P2PKH / P2SH base58check address to its 20-byte hash.
/// Accepts mainnet (version 0x00 / 0x05) and testnet (0x6f / 0xc4).
fn decode_bsv_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid bsv address: {e}"))?;
    if decoded.len() != 21 {
        return Err("bsv address wrong length".to_string());
    }
    let version = decoded[0];
    if version != 0x00 && version != 0x05 && version != 0x6f && version != 0xc4 {
        return Err(format!("unexpected bsv version byte: 0x{version:02x}"));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_bsv(sat: u64) -> String {
    let whole = sat / 100_000_000;
    let frac = sat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

pub fn validate_bsv_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x00 || b[0] == 0x05 || b[0] == 0x6f || b[0] == 0xc4))
        .unwrap_or(false)
}

/// Best-effort amount/direction decoding from a WoC tx detail object.
///
/// We don't have the previous-output values for inputs without extra
/// round-trips, so "outgoing" is returned with a zero amount when we can
/// only prove the address appeared on the input side. For incoming txs we
/// sum the vout values destined for the queried address and convert BSV
/// floats to satoshis.
fn bsv_compute_delta(tx: &WocTxDetail, address: &str) -> (i64, bool) {
    // Outgoing detection: any vin whose `addr` matches us.
    let is_outgoing = tx
        .vin
        .iter()
        .any(|v| v.addr.as_deref() == Some(address));

    // Incoming amount: sum vout values paid to us.
    let mut incoming_sats: u64 = 0;
    for v in &tx.vout {
        let pays_us = v
            .script_pub_key
            .as_ref()
            .and_then(|spk| spk.addresses.as_ref())
            .map(|addrs| addrs.iter().any(|a| a == address))
            .unwrap_or(false);
        if pays_us {
            // BSV float → satoshis; clamp negatives and NaN.
            let sats = (v.value * 100_000_000.0).round();
            if sats.is_finite() && sats >= 0.0 {
                incoming_sats = incoming_sats.saturating_add(sats as u64);
            }
        }
    }

    if is_outgoing {
        // Outgoing wins: amount is best-effort as the *negative* incoming
        // change (wallets that send to themselves will show a small delta).
        // When we can't attribute any value we return 0 rather than lie.
        let signed: i64 = incoming_sats.min(i64::MAX as u64) as i64;
        (-signed, false)
    } else if incoming_sats > 0 {
        (incoming_sats.min(i64::MAX as u64) as i64, true)
    } else {
        (0, false)
    }
}

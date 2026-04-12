//! Litecoin chain client.
//!
//! Litecoin supports both legacy P2PKH (L-addresses) and native SegWit P2WPKH
//! (ltc1q- addresses). Uses Blockbook REST API.
//! Network version byte: 0x30 (P2PKH), 0x32 (P2SH), bech32 HRP = "ltc".

use serde::{Deserialize, Serialize};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Blockbook shared types (same shape as dogecoin)
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
pub struct LtcBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtcUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtcHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Net value change for the queried address. Negative = outgoing.
    pub amount_sat: i64,
    pub fee_sat: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtcSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct LitecoinClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl LitecoinClient {
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
            async move { client.get_json(&url, RetryProfile::LitecoinRead).await }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<LtcBalance, String> {
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        let sat: u64 = info.balance.parse().unwrap_or(0);
        Ok(LtcBalance {
            balance_sat: sat,
            balance_display: format_ltc(sat),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<LtcUtxo>, String> {
        let utxos: Vec<BlockbookUtxo> = self
            .get(&format!("/api/v2/utxo/{address}"))
            .await?;
        Ok(utxos
            .into_iter()
            .map(|u| LtcUtxo {
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
            .map(|ltc_per_kb| ((ltc_per_kb * 1e8 / 1000.0).ceil() as u64).max(1))
            .unwrap_or(1)
    }

    /// Fetch the most recent 50 transactions touching `address` via
    /// Blockbook's `details=txs` pagination. `amount_sat` is returned as the
    /// net value change from the queried address's perspective (positive =
    /// received, negative = sent). Fee is the absolute tx fee; direction
    /// detection inspects the vin/vout address lists.
    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<LtcHistoryEntry>, String> {
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{address}?details=txs&page=1&pageSize=50"
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
                        .any(|a| a == address)
                });
                let amount_sat: i64 = tx.value.parse().unwrap_or(0);
                let fee_sat: u64 = tx
                    .fees
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                LtcHistoryEntry {
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

    /// Fetch confirmation status for a single txid via Blockbook `/api/v2/tx/{txid}`.
    pub async fn fetch_tx_status(&self, txid: &str) -> Result<super::bitcoin::UtxoTxStatus, String> {
        let txid = txid.to_string();
        with_fallback(&self.endpoints, |base| {
            let txid = txid.clone();
            let client = self.client.clone();
            async move {
                let url = format!("{base}/api/v2/tx/{txid}");
                let tx: BlockbookTx = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
                Ok(super::bitcoin::UtxoTxStatus {
                    txid: tx.txid,
                    confirmed,
                    block_height: tx.block_height,
                    block_time: tx.block_time,
                    confirmations: None,
                })
            }
        })
        .await
    }

    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<LtcSendResult, String> {
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
                Ok(LtcSendResult {
                    txid,
                    raw_tx_hex,
                })
            }
        })
        .await
    }

    /// Fetch UTXOs, sign a legacy P2PKH LTC transaction, and broadcast.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
    ) -> Result<LtcSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let script_pubkey = ltc_p2pkh_script(&decode_ltc_address(from_address)?)?;
        let utxo_tuples: Vec<(String, u32, u64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.value_sat, script_pubkey.clone()))
            .collect();
        let raw = sign_ltc_p2pkh(
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
// Litecoin P2PKH signing (identical wire format to DOGE/BTC legacy)
// ----------------------------------------------------------------

fn decode_ltc_address(address: &str) -> Result<[u8; 20], String> {
    // Legacy L... (0x30) or M... (0x32)
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid ltc address: {e}"))?;
    if decoded.len() < 21 {
        return Err("address too short".to_string());
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

fn ltc_p2pkh_script(pubkey_hash: &[u8; 20]) -> Result<Vec<u8>, String> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    Ok(s)
}

fn ltc_decode_txid(txid: &str) -> Result<Vec<u8>, String> {
    let mut bytes = hex::decode(txid).map_err(|e| format!("txid decode: {e}"))?;
    bytes.reverse();
    Ok(bytes)
}

fn ltc_dsha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(Sha256::digest(data)).into()
}

fn ltc_varint(n: usize) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => { let mut v = vec![0xfd]; v.extend_from_slice(&(n as u16).to_le_bytes()); v }
        _ => { let mut v = vec![0xfe]; v.extend_from_slice(&(n as u32).to_le_bytes()); v }
    }
}

fn sign_ltc_p2pkh(
    utxos: &[(String, u32, u64, Vec<u8>)],
    to_address: &str,
    amount_sat: u64,
    fee_sat: u64,
    change_address: &str,
    private_key_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| format!("invalid key: {e}"))?;
    let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &secret_key).serialize();

    let total_in: u64 = utxos.iter().map(|(_, _, v, _)| v).sum();
    let change = total_in.saturating_sub(amount_sat + fee_sat);

    let mut outputs: Vec<(Vec<u8>, u64)> = vec![
        (ltc_p2pkh_script(&decode_ltc_address(to_address)?)?, amount_sat),
    ];
    if change > 546 {
        outputs.push((ltc_p2pkh_script(&decode_ltc_address(change_address)?)?, change));
    }

    let mut signed_inputs: Vec<Vec<u8>> = Vec::new();
    for (txid, vout, _, _script_pubkey) in utxos {
        // Build SIGHASH_ALL preimage.
        let mut pre = Vec::new();
        pre.extend_from_slice(&1u32.to_le_bytes()); // version
        pre.extend_from_slice(&ltc_varint(utxos.len()));
        for (t, v, _, spk) in utxos {
            pre.extend_from_slice(&ltc_decode_txid(t)?);
            pre.extend_from_slice(&v.to_le_bytes());
            if v == vout && t == txid {
                pre.extend_from_slice(&ltc_varint(spk.len()));
                pre.extend_from_slice(spk);
            } else {
                pre.push(0x00);
            }
            pre.extend_from_slice(&0xffffffffu32.to_le_bytes());
        }
        pre.extend_from_slice(&ltc_varint(outputs.len()));
        for (s, val) in &outputs {
            pre.extend_from_slice(&val.to_le_bytes());
            pre.extend_from_slice(&ltc_varint(s.len()));
            pre.extend_from_slice(s);
        }
        pre.extend_from_slice(&0u32.to_le_bytes()); // locktime
        pre.extend_from_slice(&1u32.to_le_bytes()); // SIGHASH_ALL

        let hash = ltc_dsha256(&pre);
        let msg = Message::from_digest_slice(&hash).map_err(|e| e.to_string())?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(0x01); // SIGHASH_ALL

        let mut script_sig = Vec::new();
        script_sig.push(der.len() as u8);
        script_sig.extend_from_slice(&der);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(&pubkey_bytes);

        let mut inp = Vec::new();
        inp.extend_from_slice(&ltc_decode_txid(txid)?);
        inp.extend_from_slice(&vout.to_le_bytes());
        inp.extend_from_slice(&ltc_varint(script_sig.len()));
        inp.extend_from_slice(&script_sig);
        inp.extend_from_slice(&0xffffffffu32.to_le_bytes());
        signed_inputs.push(inp);
    }

    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes()); // version
    raw.extend_from_slice(&ltc_varint(signed_inputs.len()));
    for inp in &signed_inputs { raw.extend_from_slice(inp); }
    raw.extend_from_slice(&ltc_varint(outputs.len()));
    for (s, val) in &outputs {
        raw.extend_from_slice(&val.to_le_bytes());
        raw.extend_from_slice(&ltc_varint(s.len()));
        raw.extend_from_slice(s);
    }
    raw.extend_from_slice(&0u32.to_le_bytes()); // locktime
    Ok(raw)
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_ltc(sat: u64) -> String {
    let whole = sat / 100_000_000;
    let frac = sat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

pub fn validate_litecoin_address(address: &str) -> bool {
    // ltc1q... (bech32)
    if address.starts_with("ltc1") {
        return bech32::decode(address)
            .map(|(hrp, _)| hrp.as_str() == "ltc")
            .unwrap_or(false);
    }
    // Legacy L... or M... (P2PKH / P2SH)
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x30 || b[0] == 0x32 || b[0] == 0x05))
        .unwrap_or(false)
}

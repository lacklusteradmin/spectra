//! Litecoin send: build + sign legacy P2PKH transactions, then broadcast via
//! Blockbook `/api/v2/sendtx`.

use crate::http::{with_fallback, RetryProfile};

use crate::derivation::chains::litecoin::{decode_ltc_address, ltc_p2pkh_script};
use crate::fetch::chains::litecoin::{LitecoinClient, LtcSendResult};

impl LitecoinClient {
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
                Ok(LtcSendResult { txid, raw_tx_hex })
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
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &secret_key).serialize();

    let total_in: u64 = utxos.iter().map(|(_, _, v, _)| v).sum();
    let change = total_in.saturating_sub(amount_sat + fee_sat);

    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(
        ltc_p2pkh_script(&decode_ltc_address(to_address)?)?,
        amount_sat,
    )];
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
    for inp in &signed_inputs {
        raw.extend_from_slice(inp);
    }
    raw.extend_from_slice(&ltc_varint(outputs.len()));
    for (s, val) in &outputs {
        raw.extend_from_slice(&val.to_le_bytes());
        raw.extend_from_slice(&ltc_varint(s.len()));
        raw.extend_from_slice(s);
    }
    raw.extend_from_slice(&0u32.to_le_bytes()); // locktime
    Ok(raw)
}

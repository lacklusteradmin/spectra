//! Bitcoin Gold send: BIP143 P2PKH signer with SIGHASH_FORKID and BTG's
//! fork ID `79`. Broadcast via Trezor Blockbook `/api/v2/sendtx`.
//!
//! BIP143 preimage uses a 4-byte hash-type field encoded as
//! `(fork_id << 8) | sighash`, so for BTG (`SIGHASH_ALL=0x01`, fork_id=79)
//! the field value is `0x00004F41`. The signature itself appends only the
//! low byte (`0x41`) to the DER per standard P2PKH script.

use crate::http::{with_fallback, RetryProfile};

use crate::derivation::chains::bitcoin_gold::{btg_p2pkh_script, decode_btg_address};
use crate::fetch::chains::bitcoin_gold::{BitcoinGoldClient, BtgSendResult};

const SIGHASH_ALL_FORKID_BYTE: u8 = 0x41;
/// Preimage hash-type field: `(BTG fork id 79 << 8) | SIGHASH_ALL_FORKID`.
const SIGHASH_PREIMAGE_HASH_TYPE: u32 = 0x0000_4F41;

impl BitcoinGoldClient {
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<BtgSendResult, String> {
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
                Ok(BtgSendResult { txid, raw_tx_hex })
            }
        })
        .await
    }

    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
    ) -> Result<BtgSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let from_hash = decode_btg_address(from_address)?;
        let from_script = btg_p2pkh_script(&from_hash);
        let utxo_tuples: Vec<(String, u32, u64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.value_sat, from_script.clone()))
            .collect();
        let raw = sign_btg_tx(
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

fn sign_btg_tx(
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

    let to_hash = decode_btg_address(to_address)?;
    let change_hash = decode_btg_address(change_address)?;

    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(btg_p2pkh_script(&to_hash), amount_sat)];
    if change > 546 {
        outputs.push((btg_p2pkh_script(&change_hash), change));
    }

    // Precompute hashPrevouts and hashSequence (BIP143 §1,2).
    let mut prevouts_data = Vec::new();
    let mut sequences_data = Vec::new();
    for (txid, vout, _, _) in utxos {
        let mut txid_bytes = hex::decode(txid).unwrap_or_default();
        txid_bytes.reverse();
        prevouts_data.extend_from_slice(&txid_bytes);
        prevouts_data.extend_from_slice(&vout.to_le_bytes());
        sequences_data.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    }
    let hash_prevouts = dsha256(&prevouts_data);
    let hash_sequence = dsha256(&sequences_data);

    let mut outputs_data = Vec::new();
    for (script, value) in &outputs {
        outputs_data.extend_from_slice(&value.to_le_bytes());
        outputs_data.extend_from_slice(&varint(script.len()));
        outputs_data.extend_from_slice(script);
    }
    let hash_outputs = dsha256(&outputs_data);

    let mut signed_inputs: Vec<Vec<u8>> = Vec::with_capacity(utxos.len());
    for (txid, vout, value, script_code) in utxos {
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
        preimage.extend_from_slice(&0xffff_ffffu32.to_le_bytes()); // nSequence
        preimage.extend_from_slice(&hash_outputs);
        preimage.extend_from_slice(&0u32.to_le_bytes()); // nLocktime
        preimage.extend_from_slice(&SIGHASH_PREIMAGE_HASH_TYPE.to_le_bytes());

        let sighash = dsha256(&preimage);
        let msg = Message::from_digest_slice(&sighash).map_err(|e| e.to_string())?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(SIGHASH_ALL_FORKID_BYTE);

        let mut script_sig = Vec::with_capacity(2 + der.len() + pubkey_bytes.len());
        script_sig.push(der.len() as u8);
        script_sig.extend_from_slice(&der);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(&pubkey_bytes);

        let mut inp = Vec::new();
        inp.extend_from_slice(&txid_bytes);
        inp.extend_from_slice(&vout.to_le_bytes());
        inp.extend_from_slice(&varint(script_sig.len()));
        inp.extend_from_slice(&script_sig);
        inp.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        signed_inputs.push(inp);
    }

    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes());
    raw.extend_from_slice(&varint(signed_inputs.len()));
    for inp in &signed_inputs {
        raw.extend_from_slice(inp);
    }
    raw.extend_from_slice(&varint(outputs.len()));
    for (script, value) in &outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.extend_from_slice(&varint(script.len()));
        raw.extend_from_slice(script);
    }
    raw.extend_from_slice(&0u32.to_le_bytes());
    Ok(raw)
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

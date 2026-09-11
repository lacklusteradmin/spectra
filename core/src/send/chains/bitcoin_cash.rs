//! BCH send: SIGHASH_FORKID P2PKH signer (BIP143-variant) and Blockbook broadcast.

use super::bitcoin_wire::{build_input, build_tx, dsha256, p2pkh_script, p2pkh_script_sig, varint};
use crate::derivation::chains::bitcoin_cash::decode_bch_to_hash20;
use crate::fetch::chains::bitcoin_cash::{BchSendResult, BitcoinCashClient};

impl BitcoinCashClient {
    /// Fetch UTXOs for `from_address`, sign a BCH P2PKH (SIGHASH_FORKID) transaction,
    /// and broadcast.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
        dust_threshold: Option<u64>,
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
            dust_threshold,
        )?;
        self.broadcast_raw_tx(&hex::encode(&raw)).await
    }
}

// ── BCH SIGHASH_FORKID signing (BIP143-variant)

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
    dust_threshold: Option<u64>,
) -> Result<Vec<u8>, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let pubkey_bytes = pubkey.serialize();

    let change = super::accounting::checked_change(
        utxos.iter().map(|(_, _, v, _)| *v),
        amount_sat,
        fee_sat,
    )?;

    let to_hash = decode_bch_to_hash20(to_address)?;
    let change_hash = decode_bch_to_hash20(change_address)?;

    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(p2pkh_script(&to_hash), amount_sat)];
    if change > dust_threshold.unwrap_or(546) {
        outputs.push((p2pkh_script(&change_hash), change));
    }

    // Precompute hashPrevouts and hashSequence (BIP143 §1,2).
    let mut prevouts_data = Vec::new();
    let mut sequences_data = Vec::new();
    for (txid, vout, _, _) in utxos {
        let txid_bytes = super::bitcoin_wire::decode_txid_le(txid)?;
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
        let txid_bytes = super::bitcoin_wire::decode_txid_le(txid)?;
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

        let script_sig = p2pkh_script_sig(&der, &pubkey_bytes);
        signed_inputs.push(build_input(txid, *vout, &script_sig, 0xffff_ffff)?);
    }

    Ok(build_tx(&signed_inputs, &outputs))
}

// ── Script / tx helpers

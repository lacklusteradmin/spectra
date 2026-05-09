//! Bitcoin Gold: address validation, BIP-32 derivation, P2PKH (G…)
//! base58check encoding. Self-contained — see `REFACTOR_NOTES.md`.


// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) const BTG_P2PKH_VERSION: u8 = 0x26;
pub(crate) const BTG_P2SH_VERSION: u8 = 0x17;

// Base58check-decode a BTG address and return the 20-byte pubkey hash; rejects non-BTG version bytes.
pub(crate) fn decode_btg_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid btg address: {e}"))?;
    if decoded.len() != 21 {
        return Err("btg legacy payload must be 21 bytes".to_string());
    }
    if decoded[0] != BTG_P2PKH_VERSION && decoded[0] != BTG_P2SH_VERSION {
        return Err(format!("unrecognised btg version byte: 0x{:02x}", decoded[0]));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

// Build the standard P2PKH script (OP_DUP OP_HASH160 <hash> OP_EQUALVERIFY OP_CHECKSIG).
pub(crate) fn btg_p2pkh_script(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

// ── Derivation ────────────────────────────────────────────────────────────

use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::derivation::chains::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::SpectraBridgeError;

/// BIP-39 → secp256k1 keypair → BTG P2PKH address (version byte 0x26).
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (pk, priv_bytes) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    let address = want_address.then(|| {
        let mut payload = vec![BTG_P2PKH_VERSION];
        payload.extend_from_slice(&hash160(&pk.serialize()));
        base58check_encode(&payload)
    });
    Ok((
        address,
        want_public_key.then(|| hex::encode(pk.serialize())),
        want_private_key.then(|| hex::encode(priv_bytes)),
    ))
}

/// UniFFI export: derive Bitcoin Gold mainnet keys; only P2PKH script type is supported.
#[uniffi::export]
pub fn derive_bitcoin_gold(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    if !matches!(script_type, BitcoinScriptType::P2pkh) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Bitcoin Gold only supports P2PKH addresses.".into(),
        });
    }
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (pk, priv_bytes) = derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    let address = want_address.then(|| {
        let mut payload = vec![BTG_P2PKH_VERSION];
        payload.extend_from_slice(&hash160(&pk.serialize()));
        base58check_encode(&payload)
    });
    Ok(DerivationResult {
        address,
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(priv_bytes)),
        account, branch, index,
    })
}

/// True if address is a valid BTG P2PKH (base58check) or P2WPKH (bech32 "btg1") address.
pub fn validate_bitcoin_gold_address(address: &str) -> bool {
    if address.starts_with("btg1") {
        return bech32::decode(address)
            .map(|(hrp, _)| hrp.as_str() == "btg")
            .unwrap_or(false);
    }
    decode_btg_address(address).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_btc_p2pkh() {
        // BTC P2PKH starts with '1' (version 0x00); BTG must reject.
        assert!(!validate_bitcoin_gold_address(
            "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        ));
    }

    #[test]
    fn p2pkh_script_shape() {
        let s = btg_p2pkh_script(&[0u8; 20]);
        assert_eq!(s.len(), 25);
        assert_eq!(s[0], 0x76);
        assert_eq!(s[24], 0xac);
    }
}

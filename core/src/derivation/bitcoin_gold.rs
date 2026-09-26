//! Bitcoin Gold: address validation, BIP-32 derivation, P2PKH (G…)
//! base58check encoding

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
        return Err(format!(
            "unrecognised btg version byte: 0x{:02x}",
            decoded[0]
        ));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::derive_legacy_p2pkh;
use crate::derivation::types::{BitcoinScriptType, DerivationResult};

/// UniFFI export: derive Bitcoin Gold mainnet keys; only P2PKH script type is supported.
pub fn derive_bitcoin_gold(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        BTG_P2PKH_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
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
}

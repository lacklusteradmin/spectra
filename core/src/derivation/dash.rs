//! Dash: address validation, BIP-32 derivation, P2PKH (X…) base58check
//! encoding

// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) const DASH_P2PKH_VERSION: u8 = 0x4C;
pub(crate) const DASH_P2SH_VERSION: u8 = 0x10;

// Base58check-decode a Dash address; rejects non-Dash version bytes (0x4C / 0x10).
pub(crate) fn decode_dash_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid dash address: {e}"))?;
    if decoded.len() != 21 {
        return Err("dash legacy payload must be 21 bytes".to_string());
    }
    if decoded[0] != DASH_P2PKH_VERSION && decoded[0] != DASH_P2SH_VERSION {
        return Err(format!(
            "unrecognised dash version byte: 0x{:02x}",
            decoded[0]
        ));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

/// True if address passes Dash base58check decode with a recognised version byte.
/// Whether `address` is valid on the network asked about.
///
/// Took no network, so the testnet arm of the dispatcher ran the mainnet
/// decoder: a derived testnet address failed the app's own validator, which
/// means the receive screen showed an address the send screen would refuse.
/// Testnet addresses carry their own version byte.
pub(crate) fn decode_dash_testnet_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid dash testnet address: {e}"))?;
    if decoded.len() != 21 || decoded[0] != DASH_TESTNET_P2PKH {
        return Err("not a dash testnet address".to_string());
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

pub fn validate_dash_address(address: &str, testnet: bool) -> bool {
    match decode_dash_address(address) {
        Ok(_) => !testnet,
        Err(_) => testnet && decode_dash_testnet_address(address).is_ok(),
    }
}

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::derive_legacy_p2pkh;
use crate::derivation::types::{BitcoinScriptType, DerivationResult};

const DASH_MAINNET_P2PKH: u8 = 0x4C;
const DASH_TESTNET_P2PKH: u8 = 0x8C;

/// UniFFI export: derive Dash mainnet keys (P2PKH only).
pub fn derive_dash(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        DASH_MAINNET_P2PKH,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive Dash testnet keys.
pub fn derive_dash_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        DASH_TESTNET_P2PKH,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_btc_p2pkh() {
        assert!(!validate_dash_address(
            "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
            false
        ));
    }
}

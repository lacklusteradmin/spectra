//! Dash: address validation, BIP-32 derivation, P2PKH (X…) base58check
//! encoding

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::derive_legacy_p2pkh;
use crate::derivation::types::{BitcoinScriptType, DerivationResult};

const DASH_MAINNET_P2PKH: u8 = 0x4C;
const DASH_TESTNET_P2PKH: u8 = 0x8C;

/// Derive Dash mainnet keys (P2PKH only).
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

/// Derive Dash testnet keys.
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
    use crate::derivation::utxo_address::parse_utxo_address;
    use crate::registry::Chain;

    #[test]
    fn rejects_btc_p2pkh() {
        assert!(parse_utxo_address(Chain::Dash, "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").is_err());
    }
}

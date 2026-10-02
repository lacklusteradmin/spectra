//! Bitcoin Gold: address validation, BIP-32 derivation, P2PKH (G…)
//! base58check encoding

pub(crate) const BTG_P2PKH_VERSION: u8 = 0x26;

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::derive_legacy_p2pkh;
use crate::derivation::types::{BitcoinScriptType, DerivationResult};

/// Derive Bitcoin Gold mainnet keys; only P2PKH script type is supported.
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

#[cfg(test)]
mod tests {
    use crate::derivation::utxo_address::parse_utxo_address;
    use crate::registry::Chain;

    #[test]
    fn rejects_btc_p2pkh() {
        // BTC P2PKH starts with '1' (version 0x00); BTG must reject.
        assert!(
            parse_utxo_address(Chain::BitcoinGold, "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").is_err()
        );
    }
}

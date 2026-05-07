use crate::derivation::chains::xrp as xrp_chain;
use crate::SpectraBridgeError;
use super::types::{DerivationResult, parse_path_metadata};

fn internal(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = xrp_chain::derive_from_seed_phrase(
        &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

#[uniffi::export]
pub fn derive_xrp(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_xrp_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

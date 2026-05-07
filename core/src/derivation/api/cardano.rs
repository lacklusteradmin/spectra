use crate::derivation::chains::cardano as cardano_chain;
use crate::SpectraBridgeError;
use super::types::{DerivationResult, parse_path_metadata};

fn internal(
    mainnet: bool,
    seed_phrase: String, derivation_path: Option<String>, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = derivation_path.as_deref()
        .map(parse_path_metadata)
        .unwrap_or((0, 0, 0));
    let (address, public_key_hex, private_key_hex) = cardano_chain::derive_from_seed_phrase(
        mainnet, &seed_phrase, derivation_path.as_deref(), passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

#[uniffi::export]
pub fn derive_cardano(
    seed_phrase: String, derivation_path: Option<String>, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(true, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_cardano_preprod(
    seed_phrase: String, derivation_path: Option<String>, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(false, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

use crate::derivation::chains::monero as monero_chain;
use crate::SpectraBridgeError;
use super::types::DerivationResult;

fn internal(
    is_mainnet: bool,
    seed_phrase: String,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = monero_chain::derive_from_seed_phrase(
        is_mainnet, &seed_phrase, want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account: 0, branch: 0, index: 0 })
}

#[uniffi::export]
pub fn derive_monero(
    seed_phrase: String,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(true, seed_phrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_monero_stagenet(
    seed_phrase: String,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(false, seed_phrase, want_address, want_public_key, want_private_key)
}

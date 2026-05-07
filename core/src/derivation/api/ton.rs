use crate::derivation::chains::ton as ton_chain;
use crate::SpectraBridgeError;
use super::types::DerivationResult;

fn internal(
    seed_phrase: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = ton_chain::derive_ton_standard(
        &seed_phrase, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account: 0, branch: 0, index: 0 })
}

#[uniffi::export]
pub fn derive_ton(
    seed_phrase: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(seed_phrase, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_ton_testnet(
    seed_phrase: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    internal(seed_phrase, passphrase, want_address, want_public_key, want_private_key)
}

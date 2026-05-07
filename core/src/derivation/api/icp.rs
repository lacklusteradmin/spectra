use crate::derivation::chains::icp as icp_chain;
use crate::SpectraBridgeError;
use super::types::{DerivationResult, parse_path_metadata};

#[uniffi::export]
pub fn derive_icp(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = icp_chain::derive_from_seed_phrase(
        &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

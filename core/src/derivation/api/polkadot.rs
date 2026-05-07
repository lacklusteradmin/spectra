use crate::derivation::chains::polkadot as polkadot_chain;
use crate::derivation::chains::bittensor as bittensor_chain;
use crate::SpectraBridgeError;
use super::types::DerivationResult;

fn substrate_internal(
    ss58_prefix: u16,
    seed_phrase: String, passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let uniform_expansion = hmac_key.as_deref() == Some("uniform");
    let (mini_secret, public_key) = polkadot_chain::derive_substrate_sr25519_material(
        &seed_phrase, passphrase.as_deref().unwrap_or(""),
        None, None, 0, None, uniform_expansion,
    )?;
    Ok(DerivationResult {
        address: if want_address {
            Some(polkadot_chain::encode_ss58(&public_key, ss58_prefix))
        } else {
            None
        },
        public_key_hex: if want_public_key { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if want_private_key { Some(hex::encode(mini_secret)) } else { None },
        account: 0, branch: 0, index: 0,
    })
}

#[uniffi::export]
pub fn derive_polkadot(
    seed_phrase: String, passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    substrate_internal(0, seed_phrase, passphrase, hmac_key, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_polkadot_westend(
    seed_phrase: String, passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    substrate_internal(42, seed_phrase, passphrase, hmac_key, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bittensor(
    seed_phrase: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = bittensor_chain::derive_from_seed_phrase(
        &seed_phrase, passphrase.as_deref(), None,
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account: 0, branch: 0, index: 0 })
}

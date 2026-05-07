use crate::derivation::chains::evm as evm_chain;
use crate::SpectraBridgeError;
use super::types::{DerivationResult, parse_path_metadata};

// ── Shared internal ───────────────────────────────────────────────────────────

fn evm_derive_internal(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = evm_chain::derive_from_seed_phrase(
        &seed_phrase,
        &derivation_path,
        passphrase.as_deref(),
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

fn decode_privkey_hex(hex_str: &str) -> Result<[u8; 32], SpectraBridgeError> {
    let trimmed = hex_str.trim();
    if trimmed.len() != 64 {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Private key hex must be exactly 64 characters.".into(),
        });
    }
    let bytes = hex::decode(trimmed)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

// ── EVM mainnets ──────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_ethereum(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_ethereum_classic(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_arbitrum(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_optimism(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_avalanche(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_base(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bnb(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_polygon(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_hyperliquid(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_linea(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_scroll(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_blast(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_mantle(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_sei(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_celo(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_cronos(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_op_bnb(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_zksync_era(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_sonic(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_berachain(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_unichain(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_ink(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_x_layer(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

// ── EVM testnets ──────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_ethereum_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_ethereum_hoodi(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_ethereum_classic_mordor(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_arbitrum_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_optimism_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_base_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bnb_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_avalanche_fuji(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_polygon_amoy(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_hyperliquid_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_derive_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

// ── Private key variant (one covers all EVM chains and testnets) ──────────────

#[uniffi::export]
pub fn derive_evm_from_private_key(
    private_key_hex: String,
    want_address: bool,
    want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let key_bytes = decode_privkey_hex(&private_key_hex)?;
    let (address, public_key_hex) =
        evm_chain::derive_from_private_key_bytes(&key_bytes, want_address, want_public_key)?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex: None,
        account: 0,
        branch: 0,
        index: 0,
    })
}

//! Every Substrate network's accounts (Polkadot, Kusama, Astar, Bittensor and
//! Westend): SS58 address decoding, BIP-39 → substrate-bip39 PBKDF2
//! mini-secret → sr25519 derivation, SS58 v1 address encoding.
//!
//! - An address carries its network's SS58 prefix (`Chain::ss58_prefix`):
//!   one with another network's prefix names the same key but is refused,
//!   since it was written for that network.
//! - A derivation path is Substrate's hard (`//`) and soft (`/`) junctions
//!   (`derivation::substrate_path`); none derives the root sr25519 keypair.

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{
    decode_ss58 as decode_ss58_with_prefix, derive_substrate_sr25519_material, encode_ss58,
};

use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::Chain;

fn prefix(chain: Chain) -> Result<u16, DerivationError> {
    chain
        .ss58_prefix()
        .ok_or_else(|| DerivationError::invalid(format!("{chain} is not a Substrate network")))
}

/// The 32-byte account an SS58 address names on `chain`, refused unless the
/// address carries that network's prefix.
pub(crate) fn decode_ss58(chain: Chain, address: &str) -> Result<[u8; 32], DerivationError> {
    decode_ss58_with_prefix(address, Some(prefix(chain)?)).map(|(_, key)| key)
}

// ── Derivation entry points ────────────────────────────────────────────────────────

/// Derive a Substrate wallet from a seed phrase along a junction path; the
/// address carries `chain`'s SS58 prefix.
pub(crate) fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let ss58_prefix = prefix(chain)?;
    // The override's one meaning, on the network that offers it (a wallet
    // elsewhere is refused one when stored): `uniform` expands the root seed
    // in schnorrkel's Uniform mode. Any other value would be ignored, so it
    // is refused rather than deriving the key it does not change.
    let uniform_expansion = chain.supports_derivation_hmac_override()
        && match hmac_key.map(str::trim) {
            None | Some("") => false,
            Some("uniform") => true,
            Some(_) => {
                return Err(DerivationError::refused(
                    "%@ takes only `uniform` as its HMAC override.",
                    [chain.chain_display_name()],
                ));
            }
        };
    let (key, public_key) = derive_substrate_sr25519_material(
        seed_phrase,
        passphrase.unwrap_or(""),
        None,
        None,
        0,
        Some(path),
        uniform_expansion,
    )?;
    Ok(DerivationResult {
        address: wants.address.then(|| encode_ss58(&public_key, ss58_prefix)),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&key),
    })
}

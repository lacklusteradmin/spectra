//! The Cosmos SDK family: an account address is a hash of its secp256k1 key
//! under the network's bech32 prefix.
//!
//! The SDK's own key hashes the 33-byte compressed key with SHA-256 then
//! RIPEMD-160; Ethermint's (Injective, Dymension) takes the last 20 bytes of
//! Keccak-256 of the 64-byte uncompressed key, as Ethereum does. Which one a
//! network uses is `registry::CosmosKey`.

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Bech32, Hrp};
use secp256k1::{PublicKey, Secp256k1};

use crate::derivation::error::DerivationError;
use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::{Chain, CosmosKey};

/// The 20 bytes an account address encodes for `key` on `chain`.
pub(crate) fn account_hash(chain: Chain, key: &PublicKey) -> Result<[u8; 20], DerivationError> {
    use sha2::Digest as _;
    let network = chain.cosmos_network()?;
    Ok(match network.key {
        CosmosKey::Secp256k1 => {
            let sha = sha2::Sha256::digest(key.serialize());
            ripemd::Ripemd160::digest(sha).into()
        }
        CosmosKey::EthSecp256k1 { .. } => {
            let hash = sha3::Keccak256::digest(&key.serialize_uncompressed()[1..]);
            hash[12..]
                .try_into()
                .expect("a Keccak-256 hash has 32 bytes")
        }
    })
}

/// `bytes` as an address on `chain`: classic bech32 under its prefix.
pub(crate) fn encode_address(chain: Chain, bytes: &[u8]) -> Result<String, DerivationError> {
    let hrp = Hrp::parse(chain.cosmos_network()?.hrp).map_err(DerivationError::invalid)?;
    bech32::encode::<Bech32>(hrp, bytes).map_err(DerivationError::invalid)
}

/// The address `key` holds on `chain`.
pub(crate) fn address_from_public_key(
    chain: Chain,
    key: &PublicKey,
) -> Result<String, DerivationError> {
    encode_address(chain, &account_hash(chain, key)?)
}

/// The bytes an address on `chain` names: 20 for a key's account, 32 for an
/// account a module or contract derives. Refused unless it carries the
/// network's own prefix and a classic bech32 checksum (bech32m is another
/// checksum, which no Cosmos address uses), and is written in one case.
pub(crate) fn decode_address(chain: Chain, address: &str) -> Result<Vec<u8>, DerivationError> {
    let network = chain.cosmos_network()?;
    let invalid = || {
        DerivationError::refused(
            "Not a %@ address.",
            [chain.chain_display_name().to_string()],
        )
    };
    if address.chars().any(|c| c.is_ascii_uppercase())
        && address.chars().any(|c| c.is_ascii_lowercase())
    {
        return Err(invalid());
    }
    let checked = CheckedHrpstring::new::<Bech32>(address).map_err(|_| invalid())?;
    if !checked.hrp().as_str().eq_ignore_ascii_case(network.hrp) {
        return Err(invalid());
    }
    let bytes: Vec<u8> = checked.byte_iter().collect();
    if bytes.len() != 20 && bytes.len() != 32 {
        return Err(invalid());
    }
    Ok(bytes)
}

/// Derive a Cosmos account from a seed phrase along a BIP-32 path, as Keplr
/// and the SDK's own keyring do: the secp256k1 key at the path, whichever
/// hash the network's addresses take of it.
pub(crate) fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let private_key =
        crate::derivation::bip32::phrase_key(seed_phrase, passphrase, derivation_path)?;
    let public_key = PublicKey::from_secret_key(&Secp256k1::new(), &private_key);
    Ok(DerivationResult {
        address: if wants.address {
            Some(address_from_public_key(chain, &public_key)?)
        } else {
            None
        },
        public_key_hex: wants.public_key_hex(&public_key.serialize()),
        private_key_hex: wants.private_key_hex(&private_key.secret_bytes()),
    })
}

#[cfg(test)]
#[path = "tests/cosmos.rs"]
mod tests;

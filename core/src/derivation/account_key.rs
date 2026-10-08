//! Account public keys: the extended public key at an account's depth
//! (`m/purpose'/coin'/account'`), in the encodings its network's wallets
//! write it (`Chain::account_key_versions`). Watching one watches the whole
//! account; no private key is ever involved.

use super::bitcoin::ExtendedPublicKey;
use super::error::DerivationError;
use crate::registry::{AccountKeyVersion, Chain};

/// An account public key as a network reads it: the key and the encoding it
/// was written in, which names the script its addresses pay.
pub(crate) struct AccountKey {
    pub key: ExtendedPublicKey,
    pub version: AccountKeyVersion,
}

/// `text` as an account public key on `chain`, refused unless it is one of
/// the network's encodings, decodes with a valid checksum and public key,
/// and sits at an account's depth under a hardened account index.
pub(crate) fn parse(chain: Chain, text: &str) -> Result<AccountKey, DerivationError> {
    let text = text.trim();
    let (key, version) = ExtendedPublicKey::from_xpub_string(text).map_err(|_| {
        DerivationError::refused(
            "Not an account public key %@ reads.",
            [chain.chain_display_name()],
        )
    })?;
    let Some(version) = chain
        .account_key_versions()
        .iter()
        .find(|known| known.version == version)
        .copied()
    else {
        // The same family's other network writes it: say which mistake it is.
        let sibling = Chain::all().any(|other| {
            other != chain
                && other.mainnet_counterpart() == chain.mainnet_counterpart()
                && other
                    .account_key_versions()
                    .iter()
                    .any(|known| known.version == version)
        });
        return Err(if sibling {
            DerivationError::invalid("Account public key belongs to a different network.")
        } else {
            DerivationError::refused(
                "Not an account public key %@ reads.",
                [chain.chain_display_name()],
            )
        });
    };
    if key.depth != 3 || key.child_number < super::primitives::HARDENED_OFFSET {
        return Err(DerivationError::invalid(
            "An account public key is the key at an account's depth: m/purpose'/coin'/account'.",
        ));
    }
    Ok(AccountKey { key, version })
}

/// Whether two account public keys are one account, whatever encoding each
/// was written in: the same key and chain code at the same place.
pub(crate) fn same_account(a: &str, b: &str) -> bool {
    let identity = |text: &str| {
        ExtendedPublicKey::from_xpub_string(text.trim())
            .ok()
            .map(|(key, _)| (key.public_key, key.chain_code, key.depth, key.child_number))
    };
    identity(a).is_some_and(|a| Some(a) == identity(b))
}

/// The account's first receive address, `0/0`, as `chain` encodes it.
pub(crate) fn first_receive_address(chain: Chain, text: &str) -> Result<String, DerivationError> {
    let account = parse(chain, text)?;
    let secp = secp256k1::Secp256k1::new();
    let child = account.key.derive_child(&secp, 0)?.derive_child(&secp, 0)?;
    chain.encode_discovery_address(&child.public_key, account.version.script)
}

#[cfg(test)]
#[path = "tests/account_key.rs"]
mod tests;

//! Peercoin: legacy, SegWit and Taproot derivation.

use crate::derivation::error::DerivationError;

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::{BitcoinNetworkParams, derive_secp_keypair, encode_address_inner};
use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::registry::Chain;
use secp256k1::PublicKey;

/// Wallet-owned Peercoin inputs support legacy, SegWit v0 and Taproot key paths.
pub(crate) fn encode_peercoin_address(
    chain: Chain,
    script_type: BitcoinScriptType,
    public_key: &PublicKey,
) -> Result<String, DerivationError> {
    if !matches!(chain, Chain::Peercoin | Chain::PeercoinTestnet) {
        return Err(DerivationError::invalid("expected a Peercoin network"));
    }
    let (p2pkh_version, p2sh_versions) = chain.fixed_utxo_address_versions()?;
    let params = BitcoinNetworkParams {
        p2pkh_version,
        p2sh_version: p2sh_versions[0],
        bech32_hrp: chain.fixed_utxo_segwit_hrp().ok_or_else(|| {
            DerivationError::invalid("Peercoin network has no SegWit address prefix")
        })?,
    };
    encode_address_inner(params, script_type, public_key)
}

fn derive_peercoin_on_network(
    chain: Chain,
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (public_key, private_bytes) =
        derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    // Validate the script even when only keys were requested, so an unsupported
    // wallet path cannot be imported just by skipping address output.
    let address = encode_peercoin_address(chain, script_type, &public_key)?;
    Ok(DerivationResult {
        address: want_address.then_some(address),
        public_key_hex: want_public_key.then(|| hex::encode(public_key.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(private_bytes)),
        account,
        branch,
        index,
    })
}

/// Derive Peercoin mainnet keys (P2PKH/P2SH-P2WPKH/P2WPKH/P2TR).
pub fn derive_peercoin(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_peercoin_on_network(
        Chain::Peercoin,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

pub fn derive_peercoin_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_peercoin_on_network(
        Chain::PeercoinTestnet,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

#[cfg(test)]
#[path = "tests/peercoin.rs"]
mod tests;

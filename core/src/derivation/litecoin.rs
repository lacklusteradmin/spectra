//! Litecoin: legacy and SegWit derivation.

use crate::derivation::error::DerivationError;

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::{BitcoinNetworkParams, derive_secp_keypair, encode_address_inner};
use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::registry::Chain;
use secp256k1::PublicKey;

/// Wallet-owned Litecoin inputs are legacy or SegWit v0 single-key outputs.
/// Taproot recipients are supported separately by the send output decoder.
pub(crate) fn encode_litecoin_address(
    chain: Chain,
    script_type: BitcoinScriptType,
    public_key: &PublicKey,
) -> Result<String, DerivationError> {
    if !matches!(chain, Chain::Litecoin | Chain::LitecoinTestnet) {
        return Err(DerivationError::invalid("expected a Litecoin network"));
    }
    if matches!(script_type, BitcoinScriptType::P2tr) {
        return Err(DerivationError::invalid(
            "Litecoin wallets support P2PKH, P2SH-P2WPKH and P2WPKH addresses",
        ));
    }
    let (p2pkh_version, p2sh_versions) = chain.fixed_utxo_address_versions()?;
    let params = BitcoinNetworkParams {
        p2pkh_version,
        p2sh_version: p2sh_versions[0],
        bech32_hrp: chain.fixed_utxo_segwit_hrp().ok_or_else(|| {
            DerivationError::invalid("Litecoin network has no SegWit address prefix")
        })?,
    };
    encode_address_inner(params, script_type, public_key)
}

fn derive_litecoin_on_network(
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
    let address = encode_litecoin_address(chain, script_type, &public_key)?;
    Ok(DerivationResult {
        address: want_address.then_some(address),
        public_key_hex: want_public_key.then(|| hex::encode(public_key.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(private_bytes)),
        account,
        branch,
        index,
    })
}

/// Derive Litecoin mainnet keys (P2PKH/P2SH-P2WPKH/P2WPKH).
pub fn derive_litecoin(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_litecoin_on_network(
        Chain::Litecoin,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

pub fn derive_litecoin_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_litecoin_on_network(
        Chain::LitecoinTestnet,
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
#[path = "tests/litecoin.rs"]
mod tests;

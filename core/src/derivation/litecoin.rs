//! Litecoin: legacy and SegWit addresses.

use crate::derivation::error::DerivationError;

use crate::derivation::bitcoin::{BitcoinNetworkParams, encode_address_inner};
use crate::derivation::types::BitcoinScriptType;
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

#[cfg(test)]
#[path = "tests/litecoin.rs"]
mod tests;

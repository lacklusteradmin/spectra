//! Peercoin: legacy, SegWit and Taproot addresses.

use crate::derivation::error::DerivationError;

use crate::derivation::bitcoin::{BitcoinNetworkParams, encode_address_inner};
use crate::derivation::types::BitcoinScriptType;
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

#[cfg(test)]
#[path = "tests/peercoin.rs"]
mod tests;

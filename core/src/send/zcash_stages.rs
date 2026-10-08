//! What Zcash's transparent funds can pay. Never asks a server to construct or sign.

use crate::registry::Chain;
use crate::send::error::SendError;

/// The output script paying `address` from transparent funds: a transparent
/// address's own, or for a TEX address (ZIP-320) the P2PKH script of the key
/// hash it carries. A Sapling or unified address is paid from shielded funds.
pub(crate) fn address_script(address: &str, chain: Chain) -> Result<Vec<u8>, SendError> {
    use zcash_keys::address::Address;
    use zcash_transparent::address::TransparentAddress;
    let network = chain.zcash_network()?;
    match Address::decode(&network, address.trim()) {
        Some(
            Address::Transparent(TransparentAddress::PublicKeyHash(hash)) | Address::Tex(hash),
        ) => Ok(super::bitcoin_wire::p2pkh_script(&hash)),
        Some(Address::Transparent(TransparentAddress::ScriptHash(hash))) => {
            let mut script = vec![0xa9, 0x14];
            script.extend(hash);
            script.push(0x87);
            Ok(script)
        }
        Some(Address::Sapling(_) | Address::Unified(_)) => Err(SendError::invalid(
            "A shielded address is paid from the wallet's shielded funds.",
        )),
        None => Err(SendError::invalid("Not a Zcash address on this network")),
    }
}

#[cfg(test)]
#[path = "tests/zcash_stages.rs"]
mod tests;

//! Bittensor SS58 address handling.
//!
//! Bittensor wallets use the substrate-generic SS58 prefix (42), producing
//! addresses that start with `5…` and share the same length as Polkadot's
//! `1…` mainnet addresses. Wire-level the format is identical:
//!   `[prefix(1-2 bytes)] || [pubkey(32)] || [checksum(2)]`, base58-encoded.
//!
//! The 32-byte payload is treated as a sr25519 public key by the runtime;
//! Bittensor does not currently use the optional ECDSA path that the
//! generic SS58 envelope reserves.

pub(crate) fn decode_bittensor_ss58(address: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("bittensor ss58 decode: {e}"))?;
    if decoded.len() < 34 {
        return Err(format!("bittensor ss58 too short: {}", decoded.len()));
    }
    let key_start = if decoded[0] < 64 { 1 } else { 2 };
    let key_bytes: [u8; 32] = decoded[key_start..key_start + 32]
        .try_into()
        .map_err(|_| "bittensor ss58 key slice error".to_string())?;
    Ok(key_bytes)
}

pub fn validate_bittensor_address(address: &str) -> bool {
    decode_bittensor_ss58(address).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage() {
        assert!(!validate_bittensor_address(""));
        assert!(!validate_bittensor_address("not-a-bittensor-address"));
    }
}

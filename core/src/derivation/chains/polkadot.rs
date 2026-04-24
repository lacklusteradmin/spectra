//! Polkadot / Substrate SS58 address decoding and validation.

/// Decode an SS58-encoded Polkadot address to a 32-byte public key.
pub(crate) fn decode_ss58(address: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("ss58 decode: {e}"))?;
    // SS58: [prefix(1-2 bytes)] + [key(32)] + [checksum(2)]
    // For single-byte prefix (< 64), total = 35 bytes.
    if decoded.len() < 34 {
        return Err(format!("ss58 too short: {}", decoded.len()));
    }
    let key_start = if decoded[0] < 64 { 1 } else { 2 };
    let key_bytes: [u8; 32] = decoded[key_start..key_start + 32]
        .try_into()
        .map_err(|_| "ss58 key slice error".to_string())?;
    Ok(key_bytes)
}

pub fn validate_polkadot_address(address: &str) -> bool {
    decode_ss58(address).is_ok()
}

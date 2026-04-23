//! Solana address validation (base58 ed25519 pubkey).

/// Solana addresses are base58-encoded 32-byte Ed25519 public keys.
pub fn validate_solana_address(address: &str) -> bool {
    match bs58::decode(address).into_vec() {
        Ok(bytes) => bytes.len() == 32,
        Err(_) => false,
    }
}

/// Decode a base58 Solana address into a 32-byte pubkey.
pub(super) fn decode_b58_32(b58: &str) -> Result<[u8; 32], String> {
    let bytes = bs58::decode(b58)
        .into_vec()
        .map_err(|e| format!("b58 decode {b58}: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("b58 {b58} not 32 bytes: {}", v.len()))
}

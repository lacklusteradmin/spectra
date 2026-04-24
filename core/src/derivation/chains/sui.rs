//! Sui address derivation and validation.

/// Derive a Sui address from an Ed25519 public key.
pub fn pubkey_to_sui_address(public_key: &[u8; 32]) -> String {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut input = vec![0x00u8]; // Ed25519 flag
    input.extend_from_slice(public_key);
    let mut h = Blake2b::<U32>::new();
    h.update(&input);
    let hash = h.finalize();
    format!("0x{}", hex::encode(&hash[..]))
}

pub fn validate_sui_address(address: &str) -> bool {
    let s = address.strip_prefix("0x").unwrap_or(address);
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

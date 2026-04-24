//! Aptos address derivation and validation.

pub fn pubkey_to_aptos_address(public_key: &[u8; 32]) -> String {
    use sha3::{Digest, Sha3_256};
    let mut h = Sha3_256::new();
    h.update(public_key);
    h.update(&[0x00u8]); // Ed25519 scheme byte
    let hash = h.finalize();
    format!("0x{}", hex::encode(&hash[..]))
}

pub fn validate_aptos_address(address: &str) -> bool {
    let s = address.strip_prefix("0x").unwrap_or(address);
    (s.len() == 64 || s.len() == 1) && s.chars().all(|c| c.is_ascii_hexdigit())
}

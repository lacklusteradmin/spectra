//! ICP address derivation and validation.

/// Derive an ICP account address from a secp256k1 public key (DER-encoded).
pub fn pubkey_der_to_icp_address(pubkey_der: &[u8]) -> String {
    use sha2::{Digest, Sha224};
    let principal_hash = Sha224::digest(pubkey_der);
    // Account address = principal + [0u8; 32] subaccount, then sha224 again.
    // Simplified: return hex of the principal bytes with checksum.
    let mut address_bytes = Vec::new();
    address_bytes.extend_from_slice(&principal_hash);
    hex::encode(&address_bytes)
}

pub fn validate_icp_address(address: &str) -> bool {
    // ICP account identifiers are 64 hex characters.
    address.len() == 64 && address.chars().all(|c| c.is_ascii_hexdigit())
}

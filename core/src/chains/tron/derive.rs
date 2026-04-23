//! Tron address derivation (secp256k1 → keccak256 → base58check, prefix 0x41),
//! validation, and base58↔EVM-hex conversion for TRC-20 ABI encoding.

/// Derive a Tron address from a secp256k1 public key (uncompressed, 65 bytes).
pub fn pubkey_to_tron_address(pubkey_uncompressed: &[u8]) -> Result<String, String> {
    if pubkey_uncompressed.len() != 65 || pubkey_uncompressed[0] != 0x04 {
        return Err("expected 65-byte uncompressed public key".to_string());
    }
    let hash = keccak256(&pubkey_uncompressed[1..]);
    let addr_bytes = &hash[12..]; // last 20 bytes
    let mut versioned = vec![0x41u8]; // Tron mainnet prefix
    versioned.extend_from_slice(addr_bytes);
    Ok(bs58::encode(&versioned).with_check().into_string())
}

/// Convert a Tron base58 address (`T…`) to an EVM-style 20-byte hex string
/// (without `0x` prefix, without the Tron `0x41` version byte). This is the
/// format TronGrid expects inside ABI-encoded contract parameters.
pub fn tron_base58_to_evm_hex(address: &str) -> Result<String, String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("base58 decode: {e}"))?;
    if decoded.len() != 21 || decoded[0] != 0x41 {
        return Err(format!(
            "invalid Tron address length/prefix: len={}",
            decoded.len()
        ));
    }
    Ok(hex::encode(&decoded[1..]))
}

pub fn validate_tron_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && b[0] == 0x41)
        .unwrap_or(false)
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut h = Keccak::v256();
    h.update(data);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

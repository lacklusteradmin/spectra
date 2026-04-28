//! Zcash transparent address decoding, script building, and validation.
//!
//! Zcash transparent ("t-addresses") use the same hash160 + base58check
//! shape as Bitcoin P2PKH but with a 2-byte version prefix:
//!   - `t1...` (P2PKH)  → version `0x1CB8`
//!   - `t3...` (P2SH)   → version `0x1CBD`
//!
//! Shielded addresses (`zs...` / `u1...`) are out of scope for this client.

pub(crate) const ZCASH_T1_VERSION: [u8; 2] = [0x1C, 0xB8];
pub(crate) const ZCASH_T3_VERSION: [u8; 2] = [0x1C, 0xBD];

/// Decode a t-address into its 20-byte hash160. Accepts both `t1` (P2PKH) and
/// `t3` (P2SH) prefixes.
pub(crate) fn decode_zcash_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid zcash address: {e}"))?;
    if decoded.len() != 22 {
        return Err("zcash address payload must be 22 bytes (2 version + 20 hash)".to_string());
    }
    let version = [decoded[0], decoded[1]];
    if version != ZCASH_T1_VERSION && version != ZCASH_T3_VERSION {
        return Err(format!(
            "unrecognised zcash version bytes: {version:02x?}"
        ));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[2..22]);
    Ok(hash)
}

/// P2PKH script for a `t1...` address: `OP_DUP OP_HASH160 <20-byte hash>
/// OP_EQUALVERIFY OP_CHECKSIG` — identical to Bitcoin.
pub(crate) fn zcash_p2pkh_script(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

pub fn validate_zcash_address(address: &str) -> bool {
    decode_zcash_address(address).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_random_garbage() {
        assert!(!validate_zcash_address(""));
        assert!(!validate_zcash_address("not-a-zec-address"));
    }

    #[test]
    fn rejects_btc_p2pkh() {
        // Bitcoin P2PKH version is 0x00 (one byte), so a valid BTC address
        // base58-decodes to 21 bytes, not 22 — should be rejected.
        assert!(!validate_zcash_address("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"));
    }

    #[test]
    fn p2pkh_script_shape() {
        let hash = [0u8; 20];
        let s = zcash_p2pkh_script(&hash);
        assert_eq!(s.len(), 25);
        assert_eq!(s[0], 0x76);
        assert_eq!(s[1], 0xa9);
        assert_eq!(s[2], 0x14);
        assert_eq!(s[23], 0x88);
        assert_eq!(s[24], 0xac);
    }
}

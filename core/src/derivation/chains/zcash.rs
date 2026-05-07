//! Zcash transparent: address validation, BIP-32 derivation, t1… P2PKH
//! base58check encoding (2-byte version prefix). Self-contained — see
//! `REFACTOR_NOTES.md`.


// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) const ZCASH_T1_VERSION: [u8; 2] = [0x1C, 0xB8];
pub(crate) const ZCASH_T3_VERSION: [u8; 2] = [0x1C, 0xBD];

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
        return Err(format!("unrecognised zcash version bytes: {version:02x?}"));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[2..22]);
    Ok(hash)
}

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

//! Dash: address validation, BIP-32 derivation, P2PKH (X…) base58check
//! encoding. Self-contained — see `REFACTOR_NOTES.md`.


// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) const DASH_P2PKH_VERSION: u8 = 0x4C;
pub(crate) const DASH_P2SH_VERSION: u8 = 0x10;

pub(crate) fn decode_dash_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid dash address: {e}"))?;
    if decoded.len() != 21 {
        return Err("dash legacy payload must be 21 bytes".to_string());
    }
    if decoded[0] != DASH_P2PKH_VERSION && decoded[0] != DASH_P2SH_VERSION {
        return Err(format!("unrecognised dash version byte: 0x{:02x}", decoded[0]));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

pub(crate) fn dash_p2pkh_script(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

pub fn validate_dash_address(address: &str) -> bool {
    decode_dash_address(address).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_btc_p2pkh() {
        assert!(!validate_dash_address("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"));
    }

    #[test]
    fn p2pkh_script_shape() {
        let s = dash_p2pkh_script(&[0u8; 20]);
        assert_eq!(s.len(), 25);
        assert_eq!(s[0], 0x76);
        assert_eq!(s[24], 0xac);
    }
}

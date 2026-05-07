//! Bitcoin Cash: address validation, BIP-32 derivation, legacy P2PKH
//! base58check encoding. Self-contained — see `REFACTOR_NOTES.md`.


// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) fn normalize_bch_address(addr: &str) -> String {
    addr.strip_prefix("bitcoincash:")
        .unwrap_or(addr)
        .to_string()
}

pub(crate) fn decode_bch_to_hash20(address: &str) -> Result<[u8; 20], String> {
    let norm = normalize_bch_address(address);
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        if decoded.len() == 21 {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(&decoded[1..21]);
            return Ok(hash);
        }
    }
    Err(format!("cannot decode BCH address: {address}"))
}

pub fn validate_bch_address(address: &str) -> bool {
    let norm = normalize_bch_address(address);
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        return decoded.len() == 21 && (decoded[0] == 0x00 || decoded[0] == 0x05);
    }
    norm.starts_with('q') || norm.starts_with('p')
}

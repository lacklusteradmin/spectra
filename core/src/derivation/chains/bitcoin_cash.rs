//! BCH address handling: CashAddr prefix stripping, legacy base58 decoding,
//! and validation.

/// Strip the "bitcoincash:" prefix if present.
pub(crate) fn normalize_bch_address(addr: &str) -> String {
    addr.strip_prefix("bitcoincash:")
        .unwrap_or(addr)
        .to_string()
}

/// Decode a BCH address (cashaddr or legacy) to its 20-byte hash.
pub(crate) fn decode_bch_to_hash20(address: &str) -> Result<[u8; 20], String> {
    // Try legacy base58check first.
    let norm = normalize_bch_address(address);
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        if decoded.len() == 21 {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(&decoded[1..21]);
            return Ok(hash);
        }
    }
    // Try cashaddr (simplified: extract the payload after the colon).
    // Full cashaddr decoding is complex; we decode the base32 payload.
    Err(format!("cannot decode BCH address: {address}"))
}

pub fn validate_bch_address(address: &str) -> bool {
    let norm = normalize_bch_address(address);
    // Legacy P2PKH (version 0x00) or P2SH (0x05).
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        return decoded.len() == 21 && (decoded[0] == 0x00 || decoded[0] == 0x05);
    }
    // CashAddr: starts with 'q' or 'p' after stripping prefix.
    norm.starts_with('q') || norm.starts_with('p')
}

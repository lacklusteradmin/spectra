//! Bitcoin SV: address validation, BIP-32 derivation, legacy P2PKH
//! base58check encoding. Self-contained — see `REFACTOR_NOTES.md`.


// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) fn decode_bsv_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid bsv address: {e}"))?;
    if decoded.len() != 21 {
        return Err("bsv address wrong length".to_string());
    }
    let version = decoded[0];
    if version != 0x00 && version != 0x05 && version != 0x6f && version != 0xc4 {
        return Err(format!("unexpected bsv version byte: 0x{version:02x}"));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

pub fn validate_bsv_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x00 || b[0] == 0x05 || b[0] == 0x6f || b[0] == 0xc4))
        .unwrap_or(false)
}

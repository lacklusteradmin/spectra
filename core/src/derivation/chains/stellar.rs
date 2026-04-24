//! Stellar strkey address decoding + validation (G... ed25519 accounts).

pub(crate) fn decode_stellar_address(address: &str) -> Result<[u8; 32], String> {
    // Stellar strkey uses RFC 4648 base32 (no padding) with CRC-16 checksum.
    let decoded = base32_decode_rfc4648(address.trim())
        .ok_or_else(|| format!("stellar base32 decode failed: {address}"))?;

    // Layout: [version_byte(1)] + [key(32)] + [checksum(2)]
    if decoded.len() != 35 {
        return Err(format!("stellar address wrong length: {}", decoded.len()));
    }
    let version = decoded[0];
    if version != 0x30 {
        // G-address = 6 << 3 = 0x30
        return Err(format!("stellar address wrong version: {version:#x}"));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded[1..33]);
    Ok(key)
}

/// Minimal RFC 4648 base32 decoder (no padding, uppercase alphabet).
fn base32_decode_rfc4648(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let s = s.to_uppercase();
    let mut bits: u32 = 0;
    let mut bit_count: u8 = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let val = ALPHABET.iter().position(|&b| b == c)? as u32;
        bits = (bits << 5) | val;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    Some(out)
}

pub fn validate_stellar_address(address: &str) -> bool {
    decode_stellar_address(address).is_ok()
}

//! XRP address validation/decoding.

/// Decode an XRP base58 address to its 20-byte account ID.
pub(crate) fn decode_xrp_address(address: &str) -> Result<Vec<u8>, String> {
    // XRP uses a custom base58 alphabet.
    let alphabet = bs58::Alphabet::new(b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz")
        .map_err(|e| format!("alphabet: {e}"))?;
    let decoded = bs58::decode(address)
        .with_alphabet(&alphabet)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("xrp address decode: {e}"))?;
    // First byte is version (0x00 for mainnet).
    if decoded.len() != 21 {
        return Err(format!("xrp address length: {}", decoded.len()));
    }
    Ok(decoded[1..].to_vec())
}

pub fn validate_xrp_address(address: &str) -> bool {
    let alphabet = match bs58::Alphabet::new(
        b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz",
    ) {
        Ok(a) => a,
        Err(_) => return false,
    };
    bs58::decode(address)
        .with_alphabet(&alphabet)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && b[0] == 0x00)
        .unwrap_or(false)
}

//! Litecoin: address validation, P2PKH (L…) base58check encoding,
//! and MWEB stealth address parsing. Self-contained — see `REFACTOR_NOTES.md`.


// ── Address validation ───────────────────────────────────────────────────

pub(crate) fn decode_ltc_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid ltc address: {e}"))?;
    if decoded.len() < 21 {
        return Err("address too short".to_string());
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

pub(crate) fn ltc_p2pkh_script(pubkey_hash: &[u8; 20]) -> Result<Vec<u8>, String> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    Ok(s)
}

pub fn validate_litecoin_address(address: &str) -> bool {
    if address.starts_with("ltcmweb1") || address.starts_with("tmweb1") {
        return bech32::decode(address)
            .map(|(hrp, data)| {
                (hrp.as_str() == "ltcmweb" || hrp.as_str() == "tmweb") && data.len() == 66
            })
            .unwrap_or(false);
    }
    if address.starts_with("ltc1") {
        return bech32::decode(address)
            .map(|(hrp, _)| hrp.as_str() == "ltc")
            .unwrap_or(false);
    }
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x30 || b[0] == 0x32 || b[0] == 0x05))
        .unwrap_or(false)
}

/// Parsed form of an `ltcmweb1…` or `tmweb1…` stealth address.
/// `scan_pubkey` (A) and `spend_pubkey` (B) are 33-byte compressed secp256k1 points.
#[derive(Debug, Clone)]
pub struct MwebAddress {
    pub scan_pubkey: [u8; 33],
    pub spend_pubkey: [u8; 33],
}

/// Decode a bech32m MWEB address into its constituent scan and spend public keys.
/// Returns an error for non-MWEB addresses or malformed payloads.
pub fn parse_mweb_address(address: &str) -> Result<MwebAddress, String> {
    let (hrp, data) = bech32::decode(address)
        .map_err(|e| format!("invalid mweb address: {e}"))?;
    if hrp.as_str() != "ltcmweb" && hrp.as_str() != "tmweb" {
        return Err(format!(
            "expected ltcmweb or tmweb HRP, got \"{}\"",
            hrp.as_str()
        ));
    }
    if data.len() != 66 {
        return Err(format!(
            "mweb address payload must be 66 bytes (scan+spend pubkeys), got {}",
            data.len()
        ));
    }
    let mut scan_pubkey = [0u8; 33];
    let mut spend_pubkey = [0u8; 33];
    scan_pubkey.copy_from_slice(&data[0..33]);
    spend_pubkey.copy_from_slice(&data[33..66]);
    Ok(MwebAddress { scan_pubkey, spend_pubkey })
}

/// Returns true if `address` is a mainnet or testnet MWEB stealth address.
pub fn is_mweb_address(address: &str) -> bool {
    address.starts_with("ltcmweb1") || address.starts_with("tmweb1")
}

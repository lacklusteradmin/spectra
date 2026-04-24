//! Litecoin address decoding, script building, and validation.

pub(crate) fn decode_ltc_address(address: &str) -> Result<[u8; 20], String> {
    // Legacy L... (0x30) or M... (0x32)
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
    // ltc1q... (bech32)
    if address.starts_with("ltc1") {
        return bech32::decode(address)
            .map(|(hrp, _)| hrp.as_str() == "ltc")
            .unwrap_or(false);
    }
    // Legacy L... or M... (P2PKH / P2SH)
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x30 || b[0] == 0x32 || b[0] == 0x05))
        .unwrap_or(false)
}

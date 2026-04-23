//! Dogecoin address decoding, P2PKH scripting, and validation.

/// Decode a Dogecoin base58check address to its 20-byte hash.
pub(super) fn decode_doge_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid doge address: {e}"))?;
    // Decoded = [version_byte(1)] + [hash160(20)]
    if decoded.len() < 21 {
        return Err("address too short".to_string());
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

pub(super) fn p2pkh_script(pubkey_hash: &[u8; 20]) -> Result<Vec<u8>, String> {
    Ok(vec![
        0x76, // OP_DUP
        0xa9, // OP_HASH160
        0x14, // push 20 bytes
        pubkey_hash[0], pubkey_hash[1], pubkey_hash[2], pubkey_hash[3],
        pubkey_hash[4], pubkey_hash[5], pubkey_hash[6], pubkey_hash[7],
        pubkey_hash[8], pubkey_hash[9], pubkey_hash[10], pubkey_hash[11],
        pubkey_hash[12], pubkey_hash[13], pubkey_hash[14], pubkey_hash[15],
        pubkey_hash[16], pubkey_hash[17], pubkey_hash[18], pubkey_hash[19],
        0x88, // OP_EQUALVERIFY
        0xac, // OP_CHECKSIG
    ])
}

pub fn validate_dogecoin_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x1e || b[0] == 0x16))
        .unwrap_or(false)
}

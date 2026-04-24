//! TON address decode + validation.

pub(crate) fn decode_ton_address(address: &str) -> Result<(i8, [u8; 32]), String> {
    // TON addresses can be in raw form (workchain:hex) or user-friendly base64url.
    if address.contains(':') {
        let parts: Vec<&str> = address.splitn(2, ':').collect();
        let workchain: i8 = parts[0].parse().map_err(|e| format!("wc: {e}"))?;
        let bytes = hex::decode(parts[1]).map_err(|e| format!("addr hex: {e}"))?;
        if bytes.len() != 32 {
            return Err("addr wrong len".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        return Ok((workchain, arr));
    }

    // User-friendly: 36 bytes base64url = [flags(1)] + [wc(1)] + [addr(32)] + [crc(2)]
    let normalized = address.replace('-', "+").replace('_', "/");
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&normalized)
        .map_err(|e| format!("base64 decode: {e}"))?;
    if decoded.len() != 36 {
        return Err(format!("TON address wrong length: {}", decoded.len()));
    }
    let workchain = decoded[1] as i8;
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&decoded[2..34]);
    Ok((workchain, arr))
}

pub fn validate_ton_address(address: &str) -> bool {
    decode_ton_address(address).is_ok()
}

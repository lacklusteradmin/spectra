//! Small BCS primitives shared by the supported Aptos and Sui transfer layouts.
pub(super) fn uleb(mut value: usize, out: &mut Vec<u8>) {
    loop {
        let b = (value & 127) as u8;
        value >>= 7;
        out.push(b | if value == 0 { 0 } else { 128 });
        if value == 0 {
            break;
        }
    }
}
pub(super) fn bytes(value: &[u8], out: &mut Vec<u8>) {
    uleb(value.len(), out);
    out.extend_from_slice(value);
}
pub(super) fn address(value: &str) -> Result<[u8; 32], String> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid 32-byte address".into());
    }
    hex::decode(format!("{value:0>64}"))
        .map_err(|_| "invalid address".to_string())?
        .try_into()
        .map_err(|_| "invalid address length".into())
}

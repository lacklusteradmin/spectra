//! Small BCS primitives shared by the supported Aptos and Sui transfer layouts.

use crate::send::error::SendError;
/// BCS's ULEB128 length or count.
pub(super) fn uleb(value: usize, out: &mut Vec<u8>) {
    super::leb128::write(value as u64, out);
}
pub(super) fn bytes(value: &[u8], out: &mut Vec<u8>) {
    uleb(value.len(), out);
    out.extend_from_slice(value);
}
pub(super) fn address(value: &str) -> Result<[u8; 32], SendError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(SendError::Invalid("invalid 32-byte address".into()));
    }
    hex::decode(format!("{value:0>64}"))
        .map_err(|_| SendError::Invalid("invalid address".into()))?
        .try_into()
        .map_err(|_| SendError::Invalid("invalid address length".into()))
}

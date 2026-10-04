//! Shared Substrate hash and signing primitives. Runtime contracts are decoded from metadata.

use crate::send::error::SendError;

pub(super) fn decode_hash_hex(hex_str: &str) -> Result<[u8; 32], SendError> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes =
        hex::decode(s).map_err(|e| SendError::Invalid(format!("hash decode: {e}").into()))?;
    bytes
        .try_into()
        .map_err(|_| SendError::Invalid(format!("hash wrong length: {}", hex_str).into()))
}

pub(crate) fn blake2b_256(data: &[u8]) -> [u8; 32] {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut h = Blake2b::<U32>::new();
    h.update(data);
    h.finalize().into()
}

//! A hash-specific execution result. An absent transaction is still pending;
//! only a committed chain result can say that execution failed.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionStatus {
    Pending,
    Confirmed { succeeded: bool, block: Option<u64> },
}

pub(crate) fn validate_hex_hash(hash: &str) -> Result<(), crate::api::error::ApiError> {
    let bytes = hash.strip_prefix("0x").unwrap_or(hash);
    if bytes.len() != 64 || !bytes.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(crate::api::error::ApiError::invalid(
            "Invalid transaction hash",
        ));
    }
    Ok(())
}
pub(crate) fn validate_base58_hash(
    hash: &str,
    bytes: usize,
) -> Result<(), crate::api::error::ApiError> {
    if bs58::decode(hash)
        .into_vec()
        .map_err(crate::api::error::ApiError::invalid)?
        .len()
        != bytes
    {
        return Err(crate::api::error::ApiError::invalid(
            "Invalid transaction hash length",
        ));
    }
    Ok(())
}

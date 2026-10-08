//! Parsing, scaling and SQLite plumbing shared by the three owners. None of
//! this is exported.
//!
//! ## Error message convention
//!
//! Errors raised here read `"<context>: <reason>"`, where the context names
//! the offending field (`private_key_hex`, `planck`, `chain_id`) and the
//! reason names the failure (`hex decode: …`, `wrong length: …`). The field
//! name comes first so the most diagnostic part survives a truncated log line.

use super::*;

use serde::Serialize;

// ── Chain ID lookup ───────────────────────────────────────────────────────

pub(super) fn evm_network(chain: Chain) -> Result<Chain, SpectraBridgeError> {
    if chain.is_evm() {
        Ok(chain)
    } else {
        Err(SpectraBridgeError::failure(format!(
            "unsupported EVM chain_id: {chain}"
        )))
    }
}

/// Serialize a value to JSON, returning the bridge error type directly.
/// Used by chain dispatch arms whose FFI signature is
/// `Result<String, SpectraBridgeError>`. New endpoints should return a
/// typed `#[derive(uniffi::Record)]` value directly rather than going
/// through this helper.
pub(super) fn json_response<T: Serialize>(value: &T) -> Result<String, SpectraBridgeError> {
    serde_json::to_string(value).map_err(Into::into)
}

/// Decode a hex string of an exact byte length. Replaces repeated
/// per-arm hex decoding and fixed-length conversion boilerplate. Includes
/// the field name in both the
/// "not hex" and "wrong length" error variants.
pub(super) fn decode_hex_array<const N: usize>(
    hex_str: &str,
    field_name: &str,
) -> Result<[u8; N], SpectraBridgeError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| SpectraBridgeError::failure(format!("{field_name} hex decode: {e}")))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        SpectraBridgeError::failure(format!(
            "{field_name} wrong length: expected {N} bytes, got {}",
            v.len()
        ))
    })
}

/// Decode a variable-length private key. Eleven signing arms carried this
/// same three-line closure; `decode_hex_array` covers only the fixed-length
/// keys.
pub(super) fn decode_private_key(
    hex_str: &str,
) -> Result<zeroize::Zeroizing<Vec<u8>>, SpectraBridgeError> {
    hex::decode(hex_str)
        .map(zeroize::Zeroizing::new)
        .map_err(|_| SpectraBridgeError::failure("invalid private key hex"))
}

/// The fee to sign with: whatever the preview settled on, and otherwise the
/// chain's own [`Chain::static_fee_units`] — which is where the fee the user
/// was shown comes from.
pub(super) fn fee_or_static(
    chain: crate::registry::Chain,
    fee: Option<u64>,
) -> Result<u64, SpectraBridgeError> {
    let fee = match fee {
        Some(fee) => fee,
        None => u64::try_from(
            chain
                .static_fee_units()
                .ok_or_else(|| SpectraBridgeError::failure("No fee available for this chain"))?,
        )
        .map_err(|_| SpectraBridgeError::failure("Fee exceeds protocol range"))?,
    };
    if fee == 0 {
        return Err(SpectraBridgeError::failure("Fee must be positive"));
    }
    Ok(fee)
}

// ── Balance projection ────────────────────────────────────────────────────

// ── Fee estimate ──────────────────────────────────────────────────────────

/// A chain's fee, quoted in that chain's own native unit.
#[derive(Debug, Clone)]
pub(crate) struct NativeFeeEstimate {
    /// Smallest units, as a decimal string — some chains' fees do not fit u64.
    pub raw: String,
    /// The same amount at display scale.
    pub display: String,
    /// `"rpc"` when a node quoted it, `"static"` when the catalog did.
    pub source: &'static str,
    /// Aptos's quoted unit price, distinct from the total reserved fee.
    pub gas_unit_price_octas: Option<u64>,
}

// ── State helpers ─────────────────────────────────────────────────────────

/// Return a zero-amount AssetHolding template for the native coin of each
/// chain. Used as the default when the holding doesn't exist yet.
pub(crate) fn native_coin_template(chain_id: crate::registry::Chain) -> Option<AssetHolding> {
    Some(Chain::native_holding_template(chain_id))
}

pub(super) fn decode_secret_array<const N: usize>(
    value: &str,
) -> Result<zeroize::Zeroizing<[u8; N]>, SpectraBridgeError> {
    let bytes = decode_private_key(value)?;
    let array: &[u8; N] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| SpectraBridgeError::failure(format!("private key must be {N} bytes")))?;
    Ok(zeroize::Zeroizing::new(*array))
}

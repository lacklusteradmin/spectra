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
        Err(format!("unsupported EVM chain_id: {chain}").into())
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
        .map_err(|e| SpectraBridgeError::from(format!("{field_name} hex decode: {e}")))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        SpectraBridgeError::from(format!(
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
        .map_err(|_| SpectraBridgeError::from("invalid private key hex"))
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
                .ok_or("No fee available for this chain")?,
        )
        .map_err(|_| "Fee exceeds protocol range")?,
    };
    if fee == 0 {
        return Err("Fee must be positive".into());
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
}

/// Compute a UTXO capacity fee preview using P2PKH sizing (148 B/input,
/// 34 B/output, 10 B overhead). Assumes all confirmed UTXOs above the
/// 546-satoshi dust threshold are selected, single-output (max-send) tx.
pub(super) fn utxo_fee_preview_json(utxo_values: Vec<u64>, fee_rate: u64) -> String {
    const INPUT_BYTES: u64 = 148;
    const OUTPUT_BYTES: u64 = 34;
    const OVERHEAD: u64 = 10;
    const DUST: u64 = 546;

    let spendable: Vec<u64> = utxo_values.into_iter().filter(|&v| v >= DUST).collect();
    let n = spendable.len() as u64;
    let total: u64 = spendable.iter().sum();

    if n == 0 || total == 0 {
        return json!({
            "fee_rate_svb": fee_rate,
            "estimated_fee_sat": 0_u64,
            "estimated_tx_bytes": 0_u64,
            "selected_input_count": 0_u64,
            "uses_change_output": false,
            "spendable_balance_sat": 0_u64,
            "max_sendable_sat": 0_u64,
        })
        .to_string();
    }

    let tx_bytes = OVERHEAD + n * INPUT_BYTES + OUTPUT_BYTES;
    let fee = tx_bytes * fee_rate;
    let max_sendable = total.saturating_sub(fee);

    json!({
        "fee_rate_svb": fee_rate,
        "estimated_fee_sat": fee,
        "estimated_tx_bytes": tx_bytes,
        "selected_input_count": n,
        "uses_change_output": false,
        "spendable_balance_sat": total,
        "max_sendable_sat": max_sendable,
    })
    .to_string()
}

// ── State helpers ─────────────────────────────────────────────────────────

/// Return a zero-amount AssetHolding template for the native coin of each
/// chain. Used as the default when the holding doesn't exist yet.
pub(crate) fn native_coin_template(chain_id: crate::registry::Chain) -> Option<AssetHolding> {
    Some(Chain::native_holding_template(chain_id))
}

/// Returns `true` when `s` starts with a BIP-32 extended public key prefix.
pub(super) fn is_extended_public_key(s: &str) -> bool {
    matches!(
        s.get(..4),
        Some("xpub") | Some("ypub") | Some("zpub") | Some("Ypub") | Some("Zpub")
    )
}

pub(super) fn decode_secret_array<const N: usize>(
    value: &str,
) -> Result<zeroize::Zeroizing<[u8; N]>, SpectraBridgeError> {
    let bytes = decode_private_key(value)?;
    let array: &[u8; N] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| SpectraBridgeError::from(format!("private key must be {N} bytes")))?;
    Ok(zeroize::Zeroizing::new(*array))
}

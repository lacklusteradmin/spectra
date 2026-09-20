//! Exports that need no service state: hex normalisation, the large-movement
//! threshold, the built-in token catalog, and BIP-39.
//!
//! Synchronous and free of I/O, so Swift can call them from a `static let`.

use crate::SpectraBridgeError;

/// Trim + lowercase + strip leading `0x` from a private-key hex string.
pub(crate) fn private_key_hex_normalized(raw_value: String) -> String {
    let trimmed = raw_value.trim().to_lowercase();
    match trimmed.strip_prefix("0x") {
        Some(stripped) => stripped.to_string(),
        None => trimmed,
    }
}

/// The normalised 32-byte hex key, or `None` when the input is not one.
///
/// Not exported: the import commit is the only caller that needs the key, and
/// it is in core. The editor needs only [`is_private_key_hex`].
pub(crate) fn private_key_hex(raw_value: String) -> Option<String> {
    let normalized = private_key_hex_normalized(raw_value);
    (normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(normalized)
}

/// Whether the input is a 32-byte hex key, once normalised.
///
/// The key editor asks this on every render. It asked `private_key_hex`,
/// which answered with the normalised key itself — a fresh copy of a secret
/// back across the boundary each time, for a caller that compared it with
/// `nil` — and the app then kept up to 128 typed candidates as the keys of a
/// process-lifetime cache so as not to ask again. A yes or no is all the
/// editor reads, and it is cheap enough to ask directly.
#[uniffi::export]
pub fn is_private_key_hex(raw_value: String) -> bool {
    private_key_hex(raw_value).is_some()
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct LargeMovementEvaluation {
    pub should_alert: bool,
    pub absolute_delta: f64,
    pub ratio: f64,
    pub direction_up: bool,
}

/// Evaluate whether a portfolio-total swing crosses both an absolute USD
/// threshold and a percent-change threshold (large-movement notifications).
pub(crate) fn evaluate_large_movement(
    previous_total_usd: f64,
    current_total_usd: f64,
    usd_threshold: f64,
    percent_threshold: f64,
) -> LargeMovementEvaluation {
    if previous_total_usd <= 0.0
        || [
            previous_total_usd,
            current_total_usd,
            usd_threshold,
            percent_threshold,
        ]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return LargeMovementEvaluation {
            should_alert: false,
            absolute_delta: 0.0,
            ratio: 0.0,
            direction_up: true,
        };
    }
    let delta = current_total_usd - previous_total_usd;
    let absolute_delta = delta.abs();
    let ratio = absolute_delta / previous_total_usd;
    let should_alert = absolute_delta >= usd_threshold && ratio >= (percent_threshold / 100.0);
    LargeMovementEvaluation {
        should_alert,
        absolute_delta,
        ratio,
        direction_up: delta >= 0.0,
    }
}

use crate::tokens;

/// Return the entire built-in token catalog across every registered chain.
///
/// The per-chain variant next to it had no caller left: `tokens::list_token_deployments`
/// takes the chain id directly, so a wrapper that only forwarded one was a
/// second name for the same call.
#[uniffi::export]
pub fn list_all_builtin_token_deployments() -> Vec<tokens::TokenDeploymentEntry> {
    tokens::list_token_deployments(String::new())
}

/// Generate a new random BIP-39 mnemonic of `word_count` words.
///
/// Refuses a length BIP-39 does not define rather than substituting one. This
/// used to answer twelve words to any unrecognized count, which is not a
/// substitution a wallet may make silently: both front ends then carried their
/// own copy of the accepted lengths to avoid asking for eighteen and being
/// handed twelve. [`crate::validation::seed_phrase_entropy_bits`] is now the
/// only list.
#[uniffi::export]
pub fn generate_mnemonic(word_count: u32) -> Result<String, SpectraBridgeError> {
    use bip39::{Language, Mnemonic};
    use rand::RngCore;

    let entropy_bits =
        crate::validation::seed_phrase_entropy_bits(word_count).ok_or_else(|| {
            SpectraBridgeError::InvalidInput {
                message: format!(
                    "{word_count} is not a BIP-39 phrase length. Use 12, 15, 18, 21 or 24 words."
                ),
            }
        })?;
    let mut entropy = vec![0u8; entropy_bits as usize / 8];
    rand::thread_rng().fill_bytes(&mut entropy);
    Ok(Mnemonic::from_entropy_in(Language::English, &entropy)
        .expect("valid entropy length")
        .to_string())
}

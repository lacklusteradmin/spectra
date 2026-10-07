//! Exports that need no service state: the large-movement threshold, the
//! built-in token catalog, and phrase generation.
//!
//! Synchronous and free of I/O, so Swift can call them from a `static let`.

use crate::SpectraBridgeError;

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct LargeMovementEvaluation {
    pub should_alert: bool,
    /// In `currency`: the display currency when its rate is known, else USD.
    pub absolute_delta: f64,
    pub ratio: f64,
    pub direction_up: bool,
    pub currency: crate::store::state::FiatCurrency,
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
            currency: crate::store::state::FiatCurrency::Usd,
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
        currency: crate::store::state::FiatCurrency::Usd,
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
    tokens::list_token_deployments(None)
}

/// Generate a new phrase for a wallet on `chain`, of `word_count` words, in
/// the format the chain's own wallets restore: BIP-39 for most, Monero's
/// 25-word seed, TON's 24-word mnemonic.
///
/// Refuses a length the chain's created format does not have rather than
/// substituting one; `seed_phrase_lengths` lists them.
#[uniffi::export]
pub fn generate_seed_phrase(
    chain: crate::registry::Chain,
    word_count: u32,
) -> Result<String, SpectraBridgeError> {
    Ok(crate::derivation::phrase::generate_phrase(chain, word_count)?.to_string())
}

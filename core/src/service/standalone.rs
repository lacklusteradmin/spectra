//! Exports that need no service state: hex normalisation, the large-movement
//! threshold, the built-in token catalog, and BIP-39.
//!
//! Synchronous and free of I/O, so Swift can call them from a `static let`.

/// Trim + lowercase + strip leading `0x` from a private-key hex string.
pub(crate) fn core_private_key_hex_normalized(raw_value: String) -> String {
    let trimmed = raw_value.trim().to_lowercase();
    match trimmed.strip_prefix("0x") {
        Some(stripped) => stripped.to_string(),
        None => trimmed,
    }
}

/// The normalised 32-byte hex key, or `None` when the input is not one.
///
/// Was two exports: a normaliser and a predicate over the normaliser's own
/// result. A caller that wanted the key had to call both and hope they agreed
/// about what "normalised" meant.
#[uniffi::export]
pub fn core_private_key_hex(raw_value: String) -> Option<String> {
    let normalized = core_private_key_hex_normalized(raw_value);
    (normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(normalized)
}


#[derive(Debug, Clone, uniffi::Record)]
pub struct LargeMovementEvaluation {
    pub should_alert: bool,
    pub absolute_delta: f64,
    pub ratio: f64,
    pub direction_up: bool,
}

/// Evaluate whether a portfolio-total swing crosses both an absolute USD
/// threshold and a percent-change threshold (large-movement notifications).
#[uniffi::export]
pub fn core_evaluate_large_movement(
    previous_total_usd: f64,
    current_total_usd: f64,
    usd_threshold: f64,
    percent_threshold: f64,
) -> LargeMovementEvaluation {
    if previous_total_usd <= 0.0 {
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

/// Return the built-in token catalog filtered to one chain. Synchronous
/// so Swift can call from a `static let`. For "all chains, please" use
/// [`list_all_builtin_tokens`] — that's the named entry point, not a
/// sentinel value.
pub fn list_builtin_tokens(chain_id: String) -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(chain_id)
}

/// Return the entire built-in token catalog across every registered
/// chain. Replaces the `list_builtin_tokens(chain_id: String::MAX)`
/// sentinel pattern — the "all chains" call site now reads as exactly
/// what it means instead of forcing the reader to know the magic value.
#[uniffi::export]
pub fn list_all_builtin_tokens() -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(String::new())
}

/// Generate a new random BIP-39 mnemonic with the requested word count.
///
/// `word_count` must be 12, 15, 18, 21, or 24. Any other value falls back
/// silently to 12 words. Returns the space-joined mnemonic phrase.
#[uniffi::export]
pub fn generate_mnemonic(word_count: u32) -> String {
    use bip39::{Language, Mnemonic};
    use rand::RngCore;

    // BIP-39 entropy bytes: 128/160/192/224/256 bits → 12/15/18/21/24 words.
    let entropy_bytes: usize = match word_count {
        15 => 20,
        18 => 24,
        21 => 28,
        24 => 32,
        _ => 16, // default: 12 words
    };
    let mut entropy = vec![0u8; entropy_bytes];
    rand::thread_rng().fill_bytes(&mut entropy);
    Mnemonic::from_entropy_in(Language::English, &entropy)
        .expect("valid entropy length")
        .to_string()
}

/// Validate a BIP-39 mnemonic phrase. Returns `true` only for a valid
/// English BIP-39 mnemonic with correct word count + checksum.
#[uniffi::export]
pub fn validate_mnemonic(phrase: String) -> bool {
    use bip39::{Language, Mnemonic};
    phrase.trim().parse::<Mnemonic>().is_ok()
        || Mnemonic::parse_in(Language::English, phrase.trim()).is_ok()
}

/// The full BIP-39 word list for a language, newline-delimited.
///
/// Accepts the same language codes as the derivation functions ("en", "zh-cn",
/// …) and falls back to English for anything else — which is why
/// `bip39_english_wordlist` was this function with one argument bound, and is
/// gone.
#[uniffi::export]
pub fn bip39_wordlist(language: String) -> String {
    use bip39::Language;
    let lang = match language.trim().to_ascii_lowercase().as_str() {
        "czech" | "cs" => Language::Czech,
        "french" | "fr" => Language::French,
        "italian" | "it" => Language::Italian,
        "japanese" | "ja" | "jp" => Language::Japanese,
        "korean" | "ko" | "kr" => Language::Korean,
        "portuguese" | "pt" => Language::Portuguese,
        "spanish" | "es" => Language::Spanish,
        "simplified-chinese" | "zh-hans" | "zh-cn" | "zh" => Language::SimplifiedChinese,
        "traditional-chinese" | "zh-hant" | "zh-tw" => Language::TraditionalChinese,
        _ => Language::English,
    };
    lang.word_list().join("\n")
}

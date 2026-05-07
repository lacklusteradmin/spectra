/// Field-level validation rules shared across platforms.
///
/// Each function is pure (no I/O, no mutable state) and returns an
/// `Option<String>` error message that the UI can display inline.
/// `None` means the field is valid.

/// Returns an error or advisory message when `word_count` is outside the
/// set of BIP-39 standard mnemonic lengths, or `None` when it is valid.
///
/// BIP-39 standard lengths: 12, 15, 18, 21, 24.
/// Lengths below 12 are blocked; non-standard lengths above 12 produce
/// an advisory (not a hard block, since some wallets use custom lengths).
#[uniffi::export]
pub fn core_validate_seed_phrase_word_count(word_count: u32) -> Option<String> {
    if word_count == 0 {
        return Some("Seed phrase length must be at least 1 word.".to_string());
    }
    if word_count < 12 {
        return Some("Seed phrase is too short. Use at least 12 words.".to_string());
    }
    if ![12u32, 15, 18, 21, 24].contains(&word_count) {
        return Some(
            "Non-standard length selected. BIP-39 standard lengths are 12, 15, 18, 21, or 24 words."
                .to_string(),
        );
    }
    None
}

/// Returns an error message when `password` / `confirmation` fail the wallet
/// password rules, or `None` when both fields pass.
///
/// Rules:
///  * Both empty → valid (no password is allowed).
///  * Non-empty password shorter than 4 characters → error.
///  * Password and confirmation mismatch → error.
#[uniffi::export]
pub fn core_validate_wallet_password(password: String, confirmation: String) -> Option<String> {
    let p = password.trim();
    let c = confirmation.trim();
    if p.is_empty() && c.is_empty() {
        return None;
    }
    if p.len() < 4 {
        return Some(
            "Wallet password must be at least 4 characters, or leave it blank.".to_string(),
        );
    }
    if p != c {
        return Some("Wallet password confirmation does not match.".to_string());
    }
    None
}

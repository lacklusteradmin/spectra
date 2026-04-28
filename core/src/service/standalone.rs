//! Synchronous, stateless UniFFI exports — token catalog + BIP-39 mnemonic
//! utilities. Kept separate from the WalletService impl blocks because they
//! perform no network I/O and don't need the service's state.

use crate::registry::tokens;

/// Return the built-in token catalog filtered to one chain. Synchronous
/// so Swift can call from a `static let`. For "all chains, please" use
/// [`list_all_builtin_tokens`] — that's the named entry point, not a
/// sentinel value.
#[uniffi::export]
pub fn list_builtin_tokens(chain_id: u32) -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(chain_id)
}

/// Return the entire built-in token catalog across every registered
/// chain. Replaces the `list_builtin_tokens(chain_id: u32::MAX)`
/// sentinel pattern — the "all chains" call site now reads as exactly
/// what it means instead of forcing the reader to know the magic value.
#[uniffi::export]
pub fn list_all_builtin_tokens() -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(u32::MAX)
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

/// Return the full BIP-39 English word list as a newline-delimited string
/// (2048 words, alphabetically sorted).
#[uniffi::export]
pub fn bip39_english_wordlist() -> String {
    static WORDLIST: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| bip39::Language::English.word_list().join("\n"));
    WORDLIST.clone()
}

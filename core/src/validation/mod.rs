//! Every "is this input acceptable" rule core owns, in one module.
//!
//! This file holds the field rules — pure functions returning an
//! `Option<String>` the UI shows inline, where `None` means valid.
//! [`address`] holds the per-chain address and identifier rules, which
//! answer the same question about a much larger input.
//!
//! Address validation used to live under `derivation` because it borrows
//! that module's parsers. Validating an address is not deriving one, and the
//! split meant a caller had to know which of two modules named `validation`
//! held the rule it wanted.

pub mod address;

/// One BIP-39 seed-phrase entry, as the user typed it.
///
/// `words` are the raw per-slot entries, blanks included: whether the grid is
/// finished is part of the verdict, so the caller hands over what it has
/// rather than deciding when to ask. `language` is the wordlist the user
/// picked; `None` means "whatever language this phrase is in", which is what
/// a file-fed import wants and what a language picker never wants.
#[derive(uniffi::Record, Debug, Clone)]
pub struct SeedPhraseCheck {
    pub words: Vec<String>,
    pub language: Option<String>,
    pub expected_word_count: u32,
}

/// Everything there is to say about a seed-phrase entry, decided once.
///
/// `error` is the one line to show under the field; the rest is what a UI
/// needs to colour individual words and enable its button, and is derived
/// from the same pass so the two can never disagree.
#[derive(uniffi::Record, Debug, Clone)]
pub struct SeedPhraseVerdict {
    /// The entries normalized the way BIP-39 reads them: trimmed, lowercased,
    /// blanks dropped.
    pub words: Vec<String>,
    /// Normalized words that are not in the wordlist.
    pub invalid_words: Vec<String>,
    /// Every expected slot holds something.
    pub is_complete: bool,
    /// The phrase parses, with its checksum, in the chosen language.
    pub checksum_valid: bool,
    /// An advisory about `expected_word_count` itself, shown next to the
    /// length picker rather than the field.
    pub length_warning: Option<String>,
    /// The field's inline error, or `None` while the entry is unfinished or
    /// already valid.
    pub error: Option<String>,
}

/// Decide a seed-phrase entry: which words are not in the wordlist, whether
/// the entry is finished, whether the checksum holds, and what to say.
///
/// The order matters and is the reason this is one function. An unfinished
/// entry says nothing — every phrase is invalid until its last word. Words
/// that are not in the wordlist are their own message, so the count and the
/// checksum stay quiet behind them. Only then is a wrong count worth naming,
/// and only then is the checksum worth computing.
#[uniffi::export]
pub fn core_check_seed_phrase(check: SeedPhraseCheck) -> SeedPhraseVerdict {
    let expected = check.expected_word_count as usize;
    let is_complete = expected > 0
        && check.words.len() >= expected
        && check.words[..expected].iter().all(|w| !w.trim().is_empty());
    let words: Vec<String> = check
        .words
        .iter()
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    // With a language chosen, its list is the only one a word may come from.
    // With none, a word is unknown only when no BIP-39 language has it —
    // the caller is reading a phrase, not offering a picker.
    let wordlist: std::collections::HashSet<&'static str> = match check.language.as_deref() {
        Some(code) => bip39_language(Some(code))
            .word_list()
            .iter()
            .copied()
            .collect(),
        None => bip39::Language::ALL
            .iter()
            .flat_map(|lang| lang.word_list().iter().copied())
            .collect(),
    };
    let invalid_words: Vec<String> = words
        .iter()
        .filter(|w| !wordlist.contains(w.as_str()))
        .cloned()
        .collect();

    let checksum_valid = is_complete
        && invalid_words.is_empty()
        && words.len() == expected
        && seed_phrase_parses(&words.join(" "), check.language.as_deref());

    let error = if !is_complete || !invalid_words.is_empty() {
        None
    } else if words.len() != expected {
        Some(format!("Seed phrase must be {expected} words."))
    } else if !checksum_valid {
        Some("Invalid seed phrase checksum. Please verify your words.".to_string())
    } else {
        None
    };

    SeedPhraseVerdict {
        words,
        invalid_words,
        is_complete,
        checksum_valid,
        length_warning: seed_phrase_length_warning(check.expected_word_count),
        error,
    }
}

/// Whether `phrase` is a mnemonic with a valid checksum — in `language` when
/// one was chosen, in any BIP-39 language when none was.
///
/// Checking in the chosen language is the stricter side: English and French
/// share around a hundred words, so a phrase built from the overlap can be
/// detected as the wrong one of the two and pass a check the user's own
/// wordlist would fail.
pub fn seed_phrase_parses(phrase: &str, language: Option<&str>) -> bool {
    parse_seed_phrase(phrase, language).is_ok()
}

/// Read a phrase as a mnemonic — in `language` when one was chosen, in
/// whichever BIP-39 language holds its words when none was.
///
/// Not `Mnemonic::from_str`: that refuses a phrase whose language it cannot
/// pin down, and the Simplified and Traditional Chinese lists overlap enough
/// that ordinary Chinese mnemonics are exactly that. A phrase that reads in
/// *some* language is a mnemonic, which is the question a caller without a
/// language picker is asking.
pub fn parse_seed_phrase(phrase: &str, language: Option<&str>) -> Result<bip39::Mnemonic, String> {
    use bip39::{Language, Mnemonic};
    let phrase = phrase.trim();
    match language {
        Some(code) => {
            Mnemonic::parse_in(bip39_language(Some(code)), phrase).map_err(|e| e.to_string())
        }
        None => Language::ALL
            .iter()
            .find_map(|lang| Mnemonic::parse_in(*lang, phrase).ok())
            .ok_or_else(|| {
                Mnemonic::parse_in(Language::English, phrase)
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "Not a BIP-39 mnemonic.".to_string())
            }),
    }
}

/// The BIP-39 language for a code, English for anything unrecognized —
/// including `None`, whose callers only ever read the word list.
pub fn bip39_language(code: Option<&str>) -> bip39::Language {
    use bip39::Language;
    match code.unwrap_or("en").trim().to_ascii_lowercase().as_str() {
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
    }
}

/// An advisory when `word_count` is outside the BIP-39 standard lengths
/// (12, 15, 18, 21, 24), or `None` when it is one of them.
///
/// Below 12 is a refusal; a non-standard length above it is only a warning,
/// because some wallets do issue them.
fn seed_phrase_length_warning(word_count: u32) -> Option<String> {
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

#[cfg(test)]
mod seed_phrase_tests {
    use super::*;

    const ENGLISH: &str = "abandon abandon abandon abandon abandon abandon \
                           abandon abandon abandon abandon abandon about";

    fn check(words: &str, language: Option<&str>, expected: u32) -> SeedPhraseVerdict {
        core_check_seed_phrase(SeedPhraseCheck {
            words: words.split(' ').map(str::to_string).collect(),
            language: language.map(str::to_string),
            expected_word_count: expected,
        })
    }

    #[test]
    fn a_finished_valid_phrase_has_nothing_to_say() {
        let verdict = check(ENGLISH, Some("en"), 12);
        assert!(verdict.is_complete);
        assert!(verdict.checksum_valid);
        assert!(verdict.invalid_words.is_empty());
        assert_eq!(verdict.error, None);
        assert_eq!(verdict.length_warning, None);
    }

    #[test]
    fn an_unfinished_entry_says_nothing_at_all() {
        // Blank tail slots: every phrase is invalid until its last word, so
        // reporting a checksum failure here would be shouting at someone
        // still typing.
        let verdict = check("abandon abandon   ", Some("en"), 12);
        assert!(!verdict.is_complete);
        assert!(!verdict.checksum_valid);
        assert_eq!(verdict.error, None);
    }

    #[test]
    fn words_off_the_list_are_named_and_hide_the_checksum() {
        let verdict = check(
            "abandon zzzz abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon about",
            Some("en"),
            12,
        );
        assert_eq!(verdict.invalid_words, vec!["zzzz".to_string()]);
        assert_eq!(verdict.error, None);
    }

    #[test]
    fn a_finished_phrase_with_a_broken_checksum_says_so() {
        let broken = ENGLISH.replace("about", "abandon");
        let verdict = check(&broken, Some("en"), 12);
        assert!(verdict.invalid_words.is_empty());
        assert!(!verdict.checksum_valid);
        assert_eq!(
            verdict.error.as_deref(),
            Some("Invalid seed phrase checksum. Please verify your words.")
        );
    }

    #[test]
    fn the_count_is_reported_before_the_checksum() {
        let verdict = check(ENGLISH, Some("en"), 24);
        // Twelve filled slots do not finish a 24-word entry.
        assert!(!verdict.is_complete);
        assert_eq!(verdict.error, None);
        assert_eq!(verdict.length_warning, None);
    }

    #[test]
    fn entries_are_normalized_the_way_bip39_reads_them() {
        let verdict = check(" ABANDON  abandon ", Some("en"), 2);
        assert_eq!(
            verdict.words,
            vec!["abandon".to_string(), "abandon".to_string()]
        );
        assert!(verdict.invalid_words.is_empty());
    }

    #[test]
    fn the_chosen_language_decides_the_word_list() {
        let chinese = "的 的 的 的 的 的 的 的 的 的 的 在";
        assert!(check(chinese, Some("zh-Hans"), 12).invalid_words.is_empty());
        assert_eq!(check(chinese, Some("en"), 12).invalid_words.len(), 12);
        // No language chosen: the phrase is read in whatever language it is.
        assert!(check(chinese, None, 12).checksum_valid);
    }

    #[test]
    fn a_length_warning_is_about_the_picker_not_the_field() {
        assert!(check("abandon", Some("en"), 13).length_warning.is_some());
        assert!(check("abandon", Some("en"), 8).length_warning.is_some());
        assert!(check(ENGLISH, Some("en"), 12).length_warning.is_none());
    }
}

//! Every "is this input acceptable" rule core owns, in one module.
//!
//! This file holds the field rules — pure functions returning an
//! `Option<String>` the UI shows inline, where `None` means valid.
//! [`address`] holds the per-chain address and identifier rules, which
//! answer the same question about a much larger input.

pub mod address;

/// A BIP-39 phrase length, and the entropy a phrase of that length carries.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedPhraseLength {
    pub word_count: u32,
    pub entropy_bits: u32,
}

/// The five lengths BIP-39 defines, shortest first.
#[uniffi::export]
pub fn seed_phrase_lengths() -> Vec<SeedPhraseLength> {
    STANDARD_SEED_PHRASE_WORD_COUNTS
        .iter()
        .map(|&word_count| SeedPhraseLength {
            word_count,
            entropy_bits: entropy_bits_for(word_count),
        })
        .collect()
}

/// The entropy `word_count` words carry, or `None` when BIP-39 defines no
/// phrase of that length.
///
/// Checking for `None` is how a caller asks "is this a standard length"
/// without holding the list. Not exported: a front end reads
/// [`seed_phrase_lengths`], which answers both questions in one call.
pub(crate) fn seed_phrase_entropy_bits(word_count: u32) -> Option<u32> {
    STANDARD_SEED_PHRASE_WORD_COUNTS
        .contains(&word_count)
        .then(|| entropy_bits_for(word_count))
}

pub(crate) const STANDARD_SEED_PHRASE_WORD_COUNTS: [u32; 5] = [12, 15, 18, 21, 24];

/// BIP-39 spends 32 bits of entropy per three words, so the entropy is
/// derived rather than tabulated: 12 words carry 128 bits, 24 carry 256.
fn entropy_bits_for(word_count: u32) -> u32 {
    word_count / 3 * 32
}

/// One BIP-39 seed-phrase entry, as the user typed it.
///
/// `words` are the raw per-slot entries, blanks included: whether the grid is
/// finished is part of the verdict, so the caller hands over what it has
/// rather than deciding when to ask. `language` and `word_count` are the
/// user's overrides; `None` asks core to read each from the words, which is
/// what both the import page and a file-fed import want.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct SeedPhraseCheck {
    pub words: Vec<String>,
    /// A code from [`seed_phrase_languages`], or `None` to detect it.
    pub language: Option<String>,
    /// The length to judge the phrase at, or `None` to infer it.
    pub word_count: Option<u32>,
}

/// A BIP-39 wordlist, as a picker offers it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SeedPhraseLanguage {
    /// What [`SeedPhraseCheck::language`] takes.
    pub code: String,
    /// The English name; a front end localizes it.
    pub name: String,
}

/// Everything there is to say about a seed-phrase entry, decided once.
///
/// `problem` is the one thing to say under the field; the rest is what a UI
/// needs to size its grid, colour individual words and enable its button, and
/// is derived from the same pass so the parts can never disagree.
#[derive(uniffi::Record, Debug, Clone)]
pub struct SeedPhraseVerdict {
    /// The entries normalized the way BIP-39 reads them: trimmed, lowercased,
    /// blanks dropped.
    pub words: Vec<String>,
    /// The length the entry is judged at: the one fixed, or the shortest
    /// BIP-39 length that holds every filled slot, at least 12.
    pub word_count: u32,
    /// Whether `word_count` was inferred rather than fixed by the caller.
    pub word_count_inferred: bool,
    /// The wordlist the words were read in: the one chosen, or the one that
    /// holds the most of them. `None` until a word is in any list.
    pub language: Option<SeedPhraseLanguage>,
    /// Whether `language` was detected rather than chosen.
    pub language_detected: bool,
    /// Normalized words that are not in `language`'s list.
    pub invalid_words: Vec<String>,
    /// Every slot up to `word_count` holds something.
    pub is_complete: bool,
    /// The phrase parses, with its checksum, in `language`.
    pub checksum_valid: bool,
    /// What is wrong with the entry, or `None` while it is unfinished or
    /// already valid. A front end words it.
    pub problem: Option<SeedPhraseProblem>,
}

/// Why a seed-phrase entry is not a phrase, once there is something to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SeedPhraseProblem {
    /// The entry is judged at a length BIP-39 does not define: a fixed length
    /// outside 12, 15, 18, 21 and 24, or more than 24 words typed. No such
    /// phrase has a checksum that can hold.
    NonStandardLength { word_count: u32 },
    /// More words than the fixed length. They are kept, not cut, so the
    /// phrase is refused rather than silently shortened.
    WrongWordCount { expected: u32 },
    /// Every word is in the list but the checksum does not hold.
    InvalidChecksum,
}

/// Every BIP-39 wordlist, English first.
#[uniffi::export]
pub fn seed_phrase_languages() -> Vec<SeedPhraseLanguage> {
    bip39::Language::ALL
        .iter()
        .map(|&language| seed_phrase_language(language))
        .collect()
}

fn seed_phrase_language(language: bip39::Language) -> SeedPhraseLanguage {
    use bip39::Language as L;
    let (code, name) = match language {
        L::English => ("en", "English"),
        L::SimplifiedChinese => ("zh-hans", "Chinese (Simplified)"),
        L::TraditionalChinese => ("zh-hant", "Chinese (Traditional)"),
        L::Czech => ("cs", "Czech"),
        L::French => ("fr", "French"),
        L::Italian => ("it", "Italian"),
        L::Japanese => ("ja", "Japanese"),
        L::Korean => ("ko", "Korean"),
        L::Portuguese => ("pt", "Portuguese"),
        L::Spanish => ("es", "Spanish"),
    };
    SeedPhraseLanguage {
        code: code.to_string(),
        name: name.to_string(),
    }
}

/// The wordlist that holds the most of `words`, or `None` when none holds
/// any. A tie goes to a list the phrase parses in, then to `Language::ALL`
/// order: English and French share about a hundred words, and a phrase made
/// of the overlap is the one its checksum confirms.
fn detect_language(words: &[String], complete: bool) -> Option<bip39::Language> {
    let phrase = words.join(" ");
    bip39::Language::ALL
        .iter()
        .map(|&language| {
            let held = words
                .iter()
                .filter(|w| language.find_word(w).is_some())
                .count();
            let parses = complete && bip39::Mnemonic::parse_in(language, &phrase).is_ok();
            (language, held, parses)
        })
        .filter(|&(_, held, _)| held > 0)
        // `max_by_key` keeps the last of equals; reversing keeps the first.
        .rev()
        .max_by_key(|&(_, held, parses)| (held, parses))
        .map(|(language, _, _)| language)
}

/// Decide a seed-phrase entry: how long it is, which wordlist it is in,
/// which words are not in that list, whether the entry is finished, whether
/// the checksum holds, and what to say.
///
/// The order matters and is the reason this is one function. An unfinished
/// entry says nothing — every phrase is invalid until its last word. Words
/// that are not in the wordlist are their own message, so the count and the
/// checksum stay quiet behind them. Only then is a wrong count worth naming,
/// and only then is the checksum worth computing.
#[uniffi::export]
pub fn check_seed_phrase(check: SeedPhraseCheck) -> SeedPhraseVerdict {
    let filled_through = check
        .words
        .iter()
        .rposition(|w| !w.trim().is_empty())
        .map_or(0, |last| last + 1) as u32;
    let word_count = check.word_count.unwrap_or_else(|| {
        STANDARD_SEED_PHRASE_WORD_COUNTS
            .into_iter()
            .find(|&count| count >= filled_through)
            .unwrap_or(filled_through)
    });
    let expected = word_count as usize;
    let is_complete = expected > 0
        && check.words.len() >= expected
        && check.words[..expected].iter().all(|w| !w.trim().is_empty());
    let words: Vec<String> = check
        .words
        .iter()
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    let language = match check.language.as_deref() {
        Some(code) => Some(bip39_language(Some(code))),
        None => detect_language(&words, is_complete && words.len() == expected),
    };
    let invalid_words: Vec<String> = words
        .iter()
        .filter(|w| language.is_none_or(|language| language.find_word(w).is_none()))
        .cloned()
        .collect();

    let checksum_valid = is_complete
        && invalid_words.is_empty()
        && words.len() == expected
        && language
            .is_some_and(|language| bip39::Mnemonic::parse_in(language, words.join(" ")).is_ok());

    // A fixed length BIP-39 does not define can never hold, so it is named at
    // once; an inferred one only once the entry is finished.
    let standard = seed_phrase_entropy_bits(word_count).is_some();
    let problem = if !standard && check.word_count.is_some() {
        Some(SeedPhraseProblem::NonStandardLength { word_count })
    } else if !is_complete || !invalid_words.is_empty() {
        None
    } else if !standard {
        Some(SeedPhraseProblem::NonStandardLength { word_count })
    } else if words.len() != expected {
        Some(SeedPhraseProblem::WrongWordCount {
            expected: word_count,
        })
    } else if !checksum_valid {
        Some(SeedPhraseProblem::InvalidChecksum)
    } else {
        None
    };

    SeedPhraseVerdict {
        words,
        word_count,
        word_count_inferred: check.word_count.is_none(),
        language: language.map(seed_phrase_language),
        language_detected: check.language.is_none(),
        invalid_words,
        is_complete,
        checksum_valid,
        problem,
    }
}

/// Read a phrase as a mnemonic — in `language` when one was chosen, in
/// whichever BIP-39 language holds its words when none was.
///
/// Not `Mnemonic::from_str`: that refuses a phrase whose language it cannot
/// pin down, and the Simplified and Traditional Chinese lists overlap enough
/// that ordinary Chinese mnemonics are exactly that. A phrase that reads in
/// *some* language is a mnemonic, which is the question a caller without a
/// language picker is asking.
pub fn parse_seed_phrase(
    phrase: &str,
    language: Option<&str>,
) -> Result<bip39::Mnemonic, crate::derivation::error::DerivationError> {
    use crate::derivation::error::DerivationError;
    use bip39::{Language, Mnemonic};
    let phrase = phrase.trim();
    match language {
        Some(code) => {
            Mnemonic::parse_in(bip39_language(Some(code)), phrase).map_err(DerivationError::invalid)
        }
        None => Language::ALL
            .iter()
            .find_map(|lang| Mnemonic::parse_in(*lang, phrase).ok())
            .ok_or_else(|| {
                DerivationError::invalid(
                    Mnemonic::parse_in(Language::English, phrase)
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Not a BIP-39 mnemonic.".to_string()),
                )
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

/// Returns a typed rejection when `password` / `confirmation` fail the wallet
/// password rules, or `None` when both fields pass.
///
/// Rules:
///  * Both empty → valid (no password is allowed).
///  * Otherwise a password shorter than 4 characters, surrounding whitespace
///    excluded → error. A whitespace-only field is a blank password, which
///    core refuses to store, not the choice of none.
///  * Password and confirmation mismatch → error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletPasswordRejection {
    TooShort,
    ConfirmationMismatch,
}

#[uniffi::export]
pub fn validate_wallet_password(
    password: String,
    confirmation: String,
) -> Option<WalletPasswordRejection> {
    if password.is_empty() && confirmation.is_empty() {
        return None;
    }
    let p = password.trim();
    let c = confirmation.trim();
    if p.chars().count() < 4 {
        return Some(WalletPasswordRejection::TooShort);
    }
    if p != c {
        return Some(WalletPasswordRejection::ConfirmationMismatch);
    }
    None
}

#[cfg(test)]
mod seed_phrase_length_tests {
    use super::*;

    #[test]
    fn the_five_lengths_carry_the_entropy_bip39_defines() {
        let pairs: Vec<(u32, u32)> = seed_phrase_lengths()
            .into_iter()
            .map(|length| (length.word_count, length.entropy_bits))
            .collect();
        assert_eq!(
            pairs,
            vec![(12, 128), (15, 160), (18, 192), (21, 224), (24, 256)]
        );
    }

    #[test]
    fn a_length_bip39_does_not_define_has_no_entropy() {
        for word_count in [0, 1, 11, 13, 23, 25, 27, 48] {
            assert_eq!(seed_phrase_entropy_bits(word_count), None, "{word_count}");
        }
    }

    #[test]
    fn a_non_standard_length_is_refused_rather_than_substituted() {
        // Twelve words in answer to a request for eighteen is a weaker wallet
        // than the caller asked for.
        let refusal = crate::service::generate_mnemonic(13).expect_err("13 is not a length");
        assert!(refusal.to_string().contains("12, 15, 18, 21 or 24"));
        for length in seed_phrase_lengths() {
            let phrase = crate::service::generate_mnemonic(length.word_count)
                .expect("a standard length generates");
            assert_eq!(
                phrase.split_whitespace().count() as u32,
                length.word_count,
                "{} words requested",
                length.word_count
            );
        }
    }
}

#[cfg(test)]
mod seed_phrase_tests {
    use super::*;

    const ENGLISH: &str = "abandon abandon abandon abandon abandon abandon \
                           abandon abandon abandon abandon abandon about";

    fn check(words: &str, language: Option<&str>, expected: u32) -> SeedPhraseVerdict {
        check_seed_phrase(SeedPhraseCheck {
            words: words.split(' ').map(str::to_string).collect(),
            language: language.map(str::to_string),
            word_count: Some(expected),
        })
    }

    /// What the import page asks: no length, no language.
    fn infer(words: &str) -> SeedPhraseVerdict {
        check_seed_phrase(SeedPhraseCheck {
            words: words.split(' ').map(str::to_string).collect(),
            language: None,
            word_count: None,
        })
    }

    const ENGLISH_24: &str = "abandon abandon abandon abandon abandon abandon \
                              abandon abandon abandon abandon abandon abandon \
                              abandon abandon abandon abandon abandon abandon \
                              abandon abandon abandon abandon abandon art";

    #[test]
    fn the_length_is_the_shortest_one_that_holds_every_filled_slot() {
        assert_eq!(infer("").word_count, 12);
        assert_eq!(infer("abandon abandon").word_count, 12);
        let twelve = infer(ENGLISH);
        assert_eq!(twelve.word_count, 12);
        assert!(twelve.word_count_inferred && twelve.checksum_valid);
        // A gap still counts: the slot after it was filled.
        let mut slots = vec![String::new(); 13];
        slots[12] = "abandon".into();
        let gapped = check_seed_phrase(SeedPhraseCheck {
            words: slots,
            language: None,
            word_count: None,
        });
        assert_eq!(gapped.word_count, 15);
        assert!(!gapped.is_complete);
        assert_eq!(gapped.problem, None);
        let pasted = infer(&ENGLISH_24.split_whitespace().collect::<Vec<_>>().join(" "));
        assert_eq!(pasted.word_count, 24);
        assert!(pasted.checksum_valid, "{:?}", pasted.problem);
    }

    #[test]
    fn more_than_twenty_four_words_is_named_not_cut_short() {
        let phrase = format!(
            "{} abandon",
            ENGLISH_24.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        let verdict = infer(&phrase);
        assert_eq!(verdict.word_count, 25);
        assert_eq!(verdict.words.len(), 25);
        assert_eq!(
            verdict.problem,
            Some(SeedPhraseProblem::NonStandardLength { word_count: 25 })
        );
    }

    #[test]
    fn a_fixed_length_refuses_a_longer_phrase_rather_than_cutting_it() {
        let words = ENGLISH_24.split_whitespace().collect::<Vec<_>>().join(" ");
        let verdict = check(&words, None, 12);
        assert_eq!(verdict.words.len(), 24);
        assert!(!verdict.checksum_valid);
        assert_eq!(
            verdict.problem,
            Some(SeedPhraseProblem::WrongWordCount { expected: 12 })
        );
    }

    #[test]
    fn the_language_is_detected_from_the_words() {
        let english = infer(ENGLISH);
        assert_eq!(
            english.language.as_ref().map(|l| l.code.as_str()),
            Some("en")
        );
        assert!(english.language_detected);
        let chinese = infer("的 的 的 的 的 的 的 的 的 的 的 在");
        assert_eq!(
            chinese.language.as_ref().map(|l| l.code.as_str()),
            Some("zh-hans")
        );
        assert!(chinese.checksum_valid);
        assert!(infer("").language.is_none());
        assert!(infer("zzzz").language.is_none());
    }

    #[test]
    fn a_typo_is_named_against_the_detected_list() {
        let verdict = infer(&ENGLISH.replacen("abandon", "abandn", 1));
        assert_eq!(verdict.language.map(|l| l.code), Some("en".to_string()));
        assert_eq!(verdict.invalid_words, vec!["abandn".to_string()]);
        assert_eq!(verdict.problem, None);
    }

    #[test]
    fn a_phrase_mixing_wordlists_is_not_valid() {
        // "abandon" is English only; "的" is Chinese only.
        let mixed = ENGLISH.replacen("abandon", "的", 1);
        let verdict = infer(&mixed);
        assert_eq!(verdict.invalid_words, vec!["的".to_string()]);
        assert!(!verdict.checksum_valid);
    }

    #[test]
    fn every_wordlist_is_offered_with_a_code_the_check_reads() {
        let languages = seed_phrase_languages();
        assert_eq!(languages.len(), bip39::Language::ALL.len());
        assert_eq!(languages[0].code, "en");
        for (offered, language) in languages.iter().zip(bip39::Language::ALL) {
            assert_eq!(
                bip39_language(Some(&offered.code)),
                *language,
                "{}",
                offered.code
            );
        }
    }

    #[test]
    fn a_finished_valid_phrase_has_nothing_to_say() {
        let verdict = check(ENGLISH, Some("en"), 12);
        assert!(verdict.is_complete);
        assert!(verdict.checksum_valid);
        assert!(verdict.invalid_words.is_empty());
        assert_eq!(verdict.problem, None);
    }

    #[test]
    fn an_unfinished_entry_says_nothing_at_all() {
        // Blank tail slots: every phrase is invalid until its last word, so
        // reporting a checksum failure here would be shouting at someone
        // still typing.
        let verdict = check("abandon abandon   ", Some("en"), 12);
        assert!(!verdict.is_complete);
        assert!(!verdict.checksum_valid);
        assert_eq!(verdict.problem, None);
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
        assert_eq!(verdict.problem, None);
    }

    #[test]
    fn a_finished_phrase_with_a_broken_checksum_says_so() {
        let broken = ENGLISH.replace("about", "abandon");
        let verdict = check(&broken, Some("en"), 12);
        assert!(verdict.invalid_words.is_empty());
        assert!(!verdict.checksum_valid);
        assert_eq!(verdict.problem, Some(SeedPhraseProblem::InvalidChecksum));
    }

    #[test]
    fn the_count_is_reported_before_the_checksum() {
        let verdict = check(ENGLISH, Some("en"), 24);
        // Twelve filled slots do not finish a 24-word entry.
        assert!(!verdict.is_complete);
        assert_eq!(verdict.problem, None);
    }

    #[test]
    fn entries_are_normalized_the_way_bip39_reads_them() {
        let verdict = check(" ABANDON  abandon ", Some("en"), 12);
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
    fn a_fixed_length_bip39_does_not_define_is_named_before_any_typing() {
        for word_count in [0, 8, 13, 25] {
            assert_eq!(
                check("abandon", Some("en"), word_count).problem,
                Some(SeedPhraseProblem::NonStandardLength { word_count }),
            );
        }
        assert_eq!(check(ENGLISH, Some("en"), 12).problem, None);
    }
}

#[cfg(test)]
mod password_verdict_tests {
    use super::*;
    #[test]
    fn password_rejections_are_typed_and_count_unicode_characters() {
        for (password, confirmation, expected) in [
            ("", "", None),
            ("   ", " ", Some(WalletPasswordRejection::TooShort)),
            ("", "    ", Some(WalletPasswordRejection::TooShort)),
            ("abc", "abc", Some(WalletPasswordRejection::TooShort)),
            ("密碼", "密碼", Some(WalletPasswordRejection::TooShort)),
            ("密碼測試", "密碼測試", None),
            (
                "abcd",
                "abce",
                Some(WalletPasswordRejection::ConfirmationMismatch),
            ),
            (" abcd ", "abcd", None),
        ] {
            assert_eq!(
                validate_wallet_password(password.into(), confirmation.into()),
                expected
            );
        }
    }
}

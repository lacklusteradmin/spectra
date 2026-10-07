//! Polyseed: the 16-word Monero phrase Feather, Cake and current Monero
//! wallets create.
//!
//! Each word is an element of GF(2048); together they are a polynomial whose
//! value at 2 is zero. The first word is the checksum, the other fifteen carry
//! 10 secret bits each and one bit of a 15-bit field holding the wallet's
//! birthday (10 bits, months since November 2021) and feature flags (5 bits).
//! The key is PBKDF2-HMAC-SHA256 over the 150-bit secret, salted with the
//! coin, birthday and features. Ported from the reference implementation
//! (tevador/polyseed dd998e2), whose wordlists `core/data/wordlists/polyseed`
//! copies; vectors from that implementation are in
//! `core/tests/fixtures/phrase-formats.json`.

use std::collections::HashMap;
use std::sync::OnceLock;

use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::derivation::error::DerivationError;

const NUM_WORDS: usize = 16;
const SECRET_BITS: u32 = 150;
const DATE_BITS: u32 = 10;
const DATE_MASK: u32 = (1 << DATE_BITS) - 1;
/// The one feature bit current wallets set: the secret is masked with a
/// password-derived key.
const ENCRYPTED_MASK: u32 = 16;
/// Feature bits no wallet uses; a phrase with any of them set is refused.
const RESERVED_FEATURES: u32 = 0b11111 ^ ENCRYPTED_MASK;
/// 1 November 2021, 12:00 UTC, and one twelfth of a Gregorian year.
const EPOCH: u64 = 1_635_768_000;
const TIME_STEP: u64 = 2_629_746;
const KDF_ITERATIONS: u32 = 10_000;
/// Polyseed's coin number for Monero. It is XORed into the second word, so a
/// phrase made for another coin fails the checksum here.
const MONERO_COIN: u16 = 0;
const PREFIX_BYTES: usize = 4;

/// One Polyseed wordlist and how its words are matched.
pub(crate) struct PolyseedWordlist {
    pub code: &'static str,
    pub name: &'static str,
    /// A word may be shortened to any prefix of at least four bytes.
    has_prefix: bool,
    /// Accents are ignored when matching.
    has_accents: bool,
    /// Each word as it is matched: NFKD, accents dropped where ignored.
    keys: Vec<String>,
    by_key: HashMap<String, u16>,
}

impl PolyseedWordlist {
    fn new(
        code: &'static str,
        name: &'static str,
        has_prefix: bool,
        has_accents: bool,
        raw: &'static str,
    ) -> Self {
        let keys: Vec<String> = raw
            .lines()
            .map(|word| match_key(word, has_accents))
            .collect();
        assert_eq!(keys.len(), 2048, "{code}");
        let by_key = keys
            .iter()
            .enumerate()
            .map(|(index, key)| (key.clone(), index as u16))
            .collect();
        Self {
            code,
            name,
            has_prefix,
            has_accents,
            keys,
            by_key,
        }
    }

    /// The index `word` matches: the whole word, or in a prefix language
    /// any prefix of at least four bytes.
    pub fn index_of(&self, word: &str) -> Option<u16> {
        let key = match_key(word, self.has_accents);
        if let Some(&index) = self.by_key.get(&key) {
            return Some(index);
        }
        if !self.has_prefix || key.len() < PREFIX_BYTES {
            return None;
        }
        self.keys
            .iter()
            .position(|candidate| candidate.starts_with(&key))
            .map(|index| index as u16)
    }
}

/// A word as Polyseed compares it: NFKD, and without its combining marks in a
/// language that ignores accents.
fn match_key(word: &str, drop_accents: bool) -> String {
    let decomposed = word.trim().nfkd();
    if drop_accents {
        decomposed.filter(char::is_ascii).collect()
    } else {
        decomposed.collect()
    }
}

/// Every Polyseed wordlist, in the reference implementation's order.
pub(crate) fn wordlists() -> &'static [PolyseedWordlist] {
    static LISTS: OnceLock<Vec<PolyseedWordlist>> = OnceLock::new();
    LISTS.get_or_init(|| {
        macro_rules! list {
            ($code:literal, $name:literal, $prefix:literal, $accents:literal) => {
                PolyseedWordlist::new(
                    $code,
                    $name,
                    $prefix,
                    $accents,
                    include_str!(concat!("../../data/wordlists/polyseed/", $code, ".txt")),
                )
            };
        }
        vec![
            list!("en", "English", true, false),
            list!("ja", "Japanese", false, false),
            list!("ko", "Korean", false, false),
            list!("es", "Spanish", true, true),
            list!("fr", "French", true, true),
            list!("it", "Italian", true, false),
            list!("cs", "Czech", true, false),
            list!("pt", "Portuguese", true, false),
            list!("zh-hans", "Chinese (Simplified)", false, false),
            list!("zh-hant", "Chinese (Traditional)", false, false),
        ]
    })
}

/// Why sixteen words are not a usable Polyseed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolyseedProblem {
    /// No list holds every word.
    UnknownWords,
    /// The words read in two lists as different phrases.
    AmbiguousLanguage,
    InvalidChecksum,
    /// A reserved feature bit is set: a format revision this wallet does not
    /// know.
    UnsupportedFeatures,
    /// The secret is masked with a password, which Monero imports do not take.
    Encrypted,
}

/// The fields a valid phrase carries.
pub(crate) struct Polyseed {
    secret: Zeroizing<[u8; 32]>,
    birthday: u32,
    features: u32,
}

impl Polyseed {
    /// The approximate creation time, as a Unix timestamp: never later than
    /// the actual one.
    pub fn birthday_unix(&self) -> u64 {
        EPOCH + u64::from(self.birthday) * TIME_STEP
    }

    /// The 32-byte Monero key the phrase stands for.
    pub fn monero_key(&self) -> Zeroizing<[u8; 32]> {
        let mut salt = [0u8; 32];
        salt[..12].copy_from_slice(b"POLYSEED key");
        salt[13..16].fill(0xff);
        salt[16..20].copy_from_slice(&u32::from(MONERO_COIN).to_le_bytes());
        salt[20..24].copy_from_slice(&self.birthday.to_le_bytes());
        salt[24..28].copy_from_slice(&self.features.to_le_bytes());
        let mut key = Zeroizing::new([0u8; 32]);
        crate::kdf::pbkdf2_sha256(&*self.secret, &salt, KDF_ITERATIONS, &mut *key);
        key
    }
}

/// The list `words` read in, or why they are not a usable Polyseed.
pub(crate) fn read_words(
    words: &[&str],
) -> Result<
    (&'static PolyseedWordlist, Polyseed),
    (Option<&'static PolyseedWordlist>, PolyseedProblem),
> {
    if words.len() != NUM_WORDS {
        return Err((None, PolyseedProblem::UnknownWords));
    }
    let mut found: Option<(&'static PolyseedWordlist, Vec<u16>)> = None;
    for list in wordlists() {
        let Some(indices) = words
            .iter()
            .map(|word| list.index_of(word))
            .collect::<Option<Vec<u16>>>()
        else {
            continue;
        };
        match &found {
            None => found = Some((list, indices)),
            // Another list that reads the same phrase is the same seed.
            Some((_, first)) if *first == indices => {}
            Some((first, _)) => return Err((Some(first), PolyseedProblem::AmbiguousLanguage)),
        }
    }
    let (list, indices) = found.ok_or((None, PolyseedProblem::UnknownWords))?;
    let mut coefficients = [0u16; NUM_WORDS];
    coefficients.copy_from_slice(&indices);
    coefficients[1] ^= MONERO_COIN;
    if evaluate(&coefficients) != 0 {
        return Err((Some(list), PolyseedProblem::InvalidChecksum));
    }
    let seed = decode_coefficients(&coefficients);
    if seed.features & RESERVED_FEATURES != 0 {
        return Err((Some(list), PolyseedProblem::UnsupportedFeatures));
    }
    if seed.features & ENCRYPTED_MASK != 0 {
        return Err((Some(list), PolyseedProblem::Encrypted));
    }
    Ok((list, seed))
}

/// Read a 16-word phrase, refusing anything Monero's wallets would not
/// restore or that this wallet cannot.
pub(crate) fn decode(phrase: &str) -> Result<Polyseed, DerivationError> {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    read_words(&words).map(|(_, seed)| seed).map_err(|(_, problem)| {
        DerivationError::invalid(match problem {
            PolyseedProblem::UnknownWords => "These are not Polyseed words.",
            PolyseedProblem::AmbiguousLanguage => {
                "These words read as different Polyseed phrases in two languages."
            }
            PolyseedProblem::InvalidChecksum => "The Polyseed checksum does not match.",
            PolyseedProblem::UnsupportedFeatures => "This Polyseed uses features Spectra does not support.",
            PolyseedProblem::Encrypted => {
                "This Polyseed is encrypted with a password, which Spectra cannot take for Monero."
            }
        })
    })
}

/// Multiplication by 2 in GF(2048) as the reference implementation defines it.
fn mul2(x: u16) -> u16 {
    const TABLE: [u16; 8] = [5, 7, 1, 3, 13, 15, 9, 11];
    if x < 1024 {
        2 * x
    } else {
        TABLE[usize::from(x % 8)] + 16 * ((x - 1024) / 8)
    }
}

/// The polynomial's value at 2, by Horner's method; zero for a valid phrase.
fn evaluate(coefficients: &[u16; NUM_WORDS]) -> u16 {
    coefficients[..NUM_WORDS - 1]
        .iter()
        .rev()
        .fold(coefficients[NUM_WORDS - 1], |result, &coefficient| {
            mul2(result) ^ coefficient
        })
}

/// Unpack the fifteen data words: each holds 10 secret bits, most significant
/// first, then one bit of the birthday-and-features field.
fn decode_coefficients(coefficients: &[u16; NUM_WORDS]) -> Polyseed {
    let mut secret = Zeroizing::new([0u8; 32]);
    let mut extra: u32 = 0;
    let mut secret_index = 0usize;
    let mut secret_bits = 0u32;
    let mut total_bits = 0u32;
    for &coefficient in &coefficients[1..] {
        let mut word = u32::from(coefficient);
        extra = (extra << 1) | (word & 1);
        word >>= 1;
        let mut word_bits = 10u32;
        while word_bits > 0 {
            if secret_bits == 8 {
                secret_index += 1;
                total_bits += secret_bits;
                secret_bits = 0;
            }
            let chunk_bits = word_bits.min(8 - secret_bits);
            word_bits -= chunk_bits;
            if chunk_bits < 8 {
                secret[secret_index] <<= chunk_bits;
            }
            secret[secret_index] |= ((word >> word_bits) & ((1 << chunk_bits) - 1)) as u8;
            secret_bits += chunk_bits;
        }
    }
    debug_assert_eq!(total_bits + secret_bits, SECRET_BITS);
    Polyseed {
        secret,
        birthday: extra & DATE_MASK,
        features: extra >> DATE_BITS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference implementation's own test phrases (tests/tests.c), whose
    /// secret and salt it asserts: the key here is PBKDF2 over exactly those.
    const EN1: &str = "raven tail swear infant grief assist regular lamp duck valid someone little harsh puppy airport language";
    const EN_SHORTENED: &str =
        "rave tail swea infan grie assi regul lamp duck vali some litt hars pupp airp langua";

    #[test]
    fn the_reference_phrase_carries_the_reference_secret_and_birthday() {
        let seed = decode(EN1).unwrap();
        assert_eq!(
            hex::encode(&seed.secret[..]),
            "dd76e7359a0ded37cd0ff0f3c829a5ae01673300000000000000000000000000"
        );
        assert_eq!(seed.birthday, 1);
        assert_eq!(seed.features, 0);
        // Dec 2021: the reference asserts birthday <= creation < birthday + ~1 month.
        assert!(seed.birthday_unix() <= 1_638_446_400);
        assert!(seed.birthday_unix() + 2_630_000 > 1_638_446_400);
    }

    #[test]
    fn shortened_words_read_as_the_same_phrase() {
        assert_eq!(
            *decode(EN_SHORTENED).unwrap().monero_key(),
            *decode(EN1).unwrap().monero_key()
        );
    }

    #[test]
    fn a_wrong_word_fails_the_checksum() {
        let changed = EN1.replacen("tail", "tide", 1);
        assert!(matches!(
            read_words(&changed.split(' ').collect::<Vec<_>>()),
            Err((_, PolyseedProblem::InvalidChecksum))
        ));
    }

    /// tests.c `g_phrase_zh_mult`: every word is in both Chinese lists at the
    /// same index, so it is one phrase.
    #[test]
    fn chinese_in_both_scripts_at_the_same_indices_is_one_phrase() {
        let words: Vec<&str> = "殊 福 女 塞 答 追 看 拌 招 梯 享 箭 童 血 群 腿"
            .split(' ')
            .collect();
        let (list, seed) = read_words(&words).ok().unwrap();
        assert_eq!(list.code, "zh-hans");
        // The reference implementation's key for this phrase.
        assert_eq!(
            hex::encode(*seed.monero_key()),
            "ebc8945eb35355eb3bfa12be8c6a857a0530b00fb5f583e94940b1c6991234ad"
        );
    }

    /// tests.c `g_phrase_es_mult`: the shortened words read in Spanish and in
    /// another language as different phrases.
    #[test]
    fn words_that_read_as_two_phrases_are_refused() {
        let words: Vec<&str> =
            "impo sort usua cabi venu nobl oliv clim cont barr marc auto prod vaca torn fati"
                .split(' ')
                .collect();
        assert!(matches!(
            read_words(&words),
            Err((_, PolyseedProblem::AmbiguousLanguage))
        ));
    }
}

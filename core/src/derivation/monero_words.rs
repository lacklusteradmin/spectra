//! Monero's 25-word seed: the phrase monero-wallet-cli, the GUI and every
//! Monero wallet restore.
//!
//! 24 words encode the 32-byte spend key, three words to four little-endian
//! bytes over a 1626-word list; the 25th repeats the word a CRC-32 of the
//! first 24 words' prefixes picks. The rules here are `electrum-words.cpp`'s
//! (monero-project/monero f6a591c): every word is matched by its first
//! `prefix_len` characters, case-insensitively, the language is the first in
//! Monero's order whose words all match and whose checksum holds, and the
//! checksum runs over the matched list words' own prefixes. The wordlists in
//! `core/data/wordlists/monero` are copied from the same source.

use std::collections::HashMap;
use std::sync::OnceLock;

use zeroize::Zeroizing;

use crate::derivation::error::DerivationError;

const WORDS: u32 = 1626;

/// One Monero wordlist and the prefix length its words are matched by.
pub(crate) struct MoneroWordlist {
    pub code: &'static str,
    /// The English name, as Monero gives it.
    pub name: &'static str,
    prefix_len: usize,
    words: Vec<&'static str>,
    /// Lowercase prefix → index. A list with duplicate prefixes keeps the
    /// last index for each, as Monero's map does.
    by_prefix: HashMap<String, u32>,
}

impl MoneroWordlist {
    fn new(code: &'static str, name: &'static str, prefix_len: usize, raw: &'static str) -> Self {
        let words: Vec<&'static str> = raw.lines().collect();
        assert_eq!(words.len(), WORDS as usize, "{code}");
        let by_prefix = words
            .iter()
            .enumerate()
            .map(|(index, word)| (prefix(&word.to_lowercase(), prefix_len), index as u32))
            .collect();
        Self {
            code,
            name,
            prefix_len,
            words,
            by_prefix,
        }
    }

    /// The index `word` matches by its prefix, case-insensitively.
    pub fn index_of(&self, word: &str) -> Option<u32> {
        self.by_prefix
            .get(&prefix(&word.to_lowercase(), self.prefix_len))
            .copied()
    }

    pub fn word(&self, index: u32) -> &'static str {
        self.words[index as usize]
    }

    /// The index the checksum word must repeat, from the matched words'
    /// prefixes as the list spells them.
    fn checksum_index(&self, indices: &[u32]) -> usize {
        const CRC: crc::Crc<u32> = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
        let prefixes: String = indices
            .iter()
            .map(|&index| prefix(self.word(index), self.prefix_len))
            .collect();
        (CRC.checksum(prefixes.as_bytes()) % indices.len() as u32) as usize
    }
}

/// The first `count` characters of `word`.
fn prefix(word: &str, count: usize) -> String {
    word.chars().take(count).collect()
}

/// Every Monero wordlist, in the order Monero tries them.
pub(crate) fn wordlists() -> &'static [MoneroWordlist] {
    static LISTS: OnceLock<Vec<MoneroWordlist>> = OnceLock::new();
    LISTS.get_or_init(|| {
        macro_rules! list {
            ($code:literal, $name:literal, $prefix:literal) => {
                MoneroWordlist::new(
                    $code,
                    $name,
                    $prefix,
                    include_str!(concat!("../../data/wordlists/monero/", $code, ".txt")),
                )
            };
        }
        vec![
            list!("zh-hans", "Chinese (Simplified)", 1),
            list!("en", "English", 3),
            list!("nl", "Dutch", 4),
            list!("fr", "French", 4),
            list!("es", "Spanish", 4),
            list!("de", "German", 4),
            list!("it", "Italian", 4),
            list!("pt", "Portuguese", 4),
            list!("ja", "Japanese", 3),
            list!("ru", "Russian", 4),
            list!("eo", "Esperanto", 4),
            list!("jbo", "Lojban", 4),
            list!("en-old", "English (old)", 4),
        ]
    })
}

/// The list a 25-word phrase reads in, its words' indices, and whether its
/// checksum holds; `None` when no list holds every word.
pub(crate) fn read_words(words: &[&str]) -> Option<(&'static MoneroWordlist, Vec<u32>, bool)> {
    let mut fallback = None;
    for list in wordlists() {
        let Some(indices) = words
            .iter()
            .map(|word| list.index_of(word))
            .collect::<Option<Vec<u32>>>()
        else {
            continue;
        };
        let holds =
            indices.len() == 25 && indices[24] == indices[list.checksum_index(&indices[..24])];
        if holds {
            return Some((list, indices, true));
        }
        fallback.get_or_insert((list, indices, false));
    }
    fallback
}

/// Decode a 25-word Monero seed into the 32 bytes its words carry, refusing
/// a phrase whose words or checksum Monero would refuse.
pub(crate) fn decode(phrase: &str) -> Result<Zeroizing<[u8; 32]>, DerivationError> {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.len() != 25 {
        return Err(DerivationError::refused(
            "A Monero seed has 25 words, not %@.",
            [words.len()],
        ));
    }
    let (_, indices, holds) = read_words(&words)
        .ok_or_else(|| DerivationError::invalid("These are not Monero seed words."))?;
    if !holds {
        return Err(DerivationError::invalid(
            "The Monero seed's checksum word does not match.",
        ));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for group in 0..8 {
        let [w1, w2, w3] = [
            indices[group * 3],
            indices[group * 3 + 1],
            indices[group * 3 + 2],
        ];
        // Monero computes this in 32 bits and lets it wrap; a triple whose
        // value does not reproduce its first word is not one any key encodes.
        let value = w1
            .wrapping_add(WORDS.wrapping_mul((WORDS - w1 + w2) % WORDS))
            .wrapping_add(
                WORDS
                    .wrapping_mul(WORDS)
                    .wrapping_mul((WORDS - w2 + w3) % WORDS),
            );
        if value % WORDS != w1 {
            return Err(DerivationError::invalid(
                "These Monero seed words do not encode a key.",
            ));
        }
        key[group * 4..group * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(key)
}

/// The 25 English words for `key`, as monero-wallet-cli writes them.
pub(crate) fn encode(key: &[u8; 32]) -> Zeroizing<String> {
    let list = &wordlists()[1];
    debug_assert_eq!(list.code, "en");
    let mut indices = Vec::with_capacity(25);
    for chunk in key.as_chunks::<4>().0 {
        let value = u32::from_le_bytes(*chunk);
        let w1 = value % WORDS;
        let w2 = (value / WORDS + w1) % WORDS;
        let w3 = (value / WORDS / WORDS + w2) % WORDS;
        indices.extend([w1, w2, w3]);
    }
    indices.push(indices[list.checksum_index(&indices)]);
    Zeroizing::new(
        indices
            .iter()
            .map(|&index| list.word(index))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A new seed: a uniformly random spend key, reduced so the phrase restores
/// exactly the key it shows.
pub(crate) fn generate() -> Zeroizing<String> {
    use rand::RngCore;
    let mut wide = Zeroizing::new([0u8; 64]);
    rand::thread_rng().fill_bytes(&mut *wide);
    let key = Zeroizing::new(
        curve25519_dalek::scalar::Scalar::from_bytes_mod_order_wide(&wide).to_bytes(),
    );
    encode(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_list_has_unique_prefixes_except_the_old_english_one() {
        for list in wordlists() {
            let unique = list.by_prefix.len();
            if list.code == "en-old" {
                assert!(unique < WORDS as usize);
            } else {
                assert_eq!(unique, WORDS as usize, "{}", list.code);
            }
        }
    }

    #[test]
    fn an_encoded_key_decodes_to_itself_and_reads_as_english() {
        for _ in 0..32 {
            let phrase = generate();
            let key = decode(&phrase).unwrap();
            assert_eq!(*encode(&key), *phrase);
            let words: Vec<&str> = phrase.split(' ').collect();
            let (list, _, holds) = read_words(&words).unwrap();
            assert_eq!((list.code, holds), ("en", true));
        }
    }

    #[test]
    fn words_match_by_prefix_and_case() {
        let phrase = generate();
        let shouted: Vec<String> = phrase
            .split(' ')
            .map(|word| prefix(word, 3).to_uppercase())
            .collect();
        assert_eq!(
            *decode(&shouted.join(" ")).unwrap(),
            *decode(&phrase).unwrap()
        );
    }

    #[test]
    fn a_wrong_checksum_word_is_refused() {
        let phrase = generate();
        let mut words: Vec<&str> = phrase.split(' ').collect();
        let english = &wordlists()[1];
        let checksum = english.index_of(words[24]).unwrap();
        words[24] = english.word((checksum + 1) % WORDS);
        assert!(decode(&words.join(" ")).is_err());
        assert!(!read_words(&words).unwrap().2);
    }
}

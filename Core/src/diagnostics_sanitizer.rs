use bip39::Language;
use std::collections::HashSet;
use std::sync::OnceLock;

static BIP39_WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn bip39_word_set() -> &'static HashSet<&'static str> {
    BIP39_WORDS.get_or_init(|| Language::English.word_list().iter().copied().collect())
}

const EXT_PRIV_PREFIXES: &[&str] = &["xprv", "yprv", "zprv", "tprv", "uprv", "vprv"];
const BASE58_ALPHABET: &str =
    "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const MIN_SEED_WORD_SEQUENCE: usize = 12;

fn is_base58_char(c: char) -> bool {
    BASE58_ALPHABET.contains(c)
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// Redact xprv/yprv/zprv/tprv/uprv/vprv followed by >=32 base58 chars.
fn redact_extended_private_keys(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &input[i..];
        let matched_prefix = EXT_PRIV_PREFIXES
            .iter()
            .find(|prefix| remaining.starts_with(*prefix));
        let is_word_boundary_before =
            i == 0 || !is_word_char(input[..i].chars().next_back().unwrap_or(' '));
        if let (Some(prefix), true) = (matched_prefix, is_word_boundary_before) {
            let after_prefix = &remaining[prefix.len()..];
            let suffix_len: usize = after_prefix
                .chars()
                .take_while(|c| is_base58_char(*c))
                .map(|c| c.len_utf8())
                .sum();
            if suffix_len >= 32 {
                let end = i + prefix.len() + suffix_len;
                let next_char_is_word_boundary = input[end..]
                    .chars()
                    .next()
                    .map(|c| !is_word_char(c))
                    .unwrap_or(true);
                if next_char_is_word_boundary {
                    out.push_str("[REDACTED_EXTENDED_PRIVATE_KEY]");
                    i = end;
                    continue;
                }
            }
        }
        let ch = remaining.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Redact 64-char hex runs (optional 0x prefix) with word boundaries.
fn redact_hex_private_keys(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let is_word_boundary_before = i == 0 || !is_word_char(chars[i - 1]);
        if is_word_boundary_before {
            let has_prefix = i + 2 <= chars.len() && chars[i] == '0' && chars[i + 1] == 'x';
            let body_start = if has_prefix { i + 2 } else { i };
            if body_start + 64 <= chars.len()
                && (body_start..body_start + 64).all(|k| chars[k].is_ascii_hexdigit())
            {
                let end = body_start + 64;
                let next_is_boundary = end >= chars.len() || !is_word_char(chars[end]);
                if next_is_boundary {
                    out.push_str("[REDACTED_PRIVATE_KEY]");
                    i = end;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn redact_seed_word_sequences(input: &str) -> String {
    let words = bip39_word_set();
    let chars: Vec<char> = input.chars().collect();
    let mut word_spans: Vec<(usize, usize, bool)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            if i - start >= 2 {
                let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
                word_spans.push((start, i, words.contains(word.as_str())));
            }
        } else {
            i += 1;
        }
    }

    let mut sequences: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut current: Vec<(usize, usize)> = Vec::new();
    for (start, end, is_known) in &word_spans {
        if *is_known {
            current.push((*start, *end));
        } else {
            if current.len() >= MIN_SEED_WORD_SEQUENCE {
                sequences.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if current.len() >= MIN_SEED_WORD_SEQUENCE {
        sequences.push(current);
    }

    let mut redact_ranges: Vec<(usize, usize)> =
        sequences.into_iter().flatten().collect();
    redact_ranges.sort_by(|a, b| b.0.cmp(&a.0));

    let mut out: Vec<char> = chars;
    for (start, end) in redact_ranges {
        out.splice(start..end, "[REDACTED_SEED_WORD]".chars());
    }
    out.into_iter().collect()
}

pub fn sanitize_diagnostics_string(input: &str) -> String {
    let stage1 = redact_extended_private_keys(input);
    let stage2 = redact_hex_private_keys(&stage1);
    redact_seed_word_sequences(&stage2)
}

#[uniffi::export]
pub fn diagnostics_sanitize_string(input: String) -> String {
    sanitize_diagnostics_string(&input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_hex_private_key() {
        let input = "key=0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef end";
        let out = sanitize_diagnostics_string(input);
        assert!(out.contains("[REDACTED_PRIVATE_KEY]"));
        assert!(!out.contains("1234567890abcdef1234567890abcdef"));
    }

    #[test]
    fn redacts_extended_private_key() {
        let input = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
        let out = sanitize_diagnostics_string(input);
        assert!(out.contains("[REDACTED_EXTENDED_PRIVATE_KEY]"));
    }

    #[test]
    fn redacts_seed_word_sequence() {
        let input = "test test test test test test test test test test test junk";
        let out = sanitize_diagnostics_string(input);
        assert!(out.contains("[REDACTED_SEED_WORD]"));
        assert!(!out.contains("junk"));
    }

    #[test]
    fn leaves_short_sequence_alone() {
        let input = "abandon ability able about";
        let out = sanitize_diagnostics_string(input);
        assert_eq!(out, input);
    }
}

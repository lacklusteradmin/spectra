use crate::derivation::error::DerivationError;
use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

type HmacSha512 = Hmac<Sha512>;

pub(crate) type OptionalKeyMaterial = (Option<String>, Option<String>, Option<String>);

/// Map a wordlist name ("english", "zh-cn", etc.) to its BIP-39 language.
///
/// There is no default: a caller without a name reads the phrase in whichever
/// language holds it (`parse_mnemonic`), because an entropy-based derivation
/// under the wrong list refuses the phrase or reads different bytes from it.
fn resolve_bip39_language(name: &str) -> Result<Language, DerivationError> {
    match name.trim().to_ascii_lowercase().as_str() {
        "english" | "en" => Ok(Language::English),
        "czech" | "cs" => Ok(Language::Czech),
        "french" | "fr" => Ok(Language::French),
        "italian" | "it" => Ok(Language::Italian),
        "japanese" | "ja" | "jp" => Ok(Language::Japanese),
        "korean" | "ko" | "kr" => Ok(Language::Korean),
        "portuguese" | "pt" => Ok(Language::Portuguese),
        "spanish" | "es" => Ok(Language::Spanish),
        "simplified-chinese" | "chinese-simplified" | "simplified_chinese" | "zh-hans"
        | "zh-cn" | "zh" => Ok(Language::SimplifiedChinese),
        "traditional-chinese"
        | "chinese-traditional"
        | "traditional_chinese"
        | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(DerivationError::refused(
            "Unsupported mnemonic wordlist: %@",
            [other],
        )),
    }
}

/// Read a phrase under an explicitly chosen wordlist, or under whichever
/// BIP-39 language holds its words when the caller chose none.
///
/// A named wordlist that is not a BIP-39 language is still an error: it comes
/// from the Advanced-mode override field, and silently deriving under English
/// because of a typo there produces a different wallet.
///
/// Without a name the first language in `Language::ALL` that parses wins.
/// That is safe for derivations that read the entropy (Cardano, Substrate):
/// every word the Simplified and Traditional Chinese lists share sits at the
/// same index in both, so a phrase valid in both decodes to the same entropy.
/// English and French share 100 words at different indices, so a phrase made
/// only of those and checksum-valid in both is read as English — the reading
/// it had before languages were detected; naming the wordlist picks French.
pub(crate) fn parse_mnemonic(
    phrase: &str,
    wordlist: Option<&str>,
) -> Result<Mnemonic, DerivationError> {
    match wordlist {
        Some(name) if !name.trim().is_empty() => {
            Mnemonic::parse_in(resolve_bip39_language(name)?, phrase.trim())
                .map_err(DerivationError::invalid)
        }
        _ => crate::validation::parse_seed_phrase(phrase, None),
    }
}

/// The BIP-39 entropy a parsed phrase encodes, read from its word indices.
///
/// Not `Mnemonic::to_entropy`: bip39 2.2 re-detects the language from the
/// words there instead of using the one the phrase was parsed in, and panics
/// on a phrase valid in two lists — every all-shared-word Chinese phrase.
pub(crate) fn mnemonic_entropy(mnemonic: &Mnemonic) -> Zeroizing<Vec<u8>> {
    // 11 bits per word; the trailing word_count / 3 bits are the checksum.
    let entropy_len = mnemonic.word_count() / 3 * 4;
    let mut entropy = Zeroizing::new(Vec::with_capacity(entropy_len + 1));
    let mut acc: u32 = 0;
    let mut bits = 0;
    for index in mnemonic.word_indices() {
        acc = (acc << 11) | index as u32;
        bits += 11;
        while bits >= 8 {
            bits -= 8;
            entropy.push((acc >> bits) as u8);
        }
    }
    entropy.truncate(entropy_len);
    entropy
}

/// BIP-39 mnemonic -> 64-byte seed via NFKD normalization and PBKDF2-HMAC-SHA512.
pub(crate) fn derive_bip39_seed(
    seed_phrase: &str,
    passphrase: &str,
    iteration_count: u32,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
) -> Result<Zeroizing<[u8; 64]>, DerivationError> {
    let mnemonic = parse_mnemonic(seed_phrase, mnemonic_wordlist)?;
    let iterations = if iteration_count == 0 {
        2048
    } else {
        iteration_count
    };
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_mnemonic = Zeroizing::new(mnemonic.to_string().nfkd().collect::<String>());
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let mut seed = Zeroizing::new([0u8; 64]);
    crate::secrets::kdf::pbkdf2_sha512(
        normalized_mnemonic.as_bytes(),
        salt.as_bytes(),
        iterations,
        &mut *seed,
    );
    Ok(seed)
}

pub(crate) fn derive_substrate_mini_secret(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 32]>, DerivationError> {
    let entropy = mnemonic_entropy(&parse_mnemonic(mnemonic, wordlist)?);
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let iterations = if iteration_count == 0 {
        2048
    } else {
        iteration_count
    };
    let mut buf = Zeroizing::new([0u8; 64]);
    crate::secrets::kdf::pbkdf2_sha512(&entropy, salt.as_bytes(), iterations, &mut *buf);
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&buf[..32]);
    Ok(out)
}

/// The sr25519 signing key and public key a phrase derives along a Substrate
/// junction path (`substrate_path`; empty is the root key): the 32-byte seed
/// while every junction is hard, which a raw-key import reads too, and the
/// 64-byte expanded secret key after a soft junction, which no seed gives.
/// `uniform_expansion` expands the root seed in schnorrkel's Uniform mode
/// rather than the Ed25519 mode every Substrate wallet uses.
pub(crate) fn derive_substrate_sr25519_material(
    seed_phrase: &str,
    passphrase: &str,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
    derivation_path: Option<&str>,
    uniform_expansion: bool,
) -> Result<(Zeroizing<Vec<u8>>, [u8; 32]), DerivationError> {
    let junctions = super::substrate_path::parse(derivation_path.unwrap_or(""))?;
    let mini_secret = derive_substrate_mini_secret(
        seed_phrase,
        passphrase,
        mnemonic_wordlist,
        salt_prefix,
        iteration_count,
    )?;
    let mode = if uniform_expansion {
        schnorrkel::ExpansionMode::Uniform
    } else {
        schnorrkel::ExpansionMode::Ed25519
    };
    let (secret, seed) = super::substrate_path::derive(&mini_secret, mode, &junctions)?;
    let public_key = secret.to_public().to_bytes();
    let key = match seed {
        Some(seed) => Zeroizing::new(seed.to_vec()),
        None => Zeroizing::new(secret.to_bytes().to_vec()),
    };
    Ok((key, public_key))
}

fn ss58_prefix_bytes(network_prefix: u16) -> Vec<u8> {
    if network_prefix < 64 {
        vec![network_prefix as u8]
    } else {
        let lower = (network_prefix & 0b0000_0000_1111_1111) as u8;
        let upper = ((network_prefix & 0b0011_1111_0000_0000) >> 8) as u8;
        let first = ((lower & 0b1111_1100) >> 2) | ((upper & 0b0000_0011) << 6);
        let second = (lower & 0b0000_0011) | (upper & 0b1111_1100) | 0b0100_0000;
        vec![first | 0b0100_0000, second]
    }
}

fn ss58_prefix_from_bytes(decoded: &[u8]) -> Result<(u16, usize), DerivationError> {
    let Some(first) = decoded.first().copied() else {
        return Err(DerivationError::Invalid("ss58 empty payload".into()));
    };
    if first < 64 {
        return Ok((first as u16, 1));
    }
    let second = *decoded
        .get(1)
        .ok_or_else(|| DerivationError::Invalid("ss58 missing second prefix byte".into()))?;
    let lower = ((first & 0b0011_1111) << 2) | (second & 0b0000_0011);
    let upper = (second & 0b0011_1100) >> 2;
    Ok((((upper as u16) << 8) | lower as u16, 2))
}

fn ss58_checksum(payload_without_checksum: &[u8]) -> [u8; 2] {
    let checksum = blake2b::<64>(&[b"SS58PRE", payload_without_checksum]);
    [checksum[0], checksum[1]]
}

/// BLAKE2b with an `N`-byte digest of `chunks` in order: every plain BLAKE2b
/// in core, from Substrate's storage keys to Cardano's key hashes. A keyed
/// or personalized one names its parameters on `blake2b_simd` itself.
pub(crate) fn blake2b<const N: usize>(chunks: &[&[u8]]) -> [u8; N] {
    let mut state = blake2b_simd::Params::new().hash_length(N).to_state();
    for chunk in chunks {
        state.update(chunk);
    }
    state
        .finalize()
        .as_bytes()
        .try_into()
        .expect("the digest is N bytes")
}

pub(crate) fn encode_ss58(public_key: &[u8; 32], network_prefix: u16) -> String {
    let prefix_bytes = ss58_prefix_bytes(network_prefix);
    let mut payload = Vec::with_capacity(prefix_bytes.len() + 32 + 2);
    payload.extend_from_slice(&prefix_bytes);
    payload.extend_from_slice(public_key);

    let checksum = ss58_checksum(&payload);
    payload.extend_from_slice(&checksum);

    bs58::encode(payload).into_string()
}

pub(crate) fn decode_ss58(
    address: &str,
    expected_prefix: Option<u16>,
) -> Result<(u16, [u8; 32]), DerivationError> {
    let decoded = bs58::decode(address)
        .into_vec()
        .map_err(|e| DerivationError::Invalid(format!("ss58 decode: {e}").into()))?;
    let (prefix, key_start) = ss58_prefix_from_bytes(&decoded)?;
    if let Some(expected) = expected_prefix
        && prefix != expected
    {
        return Err(DerivationError::Invalid(
            format!("ss58 prefix: {prefix}").into(),
        ));
    }
    let checksum_start = key_start + 32;
    if decoded.len() != checksum_start + 2 {
        return Err(DerivationError::Invalid(
            format!("ss58 payload length: {}", decoded.len()).into(),
        ));
    }
    let checksum = ss58_checksum(&decoded[..checksum_start]);
    if decoded[checksum_start] != checksum[0] || decoded[checksum_start + 1] != checksum[1] {
        return Err(DerivationError::Invalid("ss58 checksum mismatch".into()));
    }
    let key_bytes: [u8; 32] = decoded[key_start..key_start + 32]
        .try_into()
        .map_err(|_| DerivationError::Invalid("ss58 key slice error".into()))?;
    Ok((prefix, key_bytes))
}

/// HMAC-SHA512 over concatenated chunks; returns a 64-byte `Zeroizing` buffer.
///
/// Aptos, Cardano, Internet Computer, Solana, Stellar, Sui and TON each had a
/// byte-identical copy of this and the two functions below.
pub(crate) fn hmac_sha512(
    key: &[u8],
    chunks: &[&[u8]],
) -> Result<Zeroizing<[u8; 64]>, DerivationError> {
    let mut mac = HmacSha512::new_from_slice(key)
        .map_err(|error| DerivationError::Internal(format!("Invalid HMAC-SHA512 key: {error}")))?;
    for chunk in chunks {
        mac.update(chunk);
    }
    let tag = mac.finalize().into_bytes();
    let mut out = Zeroizing::new([0u8; 64]);
    out.copy_from_slice(&tag);
    Ok(out)
}

/// A SLIP-10 path, every step hardened: SLIP-10 over ed25519 has no
/// unhardened derivation, so a step written without `'` still means
/// hardened. The notation is BIP-32's, read by its one reader.
pub(crate) fn parse_slip10_ed25519_path(path: &str) -> Result<Vec<u32>, DerivationError> {
    Ok(
        crate::derivation::bip32::indices(&crate::derivation::bip32::parse_path(path)?)
            .into_iter()
            .map(|index| index | crate::derivation::bip32::HARDENED_OFFSET)
            .collect(),
    )
}

/// Walk SLIP-10 hardened child derivation from seed to a 32-byte ed25519 key.
pub(crate) fn derive_slip10_ed25519_key(
    seed: &[u8],
    derivation_path: &str,
    hmac_key: Option<&str>,
) -> Result<Zeroizing<[u8; 32]>, DerivationError> {
    let key_bytes = hmac_key
        .filter(|value| !value.is_empty())
        .map(|value| value.as_bytes())
        .unwrap_or(b"ed25519 seed");
    let master = hmac_sha512(key_bytes, &[seed])?;
    let mut private_key = Zeroizing::new([0u8; 32]);
    let mut chain_code = Zeroizing::new([0u8; 32]);
    private_key.copy_from_slice(&master[..32]);
    chain_code.copy_from_slice(&master[32..]);
    for index in parse_slip10_ed25519_path(derivation_path)? {
        let index_bytes = index.to_be_bytes();
        let child = hmac_sha512(
            &*chain_code,
            &[&[0x00], &*private_key as &[u8], &index_bytes],
        )?;
        private_key.copy_from_slice(&child[..32]);
        chain_code.copy_from_slice(&child[32..]);
    }
    Ok(private_key)
}

#[cfg(test)]
mod mnemonic_language_tests {
    use super::*;

    // The all-zero entropy in two languages. Same entropy, different words —
    // and BIP-39 seeds from the words, so the two must not agree.
    const ENGLISH: &str = "abandon abandon abandon abandon abandon abandon \
                           abandon abandon abandon abandon abandon about";
    const CHINESE: &str = "的 的 的 的 的 的 的 的 的 的 的 在";

    fn seed(phrase: &str, wordlist: Option<&str>) -> Result<[u8; 64], DerivationError> {
        derive_bip39_seed(phrase, "", 0, wordlist, None).map(|s| *s)
    }

    #[test]
    fn a_phrase_with_no_wordlist_chosen_is_read_in_its_own_language() {
        // A mnemonic in any supported language parses, not only English.
        assert!(seed(CHINESE, None).is_ok());
        assert!(seed(ENGLISH, None).is_ok());
    }

    #[test]
    fn the_words_seed_the_wallet_not_the_entropy_they_encode() {
        assert_ne!(seed(CHINESE, None).unwrap(), seed(ENGLISH, None).unwrap());
    }

    #[test]
    fn a_named_wordlist_is_the_only_one_the_phrase_may_come_from() {
        assert!(seed(CHINESE, Some("zh-Hans")).is_ok());
        assert!(seed(CHINESE, Some("en")).is_err());
    }

    #[test]
    fn a_wordlist_that_is_not_a_language_is_an_error_not_a_fallback() {
        // It comes from the Advanced-mode override field; deriving under
        // English because of a typo there produces a different wallet.
        let error = seed(ENGLISH, Some("klingon")).unwrap_err();
        assert!(
            error.to_string().contains("Unsupported mnemonic wordlist"),
            "{error}"
        );
    }
}

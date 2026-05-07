//! Bittensor: SS58 address validation, BIP-39 → substrate-bip39 PBKDF2
//! mini-secret → sr25519 derivation, SS58 v1 address encoding.
//! Self-contained — see `REFACTOR_NOTES.md`.
//!
//! Bittensor wallets use the substrate-generic SS58 prefix (42), producing
//! addresses that start with `5…` and share the same length as Polkadot's
//! `1…` mainnet addresses. Wire-level the format is identical:
//!   `[prefix(1-2 bytes)] || [pubkey(32)] || [checksum(2)]`, base58-encoded.
//!
//! The 32-byte payload is treated as a sr25519 public key by the runtime;
//! Bittensor does not currently use the optional ECDSA path that the
//! generic SS58 envelope reserves.
//!
//! Substrate junction derivation (`//hard`, `/soft`) is not yet supported —
//! omit the derivation path to derive the root sr25519 keypair.

use bip39::{Language, Mnemonic};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;


// ── SS58 decoding (preserved) ────────────────────────────────────────────

pub(crate) fn decode_bittensor_ss58(address: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("bittensor ss58 decode: {e}"))?;
    if decoded.len() < 34 {
        return Err(format!("bittensor ss58 too short: {}", decoded.len()));
    }
    let key_start = if decoded[0] < 64 { 1 } else { 2 };
    let key_bytes: [u8; 32] = decoded[key_start..key_start + 32]
        .try_into()
        .map_err(|_| "bittensor ss58 key slice error".to_string())?;
    Ok(key_bytes)
}

pub fn validate_bittensor_address(address: &str) -> bool {
    decode_bittensor_ss58(address).is_ok()
}

// ── BIP-39 ───────────────────────────────────────────────────────────────

fn resolve_bip39_language(name: Option<&str>) -> Result<Language, String> {
    let value = match name {
        Some(value) if !value.trim().is_empty() => value.trim().to_ascii_lowercase(),
        _ => return Ok(Language::English),
    };
    match value.as_str() {
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
        "traditional-chinese" | "chinese-traditional" | "traditional_chinese" | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(format!("Unsupported mnemonic wordlist: {other}")),
    }
}

// ── substrate-bip39 mini-secret ──────────────────────────────────────────

fn derive_substrate_mini_secret(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 32]>, String> {
    let language = resolve_bip39_language(wordlist)?;
    let parsed =
        Mnemonic::parse_in_normalized(language, mnemonic).map_err(|e| e.to_string())?;
    let entropy = Zeroizing::new(parsed.to_entropy());
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let iterations = if iteration_count == 0 { 2048 } else { iteration_count };
    let mut buf = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(&entropy, salt.as_bytes(), iterations, &mut *buf);
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&buf[..32]);
    Ok(out)
}

pub(crate) fn derive_substrate_sr25519_material(
    seed_phrase: &str,
    passphrase: &str,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
    derivation_path: Option<&str>,
) -> Result<([u8; 32], [u8; 32]), String> {
    let path = derivation_path.unwrap_or("").trim();
    if !path.is_empty() && path != "m" && path != "M" {
        return Err(
            "Substrate junction derivation (//hard, /soft) is not yet supported; \
             omit the derivation path to derive the root sr25519 keypair."
                .to_string(),
        );
    }

    let mini_secret = derive_substrate_mini_secret(
        seed_phrase,
        passphrase,
        mnemonic_wordlist,
        salt_prefix,
        iteration_count,
    )?;

    let mini = schnorrkel::MiniSecretKey::from_bytes(&*mini_secret)
        .map_err(|e| format!("Invalid sr25519 mini-secret: {e}"))?;
    let keypair = mini.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    let public_key = keypair.public.to_bytes();

    let mut mini_out = [0u8; 32];
    mini_out.copy_from_slice(&*mini_secret);
    Ok((mini_out, public_key))
}

// ── SS58 v1 encoding ─────────────────────────────────────────────────────

fn encode_ss58(public_key: &[u8; 32], network_prefix: u16) -> String {
    use blake2::digest::consts::U64;
    use blake2::digest::Digest;
    use blake2::Blake2b;
    type Blake2b512 = Blake2b<U64>;

    let prefix_bytes: Vec<u8> = if network_prefix < 64 {
        vec![network_prefix as u8]
    } else {
        let lower = (network_prefix & 0b0000_0000_1111_1111) as u8;
        let upper = ((network_prefix & 0b0011_1111_0000_0000) >> 8) as u8;
        let first = ((lower & 0b1111_1100) >> 2) | ((upper & 0b0000_0011) << 6);
        let second = (lower & 0b0000_0011) | (upper & 0b1111_1100) | 0b0100_0000;
        vec![first | 0b0100_0000, second]
    };

    let mut payload = Vec::with_capacity(prefix_bytes.len() + 32 + 2);
    payload.extend_from_slice(&prefix_bytes);
    payload.extend_from_slice(public_key);

    let mut hasher = Blake2b512::new();
    hasher.update(b"SS58PRE");
    hasher.update(&payload);
    let checksum = hasher.finalize();
    payload.extend_from_slice(&checksum[..2]);

    bs58::encode(payload).into_string()
}

pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    passphrase: Option<&str>,
    path: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (mini_secret, public_key) = derive_substrate_sr25519_material(
        seed_phrase,
        passphrase.unwrap_or(""),
        None,
        None,
        0,
        path,
    )?;
    Ok((
        want_address.then(|| encode_ss58(&public_key, 42)),
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(mini_secret)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage() {
        assert!(!validate_bittensor_address(""));
        assert!(!validate_bittensor_address("not-a-bittensor-address"));
    }
}

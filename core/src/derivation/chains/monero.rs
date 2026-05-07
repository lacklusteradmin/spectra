//! Monero: address validation + BIP-39 derivation + chunked-base58
//! address encoding. Self-contained — see `REFACTOR_NOTES.md`.
//!
//! Monero's native Electrum-style 25-word seed (used by Cake/Monerujo) is
//! handled by wallet-rpc and is **not** what this module produces. The
//! routines here are for cross-chain BIP-39 wallets only:
//!   private_spend = sc_reduce32(BIP-39 seed[0..32])
//!   private_view  = sc_reduce32(Keccak256(private_spend))
//! Address encoding uses Monero's chunked Base58 with the chain-specific
//! network byte (0x12 = mainnet, 0x18 = stagenet).

use bip39::{Language, Mnemonic};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;


pub fn validate_monero_address(address: &str) -> bool {
    // Monero mainnet addresses start with '4' (standard) or '8' (subaddress)
    // and are 95 characters in base58 (Monero alphabet).
    if address.len() != 95 {
        return false;
    }
    let first = address.chars().next().unwrap_or('0');
    (first == '4' || first == '8')
        && address.chars().all(|c| {
            "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(c)
        })
}

/// Derive (private_spend, public_spend, private_view, public_view) from
/// a 32-byte BIP-39 seed prefix.
pub(crate) fn derive_monero_keys_from_spend_seed(
    spend_seed: &[u8; 32],
) -> Result<([u8; 32], [u8; 32], [u8; 32], [u8; 32]), String> {
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let private_spend = DalekScalar::from_bytes_mod_order(*spend_seed).to_bytes();

    use sha3::{Digest, Keccak256};
    let spend_hash: [u8; 32] = Keccak256::digest(&private_spend).into();
    let private_view = DalekScalar::from_bytes_mod_order(spend_hash).to_bytes();

    let public_spend = (DalekScalar::from_bytes_mod_order(private_spend)
        * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();
    let public_view = (DalekScalar::from_bytes_mod_order(private_view)
        * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();

    Ok((private_spend, public_spend, private_view, public_view))
}

/// Encode a Monero standard address: `network_byte || public_spend (32) ||
/// public_view (32) || keccak256(prev)[0..4]`, then chunked Base58.
/// Network byte: 0x12 for `Chain::Monero`, 0x18 for `Chain::MoneroStagenet`.
pub(crate) fn encode_monero_main_address(
    public_spend: &[u8; 32],
    public_view: &[u8; 32],
    is_mainnet: bool,
) -> Result<String, String> {
    let network_byte: u8 = if is_mainnet { 0x12 } else { 0x18 };
    let mut payload = Vec::with_capacity(69);
    payload.push(network_byte);
    payload.extend_from_slice(public_spend);
    payload.extend_from_slice(public_view);
    use sha3::{Digest, Keccak256};
    let digest: [u8; 32] = Keccak256::digest(&payload).into();
    payload.extend_from_slice(&digest[..4]);
    Ok(monero_base58_encode(&payload))
}

/// Monero's chunked Base58: split the input into 8-byte blocks, each
/// encoding to a fixed-width 11-char chunk. The alphabet differs in
/// ordering from BIP-58 but uses the same 58 characters.
pub(crate) fn monero_base58_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 58] =
        b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    const FULL_BLOCK_SIZE: usize = 8;
    const FULL_ENCODED_BLOCK_SIZE: usize = 11;
    const ENCODED_BLOCK_SIZES: [usize; FULL_BLOCK_SIZE + 1] = [0, 2, 3, 5, 6, 7, 9, 10, 11];

    let mut out = String::new();
    let full_blocks = data.len() / FULL_BLOCK_SIZE;
    let remainder = data.len() % FULL_BLOCK_SIZE;

    for i in 0..full_blocks {
        let start = i * FULL_BLOCK_SIZE;
        let block = &data[start..start + FULL_BLOCK_SIZE];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let mut chars = [b'1'; FULL_ENCODED_BLOCK_SIZE];
        for j in (0..FULL_ENCODED_BLOCK_SIZE).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    if remainder > 0 {
        let block = &data[full_blocks * FULL_BLOCK_SIZE..];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let encoded_len = ENCODED_BLOCK_SIZES[remainder];
        let mut chars = vec![b'1'; encoded_len];
        for j in (0..encoded_len).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    out
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

fn derive_bip39_seed(
    seed_phrase: &str,
    passphrase: &str,
    iteration_count: u32,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
) -> Result<Zeroizing<[u8; 64]>, String> {
    let language = resolve_bip39_language(mnemonic_wordlist)?;
    let mnemonic =
        Mnemonic::parse_in_normalized(language, seed_phrase).map_err(|e| e.to_string())?;
    let iterations = if iteration_count == 0 { 2048 } else { iteration_count };
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
    pbkdf2_hmac::<Sha512>(
        normalized_mnemonic.as_bytes(),
        salt.as_bytes(),
        iterations,
        &mut *seed,
    );
    Ok(seed)
}

pub(crate) fn derive_from_seed_phrase(
    is_mainnet: bool,
    seed_phrase: &str,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let seed = derive_bip39_seed(seed_phrase, "", 0, None, None)?;
    let mut spend_seed = [0u8; 32];
    spend_seed.copy_from_slice(&seed[..32]);
    let (private_spend, public_spend, _private_view, public_view) =
        derive_monero_keys_from_spend_seed(&spend_seed)?;

    let address = if want_address {
        Some(encode_monero_main_address(&public_spend, &public_view, is_mainnet)?)
    } else {
        None
    };

    let public_key_hex = if want_public_key {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&public_spend);
        both[32..].copy_from_slice(&public_view);
        Some(hex::encode(both))
    } else {
        None
    };

    Ok((
        address,
        public_key_hex,
        want_private_key.then(|| hex::encode(private_spend)),
    ))
}

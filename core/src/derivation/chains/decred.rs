//! Decred: address validation, BIP-32 derivation, P2PKH (Ds…) BLAKE-256
//! base58check encoding. Self-contained — see `REFACTOR_NOTES.md`.
//!
//! Decred uses BLAKE-256 (SHA-3 finalist family — NOT BLAKE2) wherever
//! Bitcoin uses double-SHA256:
//!   * `Hash160(pub) = RIPEMD-160(BLAKE-256(pub))` — address hash
//!   * Base58Check checksum = first 4 bytes of `BLAKE-256(BLAKE-256(payload))`
//!
//! Mainnet P2PKH addresses use the 2-byte version prefix `0x073F` (the
//! `Ds…` family). Testnet (`Ts…`) and simnet are out of scope.

use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use ripemd::{Digest as RipemdDigest, Ripemd160};
use secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::derivation::engine::{
    DerivedOutput, ParsedRequest, PublicKeyFormat, OUTPUT_ADDRESS, OUTPUT_PRIVATE_KEY,
    OUTPUT_PUBLIC_KEY,
};

pub(crate) const DCR_P2PKH_VERSION: [u8; 2] = [0x07, 0x3F];
pub(crate) const DCR_P2SH_VERSION: [u8; 2] = [0x07, 0x1A];

// ── BLAKE-256 (SHA-3 finalist BLAKE-1 family) ─────────────────────────────

const BLAKE256_IV: [u32; 8] = [
    0x6A09_E667, 0xBB67_AE85, 0x3C6E_F372, 0xA54F_F53A,
    0x510E_527F, 0x9B05_688C, 0x1F83_D9AB, 0x5BE0_CD19,
];

const BLAKE256_C: [u32; 16] = [
    0x243F_6A88, 0x85A3_08D3, 0x1319_8A2E, 0x0370_7344,
    0xA409_3822, 0x299F_31D0, 0x082E_FA98, 0xEC4E_6C89,
    0x4528_21E6, 0x38D0_1377, 0xBE54_66CF, 0x34E9_0C6C,
    0xC0AC_29B7, 0xC97C_50DD, 0x3F84_D5B5, 0xB547_0917,
];

const BLAKE256_SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

#[inline(always)]
fn g_mix(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, m: &[u32; 16], r: usize, e: usize) {
    let row = &BLAKE256_SIGMA[r % 10];
    state[a] = state[a]
        .wrapping_add(state[b])
        .wrapping_add(m[row[2 * e]] ^ BLAKE256_C[row[2 * e + 1]]);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a]
        .wrapping_add(state[b])
        .wrapping_add(m[row[2 * e + 1]] ^ BLAKE256_C[row[2 * e]]);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

fn blake256_compress(h: &mut [u32; 8], block: &[u8; 64], t0: u32, t1: u32) {
    let mut m = [0u32; 16];
    for i in 0..16 {
        m[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    let mut v = [0u32; 16];
    v[..8].copy_from_slice(h);
    v[8] = BLAKE256_C[0];
    v[9] = BLAKE256_C[1];
    v[10] = BLAKE256_C[2];
    v[11] = BLAKE256_C[3];
    v[12] = t0 ^ BLAKE256_C[4];
    v[13] = t0 ^ BLAKE256_C[5];
    v[14] = t1 ^ BLAKE256_C[6];
    v[15] = t1 ^ BLAKE256_C[7];

    for r in 0..14 {
        g_mix(&mut v, 0, 4, 8, 12, &m, r, 0);
        g_mix(&mut v, 1, 5, 9, 13, &m, r, 1);
        g_mix(&mut v, 2, 6, 10, 14, &m, r, 2);
        g_mix(&mut v, 3, 7, 11, 15, &m, r, 3);
        g_mix(&mut v, 0, 5, 10, 15, &m, r, 4);
        g_mix(&mut v, 1, 6, 11, 12, &m, r, 5);
        g_mix(&mut v, 2, 7, 8, 13, &m, r, 6);
        g_mix(&mut v, 3, 4, 9, 14, &m, r, 7);
    }

    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

/// Compute BLAKE-256 of `data`. Decred uses zero salt; we fold that constant
/// into the compression function to keep the public surface trivial.
pub(crate) fn blake256(data: &[u8]) -> [u8; 32] {
    let mut h = BLAKE256_IV;
    let total_bits: u64 = (data.len() as u64).wrapping_mul(8);
    let full_blocks = data.len() / 64;
    let mut t0: u32 = 0;
    let mut t1: u32 = 0;

    // Full-block compression. `t` tracks total bits consumed *including* the
    // current block.
    for i in 0..full_blocks {
        let mut block = [0u8; 64];
        block.copy_from_slice(&data[i * 64..(i + 1) * 64]);
        let (new_t0, carry) = t0.overflowing_add(512);
        t0 = new_t0;
        if carry {
            t1 = t1.wrapping_add(1);
        }
        blake256_compress(&mut h, &block, t0, t1);
    }

    // Padding rule (BLAKE-256, big-endian length suffix):
    //   message ++ 0x80 ++ zeros ++ 0x01 ++ length(8 bytes BE) so total ≡ 0 mod 512 bits
    let remaining = &data[full_blocks * 64..];
    let rem_len = remaining.len();
    let rem_bits = (rem_len as u32) * 8;

    let mut last = [0u8; 64];
    last[..rem_len].copy_from_slice(remaining);
    last[rem_len] = 0x80;

    if rem_len < 55 {
        // Single padding block. The bit counter for this block is the
        // total-message-bits-after-prefix, i.e., previous t plus rem_bits —
        // BUT if rem_len == 0 (no message bits in this block) we set t=0.
        let final_t0;
        let final_t1;
        if rem_len == 0 {
            final_t0 = 0;
            final_t1 = 0;
        } else {
            let (nt0, c) = t0.overflowing_add(rem_bits);
            final_t0 = nt0;
            final_t1 = if c { t1.wrapping_add(1) } else { t1 };
        }
        last[55] |= 0x01;
        last[56..64].copy_from_slice(&total_bits.to_be_bytes());
        blake256_compress(&mut h, &last, final_t0, final_t1);
    } else {
        // Two padding blocks. First block carries the message-bit accounting,
        // second block is length-only with t = 0.
        let (nt0, c) = t0.overflowing_add(rem_bits);
        let first_t0 = nt0;
        let first_t1 = if c { t1.wrapping_add(1) } else { t1 };
        blake256_compress(&mut h, &last, first_t0, first_t1);

        let mut tail = [0u8; 64];
        tail[55] |= 0x01;
        tail[56..64].copy_from_slice(&total_bits.to_be_bytes());
        blake256_compress(&mut h, &tail, 0, 0);
    }

    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// `RIPEMD-160(BLAKE-256(data))` — Decred's hash160 primitive.
pub(crate) fn dcr_hash160(data: &[u8]) -> [u8; 20] {
    let inner = blake256(data);
    let mut hasher = Ripemd160::new();
    RipemdDigest::update(&mut hasher, &inner);
    let out = hasher.finalize();
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&out);
    hash
}

/// Decred-flavoured base58check: 4-byte checksum is `BLAKE-256(BLAKE-256(payload))[..4]`,
/// not Bitcoin's `SHA-256(SHA-256(...))`.
pub(crate) fn dcr_base58check_encode(payload: &[u8]) -> String {
    let mut full = Vec::with_capacity(payload.len() + 4);
    full.extend_from_slice(payload);
    let hh = blake256(&blake256(payload));
    full.extend_from_slice(&hh[..4]);
    bs58::encode(full).into_string()
}

pub(crate) fn dcr_base58check_decode(input: &str) -> Result<Vec<u8>, String> {
    let raw = bs58::decode(input).into_vec().map_err(|e| format!("dcr base58 decode: {e}"))?;
    if raw.len() < 5 {
        return Err("dcr base58check payload too short".to_string());
    }
    let split = raw.len() - 4;
    let payload = &raw[..split];
    let checksum = &raw[split..];
    let expected = blake256(&blake256(payload));
    if &expected[..4] != checksum {
        return Err("dcr base58check checksum mismatch".to_string());
    }
    Ok(payload.to_vec())
}

/// Encode a `Ds…` (P2PKH) Decred address from a 20-byte pubkey hash.
pub(crate) fn encode_dcr_p2pkh(pubkey_hash: &[u8; 20]) -> String {
    let mut payload = Vec::with_capacity(22);
    payload.extend_from_slice(&DCR_P2PKH_VERSION);
    payload.extend_from_slice(pubkey_hash);
    dcr_base58check_encode(&payload)
}

/// Decode a Decred address into its 20-byte pubkey hash. Accepts both `Ds…`
/// (P2PKH) and `Dc…` (P2SH) forms; the payload is identical.
pub(crate) fn decode_dcr_address(address: &str) -> Result<[u8; 20], String> {
    let payload = dcr_base58check_decode(address)?;
    if payload.len() != 22 {
        return Err("dcr payload must be 22 bytes (2 version + 20 hash)".to_string());
    }
    let version = [payload[0], payload[1]];
    if version != DCR_P2PKH_VERSION && version != DCR_P2SH_VERSION {
        return Err(format!(
            "unrecognised dcr version bytes: {version:02x?}"
        ));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&payload[2..22]);
    Ok(hash)
}

/// Standard Decred P2PKH script: `OP_DUP OP_HASH160 <hash> OP_EQUALVERIFY OP_CHECKSIG`.
/// Identical opcodes to Bitcoin; the difference is the hash function used to
/// build the input hash, which the caller has already done.
pub(crate) fn dcr_p2pkh_script(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

pub fn validate_decred_address(address: &str) -> bool {
    decode_dcr_address(address).is_ok()
}

// ── BIP-39 ───────────────────────────────────────────────────────────────

type HmacSha512 = Hmac<Sha512>;

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

// ── BIP-32 ───────────────────────────────────────────────────────────────

const HARDENED_OFFSET: u32 = 0x80000000;

fn parse_bip32_path(path: &str) -> Result<Vec<u32>, String> {
    let trimmed = path.trim().trim_start_matches('m').trim_start_matches('M');
    let trimmed = trimmed.trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for segment in trimmed.split('/') {
        let (value, hardened) = if let Some(stripped) = segment.strip_suffix('\'') {
            (stripped, true)
        } else if let Some(stripped) = segment.strip_suffix('h') {
            (stripped, true)
        } else if let Some(stripped) = segment.strip_suffix('H') {
            (stripped, true)
        } else {
            (segment, false)
        };
        let raw: u32 = value
            .parse()
            .map_err(|_| format!("invalid path segment: {segment}"))?;
        if raw >= HARDENED_OFFSET {
            return Err(format!("path segment out of range: {segment}"));
        }
        out.push(if hardened { raw | HARDENED_OFFSET } else { raw });
    }
    Ok(out)
}

#[derive(Clone)]
struct ExtendedPrivateKey {
    private_key: SecretKey,
    chain_code: [u8; 32],
}

impl ExtendedPrivateKey {
    fn master_from_seed(hmac_key: &[u8], seed: &[u8]) -> Result<Self, String> {
        let mut mac =
            HmacSha512::new_from_slice(hmac_key).map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(seed);
        let tag = mac.finalize().into_bytes();
        let private_key = SecretKey::from_slice(&tag[..32])
            .map_err(|e| format!("Master key invalid: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self { private_key, chain_code })
    }

    fn derive_child(&self, secp: &Secp256k1<All>, index: u32) -> Result<Self, String> {
        let mut mac = HmacSha512::new_from_slice(&self.chain_code)
            .map_err(|e| format!("HMAC init: {e}"))?;
        if index >= HARDENED_OFFSET {
            mac.update(&[0x00]);
            mac.update(&self.private_key.secret_bytes());
        } else {
            let pk = PublicKey::from_secret_key(secp, &self.private_key);
            mac.update(&pk.serialize());
        }
        mac.update(&index.to_be_bytes());
        let tag = mac.finalize().into_bytes();
        let tweak = Scalar::from_be_bytes(
            tag[..32].try_into().map_err(|_| "tag slice".to_string())?,
        )
        .map_err(|_| "BIP-32 IL out of range".to_string())?;
        let private_key = self
            .private_key
            .add_tweak(&tweak)
            .map_err(|e| format!("BIP-32 tweak failed: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self { private_key, chain_code })
    }

    fn derive_path(&self, secp: &Secp256k1<All>, path: &[u32]) -> Result<Self, String> {
        let mut key = self.clone();
        for &index in path {
            key = key.derive_child(secp, index)?;
        }
        Ok(key)
    }
}

fn format_secp_public_key(
    public_key: &PublicKey,
    format: PublicKeyFormat,
) -> Result<Vec<u8>, String> {
    Ok(match format {
        PublicKeyFormat::Compressed => public_key.serialize().to_vec(),
        PublicKeyFormat::Uncompressed => public_key.serialize_uncompressed().to_vec(),
        PublicKeyFormat::XOnly => public_key.x_only_public_key().0.serialize().to_vec(),
        PublicKeyFormat::Raw => public_key.serialize().to_vec(),
        PublicKeyFormat::Auto => {
            return Err("Public key format must be explicit.".to_string());
        }
    })
}

fn requests_output(requested_outputs: u32, output: u32) -> bool {
    requested_outputs & output != 0
}

/// Derive a Decred Ds… P2PKH address.
pub(crate) fn derive(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let secp = Secp256k1::new();
    let derivation_path = request
        .derivation_path
        .clone()
        .ok_or("Derivation path is required.")?;
    let seed = derive_bip39_seed(
        &request.seed_phrase,
        &request.passphrase,
        request.iteration_count,
        request.mnemonic_wordlist.as_deref(),
        request.salt_prefix.as_deref(),
    )?;

    let key_bytes = request
        .hmac_key
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|v| v.as_bytes())
        .unwrap_or(b"Bitcoin seed");
    let master = ExtendedPrivateKey::master_from_seed(key_bytes, seed.as_ref())?;
    let path = parse_bip32_path(&derivation_path)?;
    let xpriv = master.derive_path(&secp, &path)?;
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    let private_bytes = xpriv.private_key.secret_bytes();

    let pubkey_hash = dcr_hash160(&public_key.serialize());
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(encode_dcr_p2pkh(&pubkey_hash))
    } else {
        None
    };

    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_bytes))
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake256_known_vectors() {
        // Empty string vector from BLAKE reference: BLAKE-256("") =
        // 716f6e863f744b9ac22c97ec7b76ea5f5908bc5b2f67c61510bfc4751384ea7a
        assert_eq!(
            hex::encode(blake256(b"")),
            "716f6e863f744b9ac22c97ec7b76ea5f5908bc5b2f67c61510bfc4751384ea7a"
        );
        // BLAKE-256("abc") = 1833a9fa7cf4086bd5fda73da32e5a1d75b4c3f89d5c436369f9d78bb2da5c28
        assert_eq!(
            hex::encode(blake256(b"abc")),
            "1833a9fa7cf4086bd5fda73da32e5a1d75b4c3f89d5c436369f9d78bb2da5c28"
        );
    }

    #[test]
    fn p2pkh_address_roundtrip() {
        let hash = [0x11u8; 20];
        let addr = encode_dcr_p2pkh(&hash);
        assert!(addr.starts_with("Ds"));
        let decoded = decode_dcr_address(&addr).unwrap();
        assert_eq!(decoded, hash);
    }

    #[test]
    fn rejects_garbage() {
        assert!(!validate_decred_address(""));
        assert!(!validate_decred_address("not-a-decred-address"));
        // Bitcoin P2PKH starts with "1" — wrong version byte for DCR.
        assert!(!validate_decred_address("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"));
    }
}

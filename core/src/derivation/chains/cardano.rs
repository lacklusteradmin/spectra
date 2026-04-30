//! Cardano: address validation, decoding, BIP-32-Ed25519 (Icarus / CIP-3 +
//! CIP-1852) key derivation, and CIP-19 Shelley enterprise address
//! encoding. Self-contained — see `REFACTOR_NOTES.md`.
//!
//! - Address validation accepts Shelley bech32 (`addr1` / `addr_test1`) and
//!   Byron base58.
//! - Derivation uses CIP-3 Icarus + CIP-1852: BIP-39 entropy → PBKDF2 root
//!   xprv → BIP-32-Ed25519 (Khovratovich-Law) child walk → ed25519 keypair.
//! - Address encoding uses CIP-19 Shelley enterprise (header type 6,
//!   payment key hash) bech32-encoded under HRP `addr` (mainnet) or
//!   `addr_test` (Cardano Preprod testnet).

use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use zeroize::Zeroizing;

use crate::derivation::engine::{
    DerivedOutput, ParsedRequest, OUTPUT_ADDRESS, OUTPUT_PRIVATE_KEY, OUTPUT_PUBLIC_KEY,
};
use crate::derivation::enums::Chain;

// ── Address validation + decoding (preserved) ────────────────────────────

pub fn validate_cardano_address(address: &str) -> bool {
    if address.starts_with("addr1") || address.starts_with("addr_test1") {
        return bech32::decode(address).is_ok();
    }
    bs58::decode(address).with_check(None).into_vec().is_ok()
}

pub(crate) fn decode_cardano_addr_bytes(address: &str) -> Result<Vec<u8>, String> {
    if address.starts_with("addr1") || address.starts_with("addr_test1") {
        bech32::decode(address)
            .map(|(_, data)| data)
            .map_err(|e| format!("cardano bech32 decode: {e}"))
    } else {
        let decoded = bs58::decode(address)
            .with_check(None)
            .into_vec()
            .map_err(|e| format!("cardano base58 decode: {e}"))?;
        Ok(decoded)
    }
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

// ── BIP-32 path parsing ──────────────────────────────────────────────────

fn parse_bip32_path_segments(path: &str) -> Result<Vec<u32>, String> {
    let trimmed = path.trim();
    let body = trimmed
        .strip_prefix("m/")
        .or_else(|| trimmed.strip_prefix("M/"))
        .unwrap_or_else(|| {
            if trimmed == "m" || trimmed == "M" {
                ""
            } else {
                trimmed
            }
        });
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for segment in body.split('/') {
        let seg = segment.trim();
        let (digits, hardened) = if let Some(s) = seg.strip_suffix('\'') {
            (s, true)
        } else if let Some(s) = seg.strip_suffix('h') {
            (s, true)
        } else {
            (seg, false)
        };
        let raw: u32 = digits
            .parse()
            .map_err(|_| format!("Invalid derivation path segment: {segment}"))?;
        if raw & 0x8000_0000 != 0 {
            return Err(format!("Derivation path segment out of range: {segment}"));
        }
        out.push(if hardened { raw | 0x8000_0000 } else { raw });
    }
    Ok(out)
}

// ── HMAC-SHA512 ──────────────────────────────────────────────────────────

fn hmac_sha512(key: &[u8], chunks: &[&[u8]]) -> Result<Zeroizing<[u8; 64]>, String> {
    let mut mac = HmacSha512::new_from_slice(key)
        .map_err(|error| format!("Invalid HMAC-SHA512 key: {error}"))?;
    for chunk in chunks {
        mac.update(chunk);
    }
    let tag = mac.finalize().into_bytes();
    let mut out = Zeroizing::new([0u8; 64]);
    out.copy_from_slice(&tag);
    Ok(out)
}

// ── BIP-32-Ed25519 (Khovratovich-Law) ────────────────────────────────────

pub(crate) fn derive_cardano_icarus_material(
    seed_phrase: &str,
    passphrase: &str,
    mnemonic_wordlist: Option<&str>,
    iteration_count: u32,
    derivation_path: Option<&str>,
) -> Result<([u8; 32], [u8; 32]), String> {
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let root = derive_cardano_icarus_xprv_root(
        seed_phrase,
        passphrase,
        mnemonic_wordlist,
        iteration_count,
    )?;
    let path = derivation_path
        .map(|s| s.to_string())
        .unwrap_or_else(|| "m/1852'/1815'/0'/0/0".to_string());

    let mut xprv: Zeroizing<[u8; 96]> = Zeroizing::new([0u8; 96]);
    xprv.copy_from_slice(&*root);
    for index in parse_bip32_path_segments(&path)? {
        xprv = cardano_icarus_derive_child(&xprv, index)?;
    }

    let mut private_key = [0u8; 32];
    private_key.copy_from_slice(&xprv[0..32]);

    // Public key = kL * G on Ed25519. kL is already Khovratovich-Law clamped,
    // so reducing mod ℓ does not change the group element.
    let mut scalar_bytes = [0u8; 32];
    scalar_bytes.copy_from_slice(&private_key);
    let scalar = DalekScalar::from_bytes_mod_order(scalar_bytes);
    let point = scalar * ED25519_BASEPOINT_POINT;
    let public_key = point.compress().to_bytes();

    Ok((private_key, public_key))
}

pub(crate) fn derive_cardano_icarus_xprv_root(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 96]>, String> {
    // CIP-3 Icarus / CIP-1852 root:
    //   entropy = BIP-39 entropy decoded from the mnemonic (not the PBKDF2
    //             seed; Daedalus uses a different legacy scheme)
    //   xprv    = PBKDF2-HMAC-SHA512(password = passphrase,
    //                                 salt = entropy,
    //                                 iterations = 4096,
    //                                 dklen = 96)
    //   Then clamp per Khovratovich-Law so kL is a valid ed25519 scalar
    //   multiple of 8 and < 2^254.
    let language = resolve_bip39_language(wordlist)?;
    let parsed =
        Mnemonic::parse_in_normalized(language, mnemonic).map_err(|e| e.to_string())?;
    let entropy = Zeroizing::new(parsed.to_entropy());
    let iterations = if iteration_count == 0 { 4096 } else { iteration_count };
    let mut xprv = Zeroizing::new([0u8; 96]);
    pbkdf2_hmac::<Sha512>(passphrase.as_bytes(), &entropy, iterations, &mut *xprv);
    xprv[0] &= 0b1111_1000;
    xprv[31] &= 0b0001_1111;
    xprv[31] |= 0b0100_0000;
    Ok(xprv)
}

fn cardano_icarus_derive_child(
    xprv: &[u8; 96],
    index: u32,
) -> Result<Zeroizing<[u8; 96]>, String> {
    // BIP-32-Ed25519 (Khovratovich-Law) child key derivation.
    //   xprv = kL (32) || kR (32) || chain_code (32)
    //   hardened (i >= 2^31):
    //     Z  = HMAC-SHA512(chain_code, 0x00 || kL || kR || i_LE)
    //     cc = HMAC-SHA512(chain_code, 0x01 || kL || kR || i_LE)[32..64]
    //   soft:
    //     A  = compressed(kL * G)   // ed25519 public point
    //     Z  = HMAC-SHA512(chain_code, 0x02 || A || i_LE)
    //     cc = HMAC-SHA512(chain_code, 0x03 || A || i_LE)[32..64]
    //   child_kL = parent_kL + 8 * ZL_28  (256-bit LE, overflow discarded)
    //   child_kR = parent_kR + ZR          (256-bit LE, overflow discarded)
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let kl = &xprv[0..32];
    let kr = &xprv[32..64];
    let cc = &xprv[64..96];
    let hardened = index >= 0x8000_0000;
    let i_le = index.to_le_bytes();

    let (z_tag, cc_tag): (u8, u8) = if hardened { (0x00, 0x01) } else { (0x02, 0x03) };

    let a_compressed = if hardened {
        [0u8; 32]
    } else {
        let mut scalar_bytes = [0u8; 32];
        scalar_bytes.copy_from_slice(kl);
        let scalar = DalekScalar::from_bytes_mod_order(scalar_bytes);
        (scalar * ED25519_BASEPOINT_POINT).compress().to_bytes()
    };

    let z = if hardened {
        hmac_sha512(cc, &[&[z_tag], kl, kr, &i_le])?
    } else {
        hmac_sha512(cc, &[&[z_tag], &a_compressed, &i_le])?
    };
    let child_cc_full = if hardened {
        hmac_sha512(cc, &[&[cc_tag], kl, kr, &i_le])?
    } else {
        hmac_sha512(cc, &[&[cc_tag], &a_compressed, &i_le])?
    };

    let zl_28 = &z[0..28];
    let zr = &z[32..64];

    // 8 * ZL_28 as a 32-byte little-endian integer.
    let mut eight_zl = [0u8; 32];
    let mut carry: u16 = 0;
    for (dst, &src) in eight_zl.iter_mut().zip(zl_28.iter()) {
        let v = (src as u16) * 8 + carry;
        *dst = (v & 0xff) as u8;
        carry = v >> 8;
    }
    if carry > 0 {
        eight_zl[28] = carry as u8;
    }

    let mut child_xprv = Zeroizing::new([0u8; 96]);
    let mut carry: u16 = 0;
    for i in 0..32 {
        let v = (kl[i] as u16) + (eight_zl[i] as u16) + carry;
        child_xprv[i] = (v & 0xff) as u8;
        carry = v >> 8;
    }
    let mut carry: u16 = 0;
    for i in 0..32 {
        let v = (kr[i] as u16) + (zr[i] as u16) + carry;
        child_xprv[32 + i] = (v & 0xff) as u8;
        carry = v >> 8;
    }
    child_xprv[64..96].copy_from_slice(&child_cc_full[32..64]);
    Ok(child_xprv)
}

// ── CIP-19 Shelley enterprise address ────────────────────────────────────

pub(crate) fn derive_cardano_shelley_enterprise_address(
    public_key: &[u8; 32],
    chain: Chain,
) -> Result<String, String> {
    use blake2::digest::consts::U28;
    use blake2::digest::Digest;
    use blake2::Blake2b;
    type Blake2b224 = Blake2b<U28>;

    let mut hasher = Blake2b224::new();
    hasher.update(public_key);
    let payment_hash = hasher.finalize();

    let is_mainnet = matches!(chain, Chain::Cardano);
    let network_id: u8 = if is_mainnet { 1 } else { 0 };
    let header = 0x60 | network_id;

    let mut payload = Vec::with_capacity(29);
    payload.push(header);
    payload.extend_from_slice(&payment_hash);

    let hrp_str = if is_mainnet { "addr" } else { "addr_test" };
    let hrp = bech32::Hrp::parse(hrp_str).map_err(|e| e.to_string())?;
    bech32::encode::<bech32::Bech32>(hrp, &payload).map_err(|e| e.to_string())
}

fn requests_output(requested_outputs: u32, output: u32) -> bool {
    requested_outputs & output != 0
}

/// Derive a Cardano Shelley enterprise address from BIP-39 + Icarus
/// BIP-32-Ed25519 + CIP-19. Network is selected by `Chain::Cardano`
/// (mainnet) vs `Chain::CardanoPreprod` (testnet).
pub(crate) fn derive(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_cardano_icarus_material(
        &request.seed_phrase,
        &request.passphrase,
        request.mnemonic_wordlist.as_deref(),
        request.iteration_count,
        request.derivation_path.as_deref(),
    )?;
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(derive_cardano_shelley_enterprise_address(
            &public_key,
            request.chain,
        )?)
    } else {
        None
    };
    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

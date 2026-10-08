//! Cardano: address validation, decoding, BIP-32-Ed25519 (Icarus / CIP-3 +
//! CIP-1852) key derivation, and CIP-19 Shelley address encoding
//!
//! - Address validation accepts Shelley bech32 (`addr1` / `addr_test1`) and
//!   Byron base58.
//! - Derivation uses CIP-3 Icarus + CIP-1852: BIP-39 entropy → PBKDF2 root
//!   xprv → BIP-32-Ed25519 (Khovratovich-Law) child walk → ed25519 keypair.
//! - A phrase derives a CIP-19 base address (header type 0): the payment key
//!   hash, then the hash of its account's stake key at role 2, index 0, as
//!   every mainstream wallet derives it. A raw extended key holds no stake
//!   key and derives the enterprise address (header type 6). Both are
//!   bech32-encoded under HRP `addr` (mainnet) or `addr_test` (Cardano
//!   Preprod testnet).

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{mnemonic_entropy, parse_mnemonic};
use zeroize::Zeroizing;

// ── Address validation + decoding (preserved) ────────────────────────────

// Decode a Cardano address: bech32 for Shelley (addr1/addr_test1), base58check for Byron.
pub(crate) fn decode_cardano_addr_bytes(address: &str) -> Result<Vec<u8>, DerivationError> {
    if address.starts_with("addr1") || address.starts_with("addr_test1") {
        bech32::decode(address)
            .map(|(_, data)| data)
            .map_err(|e| DerivationError::Invalid(format!("cardano bech32 decode: {e}").into()))
    } else {
        let decoded = bs58::decode(address)
            .with_check(None)
            .into_vec()
            .map_err(|e| DerivationError::Invalid(format!("cardano base58 decode: {e}").into()))?;
        Ok(decoded)
    }
}

// ── BIP-32 path parsing ──────────────────────────────────────────────────

// Parse a BIP-32 derivation path string into a list of child index integers (hardened or soft).
fn parse_bip32_path_segments(path: &str) -> Result<Vec<u32>, DerivationError> {
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
        let raw: u32 = digits.parse().map_err(|_| {
            DerivationError::refused("Invalid derivation path segment: %@", [segment])
        })?;
        if raw & 0x8000_0000 != 0 {
            return Err(DerivationError::refused(
                "Derivation path segment out of range: %@",
                [segment],
            ));
        }
        out.push(if hardened { raw | 0x8000_0000 } else { raw });
    }
    Ok(out)
}

// ── BIP-32-Ed25519 (Khovratovich-Law) ────────────────────────────────────

const HARDENED: u32 = 0x8000_0000;

/// The default payment path: account 0's first external address.
const DEFAULT_PATH: &str = "m/1852'/1815'/0'/0/0";

// Walk `segments` down from a 96-byte root xprv; return the extended key and its public key.
fn derive_icarus_child_key(
    root: &[u8; 96],
    segments: &[u32],
) -> Result<([u8; 64], [u8; 32]), DerivationError> {
    let mut xprv: Zeroizing<[u8; 96]> = Zeroizing::new(*root);
    for index in segments {
        xprv = cardano_icarus_derive_child(&xprv, *index)?;
    }
    let mut private_key = [0u8; 64];
    private_key.copy_from_slice(&xprv[0..64]);
    let public_key = public_from_extended_key(&private_key)?;
    Ok((private_key, public_key))
}

/// The stake key path of a CIP-1852 payment path: its account's key at
/// role 2, index 0. Every address of an account shares that one stake key,
/// so a path of any other shape names none and is refused.
fn stake_key_path(payment: &[u32]) -> Result<[u32; 5], DerivationError> {
    match *payment {
        [purpose, coin, account, role, index]
            if purpose == 1852 | HARDENED
                && coin == 1815 | HARDENED
                && account & HARDENED != 0
                && role <= 1
                && index & HARDENED == 0 =>
        {
            Ok([purpose, coin, account, 2, 0])
        }
        _ => Err(DerivationError::invalid(
            "A Cardano path follows CIP-1852: m/1852'/1815'/account'/role/index, with role 0 or 1",
        )),
    }
}

/// A phrase's payment key at `path` and the public stake key of its
/// account: the two keys a base address names.
pub(crate) fn derive_cardano_base_keys(
    seed_phrase: &str,
    passphrase: &str,
    derivation_path: &str,
) -> Result<([u8; 64], [u8; 32], [u8; 32]), DerivationError> {
    let payment_path = parse_bip32_path_segments(derivation_path)?;
    let stake_path = stake_key_path(&payment_path)?;
    let root = derive_cardano_icarus_xprv_root(seed_phrase, passphrase, None, 0)?;
    let (private_key, payment_public) = derive_icarus_child_key(&root, &payment_path)?;
    let (_, stake_public) = derive_icarus_child_key(&root, &stake_path)?;
    Ok((private_key, payment_public, stake_public))
}

/// Public key of a validated Cardano extended signing scalar and nonce prefix.
pub(crate) fn public_from_extended_key(key: &[u8; 64]) -> Result<[u8; 32], DerivationError> {
    use curve25519_dalek::{constants::ED25519_BASEPOINT_POINT, scalar::Scalar};
    let scalar_bytes =
        Zeroizing::new(<[u8; 32]>::try_from(&key[..32]).expect("fixed extended key"));
    if scalar_bytes[0] & 7 != 0 || scalar_bytes[31] & 0xc0 != 0x40 {
        return Err(DerivationError::invalid("Invalid Cardano extended scalar"));
    }
    let scalar = Zeroizing::new(Scalar::from_bytes_mod_order(*scalar_bytes));
    Ok((*scalar * ED25519_BASEPOINT_POINT).compress().to_bytes())
}

// CIP-3 Icarus root xprv: PBKDF2-HMAC-SHA512(passphrase, entropy, 4096, 96) then Khovratovich-Law clamp.
pub(crate) fn derive_cardano_icarus_xprv_root(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 96]>, DerivationError> {
    // CIP-3 Icarus / CIP-1852 root:
    //   entropy = BIP-39 entropy decoded from the mnemonic (not the PBKDF2
    //             seed; Daedalus uses a different legacy scheme)
    //   xprv    = PBKDF2-HMAC-SHA512(password = passphrase,
    //                                 salt = entropy,
    //                                 iterations = 4096,
    //                                 dklen = 96)
    //   Then clamp per Khovratovich-Law so kL is a valid ed25519 scalar
    //   multiple of 8 and < 2^254.
    // The entropy depends on the wordlist, so with none given the phrase is
    // read in its own language rather than assumed to be English.
    let entropy = mnemonic_entropy(&parse_mnemonic(mnemonic, wordlist)?);
    let iterations = if iteration_count == 0 {
        4096
    } else {
        iteration_count
    };
    let mut xprv = Zeroizing::new([0u8; 96]);
    crate::kdf::pbkdf2_sha512(passphrase.as_bytes(), &entropy, iterations, &mut *xprv);
    xprv[0] &= 0b1111_1000;
    xprv[31] &= 0b0001_1111;
    xprv[31] |= 0b0100_0000;
    Ok(xprv)
}

// BIP-32-Ed25519 (Khovratovich-Law) one-step child key derivation from a 96-byte xprv.
fn cardano_icarus_derive_child(
    xprv: &[u8; 96],
    index: u32,
) -> Result<Zeroizing<[u8; 96]>, DerivationError> {
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

// Blake2b-224 of a public key: the key hash a Shelley address carries.
fn key_hash(public_key: &[u8; 32]) -> [u8; 28] {
    use blake2::Blake2b;
    use blake2::digest::Digest;
    use blake2::digest::consts::U28;
    Blake2b::<U28>::digest(public_key).into()
}

// A CIP-19 Shelley address of key credentials: the header (type, network),
// then each key's hash, bech32-encoded.
fn shelley_key_address(
    address_type: u8,
    keys: &[&[u8; 32]],
    is_mainnet: bool,
) -> Result<String, DerivationError> {
    let mut payload = Vec::with_capacity(1 + 28 * keys.len());
    payload.push(address_type << 4 | u8::from(is_mainnet));
    for key in keys {
        payload.extend_from_slice(&key_hash(key));
    }
    let hrp_str = if is_mainnet { "addr" } else { "addr_test" };
    let hrp = bech32::Hrp::parse(hrp_str).map_err(DerivationError::invalid)?;
    bech32::encode::<bech32::Bech32>(hrp, &payload).map_err(DerivationError::invalid)
}

/// A CIP-19 Shelley enterprise address (type 6): the payment key hash
/// alone. What a raw extended key, which holds no stake key, derives.
pub(crate) fn derive_cardano_shelley_enterprise_address(
    public_key: &[u8; 32],
    is_mainnet: bool,
) -> Result<String, DerivationError> {
    shelley_key_address(6, &[public_key], is_mainnet)
}

/// A CIP-19 Shelley base address (type 0): the payment key hash, then the
/// stake key hash.
pub(crate) fn cardano_base_address(
    payment_public: &[u8; 32],
    stake_public: &[u8; 32],
    is_mainnet: bool,
) -> Result<String, DerivationError> {
    shelley_key_address(0, &[payment_public, stake_public], is_mainnet)
}

// Derive a phrase's Cardano base address, payment public key and payment
// private key via CIP-3 Icarus + BIP-32-Ed25519.
pub(crate) fn derive_from_seed_phrase(
    mainnet: bool,
    seed_phrase: &str,
    derivation_path: Option<&str>,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
    let (private_key, public_key, stake_public) = derive_cardano_base_keys(
        seed_phrase,
        passphrase.unwrap_or(""),
        derivation_path.unwrap_or(DEFAULT_PATH),
    )?;
    let address = if want_address {
        Some(cardano_base_address(&public_key, &stake_public, mainnet)?)
    } else {
        None
    };
    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(private_key)),
    ))
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::SpectraBridgeError;
use crate::derivation::primitives::hmac_sha512;
use crate::derivation::types::{DerivationResult, parse_path_metadata};

// Shared derivation logic for Cardano networks; mainnet flag selects addr/addr_test HRP.
fn cardano_internal(
    mainnet: bool,
    seed_phrase: String,
    derivation_path: Option<String>,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = derivation_path
        .as_deref()
        .map(parse_path_metadata)
        .unwrap_or((0, 0, 0));
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        mainnet,
        &seed_phrase,
        derivation_path.as_deref(),
        passphrase.as_deref(),
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account,
        branch,
        index,
    })
}

/// Derive a Cardano mainnet wallet (addr1… base address) from a seed phrase.
pub fn derive_cardano(
    seed_phrase: String,
    derivation_path: Option<String>,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    cardano_internal(
        true,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive a Cardano Preprod wallet (addr_test1… base address) from a seed phrase.
pub fn derive_cardano_preprod(
    seed_phrase: String,
    derivation_path: Option<String>,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    cardano_internal(
        false,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

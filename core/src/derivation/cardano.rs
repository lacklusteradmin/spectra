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

// A path's steps as indices, hardened or soft, read by BIP-32's one reader.
fn parse_bip32_path_segments(path: &str) -> Result<Vec<u32>, DerivationError> {
    Ok(crate::derivation::bip32::indices(
        &crate::derivation::bip32::parse_path(path)?,
    ))
}

// ── BIP-32-Ed25519 (Khovratovich-Law) ────────────────────────────────────

const HARDENED: u32 = 0x8000_0000;

// Walk `segments` down from a 96-byte root xprv; return the extended key and its public key.
fn derive_icarus_child_key(
    root: &[u8; 96],
    segments: &[u32],
) -> Result<([u8; 64], [u8; 32]), DerivationError> {
    let mut xprv: Zeroizing<[u8; 96]> = Zeroizing::new(*root);
    for index in segments {
        xprv = crate::derivation::ed25519_extended::derive_child(
            &xprv,
            *index,
            crate::derivation::ed25519_extended::KHOVRATOVICH_BITS,
        )?;
    }
    let mut private_key = [0u8; 64];
    private_key.copy_from_slice(&xprv[0..64]);
    let public_key = crate::derivation::ed25519_extended::public_key(&private_key)?;
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
    crate::secrets::kdf::pbkdf2_sha512(passphrase.as_bytes(), &entropy, iterations, &mut *xprv);
    xprv[0] &= 0b1111_1000;
    xprv[31] &= 0b0001_1111;
    xprv[31] |= 0b0100_0000;
    Ok(xprv)
}

/// The cosigner keys a phrase holds for shared (multisig) wallets, per
/// CIP-1854: `m/1854'/1815'/0'/0/index` for `index` from 0 up to, not
/// including, `count`, each with its key hash.
pub(crate) fn cosigner_keys(
    seed_phrase: &str,
    passphrase: &str,
    count: u32,
) -> Result<Vec<(Zeroizing<[u8; 64]>, [u8; 32], [u8; 28])>, DerivationError> {
    let root = derive_cardano_icarus_xprv_root(seed_phrase, passphrase, None, 0)?;
    (0..count)
        .map(|index| {
            let (private, public) = derive_icarus_child_key(
                &root,
                &[1854 | HARDENED, 1815 | HARDENED, HARDENED, 0, index],
            )?;
            Ok((Zeroizing::new(private), public, key_hash(&public)))
        })
        .collect()
}

/// A CIP-19 Shelley enterprise address of a script credential (type 7):
/// the script's hash alone.
pub(crate) fn script_enterprise_address(
    script_hash: &[u8; 28],
    is_mainnet: bool,
) -> Result<String, DerivationError> {
    let mut payload = vec![7 << 4 | u8::from(is_mainnet)];
    payload.extend_from_slice(script_hash);
    let hrp = bech32::Hrp::parse(if is_mainnet { "addr" } else { "addr_test" })
        .map_err(DerivationError::invalid)?;
    bech32::encode::<bech32::Bech32>(hrp, &payload).map_err(DerivationError::invalid)
}

/// Blake2b-224 of a public key: the key hash a vkey witness proves.
pub(crate) fn verification_key_hash(public_key: &[u8; 32]) -> [u8; 28] {
    key_hash(public_key)
}

// Blake2b-224 of a public key: the key hash a Shelley address carries.
fn key_hash(public_key: &[u8; 32]) -> [u8; 28] {
    crate::derivation::primitives::blake2b::<28>(&[public_key])
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

/// The CIP-19 reward address of a base address's stake credential, where
/// its rewards and delegation live: type 14 (`stake1…`, `stake_test1…`) for
/// a key, as every phrase wallet's is, or 15 for a script. `None` for an
/// address with no stake credential of its own: enterprise, as a raw key
/// derives, pointer, reward and Byron.
pub(crate) fn cardano_stake_address(address: &str) -> Result<Option<String>, DerivationError> {
    if !address.starts_with("addr1") && !address.starts_with("addr_test1") {
        return Ok(None);
    }
    let data = decode_cardano_addr_bytes(address)?;
    let header = *data
        .first()
        .ok_or_else(|| DerivationError::invalid("Empty Cardano address"))?;
    let (kind, network) = (header >> 4, header & 0x0f);
    if kind > 3 {
        return Ok(None);
    }
    if data.len() != 57 || (network == 1) != address.starts_with("addr1") {
        return Err(DerivationError::invalid("Invalid Cardano base address"));
    }
    // Types 2 and 3 carry a script's hash as their stake credential.
    let mut reward = Vec::with_capacity(29);
    reward.push(if kind >= 2 { 0xf0 } else { 0xe0 } | network);
    reward.extend_from_slice(&data[29..]);
    let hrp = bech32::Hrp::parse(if network == 1 { "stake" } else { "stake_test" })
        .map_err(DerivationError::invalid)?;
    Ok(Some(
        bech32::encode::<bech32::Bech32>(hrp, &reward).map_err(DerivationError::invalid)?,
    ))
}

/// Derive a phrase's Cardano base address on `chain`'s network, payment
/// public key and payment private key via CIP-3 Icarus + BIP-32-Ed25519.
pub(crate) fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let (private_key, public_key, stake_public) =
        derive_cardano_base_keys(seed_phrase, passphrase.unwrap_or(""), derivation_path)?;
    Ok(DerivationResult {
        address: wants
            .address
            .then(|| cardano_base_address(&public_key, &stake_public, !chain.is_testnet()))
            .transpose()?,
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&private_key),
    })
}

use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::Chain;

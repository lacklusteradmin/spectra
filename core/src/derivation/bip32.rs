//! BIP-32: the one reader of a derivation path, and the secp256k1 extended
//! keys every secp256k1 network derives from the same `"Bitcoin seed"`
//! master. The keys are rust-bitcoin's, which the multisig descriptors and
//! PSBTs use too, so one implementation answers for all of them.
//!
//! A path is written `m/44'/0'/0'/0/0`: an optional `m` (or `M`), then steps
//! of a number below 2^31, hardened by a trailing `'`, `h` or `H`. SLIP-10
//! and Cardano's BIP32-Ed25519 walk paths in the same notation and read them
//! here as well.

use bitcoin::bip32::{ChainCode, ChildNumber, DerivationPath, Fingerprint, Xpriv, Xpub};
use secp256k1::SecretKey;

use crate::derivation::error::DerivationError;

/// The bit that marks a hardened step.
pub(crate) const HARDENED_OFFSET: u32 = 1 << 31;

/// The version bytes of a Bitcoin mainnet `xpub` and a testnet `tpub`, the
/// ones rust-bitcoin reads.
pub(crate) const XPUB_VERSION_MAINNET: [u8; 4] = [0x04, 0x88, 0xB2, 0x1E];
pub(crate) const XPUB_VERSION_TESTNET: [u8; 4] = [0x04, 0x35, 0x87, 0xCF];

/// One step: `44'`, `44h`, `44H` or `44`.
pub(crate) fn parse_step(step: &str) -> Result<ChildNumber, DerivationError> {
    let step = step.trim();
    let (digits, hardened) = match step.strip_suffix(['\'', 'h', 'H']) {
        Some(digits) => (digits, true),
        None => (step, false),
    };
    let index: u32 = digits
        .parse()
        .map_err(|_| DerivationError::refused("Invalid derivation path segment: %@", [step]))?;
    if hardened {
        ChildNumber::from_hardened_idx(index)
    } else {
        ChildNumber::from_normal_idx(index)
    }
    .map_err(|_| DerivationError::refused("Derivation path segment out of range: %@", [step]))
}

/// A path: `m`, `m/44'/0'/0'/0/0`, or the same steps without the `m/`.
pub(crate) fn parse_path(path: &str) -> Result<DerivationPath, DerivationError> {
    let path = path.trim();
    let steps = if path.eq_ignore_ascii_case("m") {
        ""
    } else {
        path.strip_prefix("m/")
            .or_else(|| path.strip_prefix("M/"))
            .unwrap_or(path)
    };
    if steps.is_empty() {
        return Ok(DerivationPath::master());
    }
    steps
        .split('/')
        .map(parse_step)
        .collect::<Result<Vec<_>, _>>()
        .map(DerivationPath::from)
}

/// A path's steps as BIP-32 indices, a hardened one with
/// [`HARDENED_OFFSET`] set.
pub(crate) fn indices(path: &DerivationPath) -> Vec<u32> {
    path.into_iter().map(|step| u32::from(*step)).collect()
}

/// A step's number, hardened or not.
pub(crate) fn step_number(step: ChildNumber) -> u32 {
    match step {
        ChildNumber::Normal { index } | ChildNumber::Hardened { index } => index,
    }
}

/// The master key HMAC-SHA512 makes of `seed` under `hmac_key`: BIP-32's
/// `"Bitcoin seed"`, unless a wallet's own derivation names another.
pub(crate) fn master(seed: &[u8], hmac_key: &[u8]) -> Result<Xpriv, DerivationError> {
    let tag = crate::derivation::primitives::hmac_sha512(hmac_key, &[seed])?;
    let private_key = SecretKey::from_slice(&tag[..32]).map_err(|error| {
        DerivationError::Internal(format!("Derived BIP-32 master key is invalid: {error}"))
    })?;
    let mut chain_code = [0u8; 32];
    chain_code.copy_from_slice(&tag[32..]);
    Ok(Xpriv {
        network: bitcoin::NetworkKind::Main,
        depth: 0,
        parent_fingerprint: Fingerprint::default(),
        child_number: ChildNumber::Normal { index: 0 },
        private_key,
        chain_code: ChainCode::from(chain_code),
    })
}

/// The extended private key at `path` under `seed`'s `"Bitcoin seed"`
/// master.
pub(crate) fn derive(seed: &[u8], path: &DerivationPath) -> Result<Xpriv, DerivationError> {
    derive_private(&master(seed, b"Bitcoin seed")?, path)
}

/// The extended private key `path` below `key`.
pub(crate) fn derive_private(key: &Xpriv, path: &DerivationPath) -> Result<Xpriv, DerivationError> {
    key.derive_priv(secp256k1::SECP256K1, path)
        .map_err(|error| DerivationError::Internal(format!("BIP-32 derivation failed: {error}")))
}

/// The extended public key `steps` below `key`. A public key walks
/// unhardened steps only; a hardened one is refused.
pub(crate) fn derive_public(key: &Xpub, steps: &[u32]) -> Result<Xpub, DerivationError> {
    let steps = steps
        .iter()
        .map(|&step| ChildNumber::from_normal_idx(step))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            DerivationError::Invalid("cannot derive a hardened child from an xpub".into())
        })?;
    key.derive_pub(secp256k1::SECP256K1, &steps)
        .map_err(|error| DerivationError::Internal(format!("BIP-32 derivation failed: {error}")))
}

/// The secp256k1 key a recovery phrase holds at `path`.
pub(crate) fn phrase_key(
    seed_phrase: &str,
    passphrase: Option<&str>,
    path: &str,
) -> Result<SecretKey, DerivationError> {
    let seed = crate::derivation::primitives::derive_bip39_seed(
        seed_phrase,
        passphrase.unwrap_or(""),
        0,
        None,
        None,
    )?;
    Ok(derive(seed.as_ref(), &parse_path(path)?)?.private_key)
}

/// `key` as its network writes an account key: its 78 bytes under `version`.
pub(crate) fn encode_xpub(key: &Xpub, version: [u8; 4]) -> String {
    let mut bytes = key.encode();
    bytes[..4].copy_from_slice(&version);
    bitcoin::base58::encode_check(&bytes)
}

/// An account key in whichever version it is written, with that version:
/// the caller knows which versions its network accepts.
pub(crate) fn decode_xpub(text: &str) -> Result<(Xpub, [u8; 4]), DerivationError> {
    let mut bytes = bitcoin::base58::decode_check(text.trim())
        .map_err(|error| DerivationError::Invalid(format!("xpub: {error}").into()))?;
    if bytes.len() != 78 {
        return Err(DerivationError::Invalid(
            format!("xpub payload must be 78 bytes, got {}", bytes.len()).into(),
        ));
    }
    let mut version = [0u8; 4];
    version.copy_from_slice(&bytes[..4]);
    bytes[..4].copy_from_slice(&XPUB_VERSION_MAINNET);
    let key = Xpub::decode(&bytes)
        .map_err(|error| DerivationError::Invalid(format!("xpub: {error}").into()))?;
    Ok((key, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling of a hardened step is one step, `m` is optional, and a
    /// number at or past 2^31 is refused rather than wrapped.
    #[test]
    fn a_path_reads_one_way_whatever_its_spelling() {
        let expected = parse_path("m/44'/0'/0'/0/7").unwrap();
        for spelling in [
            "m/44h/0H/0'/0/7",
            "M/44'/0'/0'/0/7",
            "44'/0'/0'/0/7",
            " m/44'/0'/0'/0/7 ",
        ] {
            assert_eq!(parse_path(spelling).unwrap(), expected, "{spelling}");
        }
        assert_eq!(
            indices(&expected),
            [44 | HARDENED_OFFSET, HARDENED_OFFSET, HARDENED_OFFSET, 0, 7]
        );
        for empty in ["", "m", "M"] {
            assert!(parse_path(empty).unwrap().is_master(), "{empty}");
        }
        for refused in ["m/2147483648", "m/x", "m//0", "m/44''", "mm/0"] {
            assert!(parse_path(refused).is_err(), "{refused}");
        }
    }

    /// An account key keeps the version it was written in, which rust-bitcoin
    /// alone would refuse for anything but `xpub` and `tpub`.
    #[test]
    fn an_account_key_round_trips_in_its_own_version() {
        let key = Xpub::from_priv(
            secp256k1::SECP256K1,
            &derive(&[7; 64], &parse_path("m/84'/0'/0'").unwrap()).unwrap(),
        );
        let zpub = [0x04, 0xB2, 0x47, 0x46];
        let text = encode_xpub(&key, zpub);
        assert!(text.starts_with("zpub"), "{text}");
        assert_eq!(decode_xpub(&text).unwrap(), (key, zpub));
    }
}

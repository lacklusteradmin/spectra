//! ICP secp256k1 self-authenticating principal and default ledger account.
//! A phrase's key is the BIP-32 secp256k1 key at `m/44'/223'/account'/0/0`,
//! the one quill, dfx and DFINITY's agent libraries derive.
//! Account identifier: CRC32(SHA224(domain || principal || subaccount)) || hash.

use crate::derivation::error::DerivationError;
use crate::derivation::primitives::{ExtendedPrivateKey, derive_bip39_seed, parse_bip32_path};
use secp256k1::PublicKey;
use sha2::{Digest, Sha224};

/// SubjectPublicKeyInfo up to the point: `id-ecPublicKey` on the
/// `secp256k1` curve, then a 65-byte bit string.
const SECP256K1_SPKI_PREFIX: [u8; 23] = [
    0x30, 0x56, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b,
    0x81, 0x04, 0x00, 0x0a, 0x03, 0x42, 0x00,
];

/// The DER key an ingress message names its sender by: the uncompressed
/// point, as the agent libraries encode it. The principal hashes these
/// bytes, so the compressed point would name another identity.
pub(crate) fn public_key_der(public_key: &PublicKey) -> Vec<u8> {
    let mut der = SECP256K1_SPKI_PREFIX.to_vec();
    der.extend(public_key.serialize_uncompressed());
    der
}

pub(crate) fn principal(public_key: &PublicKey) -> Vec<u8> {
    let mut result = Sha224::digest(public_key_der(public_key)).to_vec();
    result.push(2); // Self-authenticating principal class.
    result
}

pub(crate) fn account_from_principal(principal: &[u8]) -> [u8; 32] {
    account_with_subaccount(principal, &[0; 32])
}

pub(crate) fn account_with_subaccount(principal: &[u8], subaccount: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha224::new();
    hash.update(b"\x0aaccount-id");
    hash.update(principal);
    hash.update(subaccount);
    let hash = hash.finalize();
    let mut result = [0; 32];
    result[..4].copy_from_slice(
        &crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC)
            .checksum(&hash)
            .to_be_bytes(),
    );
    result[4..].copy_from_slice(&hash);
    result
}

pub(crate) fn validate_account(address: &str) -> Result<[u8; 32], DerivationError> {
    let bytes: [u8; 32] = hex::decode(address)
        .map_err(DerivationError::invalid)?
        .try_into()
        .map_err(|_| DerivationError::Invalid("ICP account must be 32 bytes".into()))?;
    let checksum = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC)
        .checksum(&bytes[4..])
        .to_be_bytes();
    if bytes[..4] != checksum {
        return Err(DerivationError::Invalid(
            "Invalid ICP account checksum".into(),
        ));
    }
    Ok(bytes)
}

pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let master = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())?;
    let xpriv = master.derive_path(secp256k1::SECP256K1, &parse_bip32_path(derivation_path)?)?;
    let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &xpriv.private_key);

    let address =
        want_address.then(|| hex::encode(account_from_principal(&principal(&public_key))));

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(xpriv.private_key.secret_bytes())),
    ))
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::derivation::types::{DerivationResult, parse_path_metadata};
use crate::error::SpectraBridgeError;

/// Derive Internet Computer keys from a BIP-39 seed phrase.
pub fn derive_icp(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        &seed_phrase,
        &derivation_path,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ledger_checksum_and_principal_domain() {
        // Official ledger account-identifier checksum vector (zero hash).
        let valid = "807077e900000000000000000000000000000000000000000000000000000000";
        assert!(validate_account(valid).is_ok());
        assert!(validate_account(&"00".repeat(32)).is_err());
        for i in 0..32 {
            let mut b = validate_account(valid).unwrap();
            b[i] ^= 1;
            assert!(validate_account(&hex::encode(b)).is_err());
        }
        let key = crate::send::keys::Secp256k1Key::from_hex(&"01".repeat(32)).unwrap();
        let id = account_from_principal(&principal(&key.public_key()));
        assert_eq!(validate_account(&hex::encode(id)).unwrap(), id);
        assert_eq!(principal(&key.public_key()).len(), 29);
        assert_eq!(public_key_der(&key.public_key()).len(), 88);
    }
}

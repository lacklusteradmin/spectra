//! ICP Ed25519 self-authenticating principal and default ledger account.
//! Account identifier: CRC32(SHA224(domain || principal || subaccount)) || hash.
use crate::derivation::primitives::derive_bip39_seed;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha224};

pub(crate) fn public_key_der(public_key: &[u8; 32]) -> Vec<u8> {
    let mut der = vec![
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    der.extend(public_key);
    der
}

pub(crate) fn principal(public_key: &[u8; 32]) -> Vec<u8> {
    let mut result = Sha224::digest(public_key_der(public_key)).to_vec();
    result.push(2); // Self-authenticating principal class.
    result
}

pub(crate) fn account_from_principal(principal: &[u8]) -> [u8; 32] {
    let mut hash = Sha224::new();
    hash.update(b"\x0aaccount-id");
    hash.update(principal);
    hash.update([0; 32]);
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

pub(crate) fn validate_account(address: &str) -> Result<[u8; 32], String> {
    let bytes: [u8; 32] = hex::decode(address)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "ICP account must be 32 bytes")?;
    let checksum = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC)
        .checksum(&bytes[4..])
        .to_be_bytes();
    if bytes[..4] != checksum {
        return Err("Invalid ICP account checksum".into());
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
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, String> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, None)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    let address =
        want_address.then(|| hex::encode(account_from_principal(&principal(&public_key))));

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(*private_key)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::primitives::derive_slip10_ed25519_key;
use crate::derivation::types::{parse_path_metadata, DerivationResult};
use crate::SpectraBridgeError;

/// UniFFI export: derive Internet Computer keys from a BIP-39 seed phrase.
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
        let key = SigningKey::from_bytes(&[1; 32]);
        let id = account_from_principal(&principal(&key.verifying_key().to_bytes()));
        assert_eq!(validate_account(&hex::encode(id)).unwrap(), id);
        assert_eq!(principal(&key.verifying_key().to_bytes()).len(), 29);
    }
}

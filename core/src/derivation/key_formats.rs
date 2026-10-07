//! Each chain's own private-key encodings, read into the hex key derivation
//! takes.
//!
//! Wallets export keys in their chain's format, not as bare hex: WIF on the
//! Bitcoin family, a base58 or JSON 64-byte keypair on Solana, an `S…` secret
//! seed on Stellar, `suiprivkey1…` on Sui, AIP-80 on Aptos and `ed25519:…` on
//! NEAR. Every check here refuses before anything is sealed: a WIF for another
//! network or for an uncompressed key, and a keypair whose public half is not
//! its secret's. Vectors from each chain's own SDK are in
//! `core/tests/fixtures/private-key-formats.json`.

use ed25519_dalek::SigningKey;
use zeroize::Zeroizing;

use crate::derivation::error::DerivationError;
use crate::derivation::setup::WalletSecretFormat;
use crate::registry::Chain;

/// The key in `input`, in whichever of `chain`'s private-key formats it is
/// written, as lowercase hex: 32 bytes, or Cardano's 64.
pub(crate) fn parse_private_key(
    chain: Chain,
    input: &str,
) -> Result<Zeroizing<String>, DerivationError> {
    let input = input.trim();
    let formats = chain.private_key_formats();
    if formats.is_empty() {
        return Err(DerivationError::refused(
            "%@ cannot derive an address from a private key.",
            [chain.chain_display_name()],
        ));
    }
    for format in &formats {
        if let Some(read) = read(*format, chain, input) {
            return read;
        }
    }
    Err(DerivationError::refused(
        "This is not a private key %@ takes.",
        [chain.chain_display_name()],
    ))
}

/// Whether `raw_value` is a private key `chain` takes: the yes-or-no a key
/// editor asks as it is typed, so no copy of the key crosses back.
#[uniffi::export]
pub fn is_valid_private_key(chain: Chain, raw_value: String) -> bool {
    parse_private_key(chain, &raw_value).is_ok()
}

/// `None` when `input` is not written in `format` at all; `Some(Err)` when it
/// is, but is not a key this chain can use.
fn read(
    format: WalletSecretFormat,
    chain: Chain,
    input: &str,
) -> Option<Result<Zeroizing<String>, DerivationError>> {
    match format {
        WalletSecretFormat::HexSecret32 => hex_key(input, 32).map(Ok),
        WalletSecretFormat::CardanoExtendedKey => hex_key(input, 64).map(Ok),
        WalletSecretFormat::Wif => wif(chain, input),
        WalletSecretFormat::SolanaKeypair => solana_keypair(input),
        WalletSecretFormat::StellarSecretSeed => stellar_secret_seed(input),
        WalletSecretFormat::SuiPrivateKey => sui_private_key(input),
        WalletSecretFormat::AptosPrivateKey => {
            let rest = input.strip_prefix("ed25519-priv-")?;
            Some(hex_key(rest, 32).ok_or_else(|| {
                DerivationError::invalid("An AIP-80 Aptos key holds 32 bytes of hex.")
            }))
        }
        WalletSecretFormat::NearSecretKey => near_secret_key(input),
        _ => None,
    }
}

/// `bytes` bytes of hex, with or without `0x`, lowercased.
fn hex_key(input: &str, bytes: usize) -> Option<Zeroizing<String>> {
    let digits = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        .unwrap_or(input);
    (digits.len() == bytes * 2 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| Zeroizing::new(digits.to_ascii_lowercase()))
}

/// Base58Check: version, 32-byte key, and `0x01` for a compressed key.
fn wif(chain: Chain, input: &str) -> Option<Result<Zeroizing<String>, DerivationError>> {
    let expected = chain.wif_version()?;
    let payload = Zeroizing::new(bs58::decode(input).with_check(None).into_vec().ok()?);
    let compressed = match payload.len() {
        34 if payload[33] == 0x01 => true,
        33 => false,
        _ => return None,
    };
    Some(if payload[0] != expected {
        Err(DerivationError::refused(
            "This key is not for %@.",
            [chain.chain_display_name()],
        ))
    } else if !compressed {
        // An uncompressed key owns a different address; compressing it would
        // import a wallet the key's own address does not belong to.
        Err(DerivationError::invalid(
            "This WIF is for an uncompressed key, which Spectra does not sign with.",
        ))
    } else {
        Ok(Zeroizing::new(hex::encode(&payload[1..33])))
    })
}

/// The secret half of a `secret ‖ public` ed25519 keypair, refused when the
/// public half is not the secret's.
fn ed25519_pair(pair: &[u8]) -> Result<Zeroizing<String>, DerivationError> {
    let secret: [u8; 32] = pair[..32]
        .try_into()
        .map_err(|_| DerivationError::invalid("A keypair holds 64 bytes."))?;
    let secret = Zeroizing::new(secret);
    if SigningKey::from_bytes(&secret).verifying_key().as_bytes() != &pair[32..] {
        return Err(DerivationError::invalid(
            "This keypair's public key does not belong to its secret key.",
        ));
    }
    Ok(Zeroizing::new(hex::encode(*secret)))
}

/// A Solana keypair: 64 bytes in base58, as wallets export it, or as the JSON
/// byte array the Solana CLI keeps.
fn solana_keypair(input: &str) -> Option<Result<Zeroizing<String>, DerivationError>> {
    let bytes = Zeroizing::new(if input.starts_with('[') {
        serde_json::from_str::<Vec<u8>>(input).ok()?
    } else {
        bs58::decode(input).into_vec().ok()?
    });
    (bytes.len() == 64).then(|| ed25519_pair(&bytes))
}

/// A Stellar secret seed: StrKey version `18 << 3` (`S…`), 32 bytes, and a
/// little-endian CRC-16/XMODEM.
fn stellar_secret_seed(input: &str) -> Option<Result<Zeroizing<String>, DerivationError>> {
    if !input.starts_with('S') || input.len() != 56 {
        return None;
    }
    let decoded = Zeroizing::new(data_encoding::BASE32_NOPAD.decode(input.as_bytes()).ok()?);
    if decoded.len() != 35 || decoded[0] != 18 << 3 {
        return None;
    }
    const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
    Some(
        if CRC.checksum(&decoded[..33]).to_le_bytes() == decoded[33..] {
            Ok(Zeroizing::new(hex::encode(&decoded[1..33])))
        } else {
            Err(DerivationError::invalid(
                "This Stellar secret seed's checksum does not match.",
            ))
        },
    )
}

/// A Sui key: bech32 `suiprivkey`, a scheme flag and 32 bytes. Sui wallets
/// here are ed25519, flag 0.
fn sui_private_key(input: &str) -> Option<Result<Zeroizing<String>, DerivationError>> {
    if !input.starts_with("suiprivkey1") {
        return None;
    }
    let (hrp, data) = bech32::decode(input).ok()?;
    if hrp.as_str() != "suiprivkey" || data.len() != 33 {
        return None;
    }
    let data = Zeroizing::new(data);
    Some(if data[0] == 0x00 {
        Ok(Zeroizing::new(hex::encode(&data[1..])))
    } else {
        Err(DerivationError::invalid(
            "This Sui key is not an Ed25519 key, the scheme Spectra's Sui wallets use.",
        ))
    })
}

/// A NEAR key string: `ed25519:` and the base58 `secret ‖ public` keypair.
fn near_secret_key(input: &str) -> Option<Result<Zeroizing<String>, DerivationError>> {
    let encoded = input.strip_prefix("ed25519:")?;
    let bytes = Zeroizing::new(bs58::decode(encoded).into_vec().ok()?);
    (bytes.len() == 64).then(|| ed25519_pair(&bytes))
}

/// The encoding a key on `chain` is exported in: the chain's own, where its
/// wallets have one, or hex. `None` where a key alone yields no address.
pub(crate) fn export_format(chain: Chain) -> Option<WalletSecretFormat> {
    let formats = chain.private_key_formats();
    formats
        .iter()
        .copied()
        .find(|format| *format != WalletSecretFormat::HexSecret32)
        .or_else(|| formats.first().copied())
}

/// `key_hex` written in `format` for `chain`: the inverse of
/// [`parse_private_key`], so an exported key reads back as itself.
pub(crate) fn encode_private_key(
    chain: Chain,
    format: WalletSecretFormat,
    key_hex: &str,
) -> Result<Zeroizing<String>, DerivationError> {
    let key = Zeroizing::new(hex::decode(key_hex).map_err(DerivationError::invalid)?);
    let seed = || -> Result<Zeroizing<[u8; 32]>, DerivationError> {
        let seed: [u8; 32] = key
            .as_slice()
            .try_into()
            .map_err(|_| DerivationError::invalid("Private key must be exactly 32 bytes"))?;
        Ok(Zeroizing::new(seed))
    };
    // An ed25519 keypair as wallets write it: the secret, then its public key.
    let pair = || -> Result<Zeroizing<Vec<u8>>, DerivationError> {
        let seed = seed()?;
        let mut pair = Zeroizing::new(seed.to_vec());
        pair.extend_from_slice(SigningKey::from_bytes(&seed).verifying_key().as_bytes());
        Ok(pair)
    };
    Ok(Zeroizing::new(match format {
        WalletSecretFormat::HexSecret32 => hex::encode(*seed()?),
        WalletSecretFormat::CardanoExtendedKey if key.len() == 64 => hex::encode(&*key),
        WalletSecretFormat::Wif => {
            let version = chain.wif_version().ok_or_else(|| {
                DerivationError::refused(
                    "This is not a private key %@ takes.",
                    [chain.chain_display_name()],
                )
            })?;
            // Compressed: the key Spectra signs with owns the compressed
            // key's address.
            let mut payload = Zeroizing::new(vec![version]);
            payload.extend_from_slice(&*seed()?);
            payload.push(0x01);
            bs58::encode(&*payload).with_check().into_string()
        }
        WalletSecretFormat::SolanaKeypair => bs58::encode(&*pair()?).into_string(),
        WalletSecretFormat::NearSecretKey => {
            format!("ed25519:{}", bs58::encode(&*pair()?).into_string())
        }
        WalletSecretFormat::StellarSecretSeed => {
            const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
            let mut payload = Zeroizing::new(vec![18 << 3]);
            payload.extend_from_slice(&*seed()?);
            let checksum = CRC.checksum(&payload).to_le_bytes();
            payload.extend_from_slice(&checksum);
            data_encoding::BASE32_NOPAD.encode(&payload)
        }
        WalletSecretFormat::SuiPrivateKey => {
            let mut payload = Zeroizing::new(vec![0x00]);
            payload.extend_from_slice(&*seed()?);
            bech32::encode::<bech32::Bech32>(bech32::Hrp::parse_unchecked("suiprivkey"), &payload)
                .map_err(DerivationError::invalid)?
        }
        WalletSecretFormat::AptosPrivateKey => {
            format!("ed25519-priv-0x{}", hex::encode(*seed()?))
        }
        _ => {
            return Err(DerivationError::refused(
                "This is not a private key %@ takes.",
                [chain.chain_display_name()],
            ));
        }
    }))
}

#[cfg(test)]
#[path = "tests/key_formats.rs"]
mod tests;

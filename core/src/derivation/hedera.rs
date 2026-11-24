//! Hedera: account ids, and the keys an account holds.
//!
//! An account is `shard.realm.num` (`0.0.N` on mainnet), created by a
//! transaction; no address derives from a key. HIP-15 adds a five-letter
//! checksum over the id and the network's ledger id (`0.0.123-vfmkw`), which
//! an id typed with one must match; the id is stored without it.
//!
//! A key is Ed25519 or ECDSA secp256k1. Hedera's wallets and SDK write both
//! in DER: PKCS#8 for a private key, SubjectPublicKeyInfo for a public one,
//! which name the scheme a bare 32 bytes does not. A phrase derives along the
//! profile's path: SLIP-10 Ed25519 along `m/44'/3030'/0'/0'/{i}'`, or BIP-32
//! secp256k1 along `m/44'/3030'/0'/0/{i}` or Ethereum's `m/44'/60'/0'/0/{i}`,
//! as `@hashgraph/sdk`'s `Mnemonic` does. What a derivation calls the
//! address is the public key in DER hex: the key an account is looked up by.

use ed25519_dalek::{Signer as _, SigningKey};
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use zeroize::Zeroizing;

use crate::catalog::chains::DerivationProfile;
use crate::derivation::error::DerivationError;
use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::Chain;

/// The signature scheme of a Hedera key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HederaKeyScheme {
    Ed25519,
    EcdsaSecp256k1,
}

/// PKCS#8 of an Ed25519 private key, before its 32 bytes.
const ED25519_PRIVATE_DER: [u8; 16] = hex_literal("302e020100300506032b657004220420");
/// PKCS#8 of a secp256k1 private key, before its 32 bytes.
const ECDSA_PRIVATE_DER: [u8; 18] = hex_literal("3030020100300706052b8104000a04220420");
/// SubjectPublicKeyInfo of an Ed25519 public key, before its 32 bytes.
const ED25519_PUBLIC_DER: [u8; 12] = hex_literal("302a300506032b6570032100");
/// SubjectPublicKeyInfo of a compressed secp256k1 public key, before its
/// 33 bytes.
const ECDSA_PUBLIC_DER: [u8; 14] = hex_literal("302d300706052b8104000a032200");

/// The bytes a constant string of hex digits writes.
const fn hex_literal<const N: usize>(text: &str) -> [u8; N] {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("not a hex digit"),
        }
    }
    let text = text.as_bytes();
    assert!(text.len() == N * 2);
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = digit(text[2 * i]) << 4 | digit(text[2 * i + 1]);
        i += 1;
    }
    out
}

/// A Hedera account id: `shard.realm.num`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct HederaAccountId {
    pub shard: u64,
    pub realm: u64,
    pub num: u64,
}

impl std::fmt::Display for HederaAccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.shard, self.realm, self.num)
    }
}

impl HederaAccountId {
    /// `0.0.N` or, with its HIP-15 checksum, `0.0.N-abcde`; the checksum,
    /// where there is one, must be the one `chain`'s ledger gives the id.
    /// Each part is a decimal without leading zeros, as the network and
    /// its checksum write it.
    pub(crate) fn parse(chain: Chain, text: &str) -> Result<Self, DerivationError> {
        let invalid =
            || DerivationError::refused("Not a %@ account: %@", [chain.chain_display_name(), text]);
        let network = chain.hedera_network()?;
        let (id, checksum) = match text.split_once('-') {
            Some((id, checksum)) => (id, Some(checksum)),
            None => (text, None),
        };
        let parts: Vec<&str> = id.split('.').collect();
        let [shard, realm, num] = parts.as_slice() else {
            return Err(invalid());
        };
        let part = |part: &str| -> Option<u64> {
            let canonical = !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            canonical.then(|| part.parse().ok()).flatten()
        };
        let account = Self {
            shard: part(shard).ok_or_else(invalid)?,
            realm: part(realm).ok_or_else(invalid)?,
            num: part(num).ok_or_else(invalid)?,
        };
        // Mainnet's accounts are all in shard 0, realm 0; an id outside them
        // names no account a transfer here reaches.
        if account.shard != 0 || account.realm != 0 {
            return Err(invalid());
        }
        if let Some(checksum) = checksum
            && checksum != account.checksum(network.ledger_id)
        {
            return Err(DerivationError::refused(
                "The checksum of %@ is not this network's: check the account.",
                [text],
            ));
        }
        Ok(account)
    }

    /// The HIP-15 checksum of this id on the ledger `ledger_id`: five
    /// letters, which a mistyped id or another network's ledger changes.
    pub(crate) fn checksum(&self, ledger_id: &[u8]) -> String {
        const P3: u64 = 26 * 26 * 26;
        const P5: u64 = 26 * 26 * 26 * 26 * 26;
        const M: u64 = 1_000_003;
        const W: u64 = 31;
        let address = self.to_string();
        let digits: Vec<u64> = address
            .bytes()
            .map(|b| if b == b'.' { 10 } else { u64::from(b - b'0') })
            .collect();
        let (mut sd0, mut sd1, mut sd, mut sh) = (0u64, 0u64, 0u64, 0u64);
        for (i, digit) in digits.iter().enumerate() {
            sd = (W * sd + digit) % P3;
            if i % 2 == 0 {
                sd0 = (sd0 + digit) % 11;
            } else {
                sd1 = (sd1 + digit) % 11;
            }
        }
        for byte in ledger_id.iter().chain([0u8; 6].iter()) {
            sh = (W * sh + u64::from(*byte)) % P5;
        }
        let c = ((((digits.len() as u64 % 5) * 11 + sd0) * 11 + sd1) * P3 + sd + sh) % P5;
        let mut cp = (c * M) % P5;
        let mut letters = [b'a'; 5];
        for letter in letters.iter_mut().rev() {
            *letter = b'a' + (cp % 26) as u8;
            cp /= 26;
        }
        String::from_utf8(letters.to_vec()).expect("ASCII letters")
    }

    /// Whether this is one of the network's own accounts (fees, staking
    /// rewards, nodes) rather than a person's.
    pub(crate) fn is_system(&self) -> bool {
        self.num <= crate::registry::HEDERA_LAST_SYSTEM_ACCOUNT
    }
}

/// A Hedera private key and the scheme it signs in.
pub(crate) struct HederaPrivateKey {
    pub scheme: HederaKeyScheme,
    secret: Zeroizing<[u8; 32]>,
}

/// A Hedera public key: 32 Ed25519 bytes, or a 33-byte compressed
/// secp256k1 point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HederaPublicKey {
    pub scheme: HederaKeyScheme,
    pub bytes: Vec<u8>,
}

impl HederaPrivateKey {
    pub(crate) fn new(
        scheme: HederaKeyScheme,
        secret: Zeroizing<[u8; 32]>,
    ) -> Result<Self, DerivationError> {
        if scheme == HederaKeyScheme::EcdsaSecp256k1 {
            SecretKey::from_slice(secret.as_slice())
                .map_err(|_| DerivationError::invalid("Not a secp256k1 private key."))?;
        }
        Ok(Self { scheme, secret })
    }

    /// A key in the DER hex Hedera's wallets export: PKCS#8 naming Ed25519
    /// or secp256k1. `None` when `text` is not one; a bare 32 bytes names
    /// no scheme.
    pub(crate) fn from_der_hex(text: &str) -> Option<Result<Self, DerivationError>> {
        let digits = text
            .strip_prefix("0x")
            .or_else(|| text.strip_prefix("0X"))
            .unwrap_or(text);
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = Zeroizing::new(hex::decode(digits).ok()?);
        let (scheme, rest) = if let Some(rest) = bytes.strip_prefix(&ED25519_PRIVATE_DER) {
            (HederaKeyScheme::Ed25519, rest)
        } else {
            (
                HederaKeyScheme::EcdsaSecp256k1,
                bytes.strip_prefix(&ECDSA_PRIVATE_DER)?,
            )
        };
        let secret: [u8; 32] = match rest.try_into() {
            Ok(secret) => secret,
            Err(_) => {
                return Some(Err(DerivationError::invalid(
                    "A Hedera DER private key holds 32 bytes after its header.",
                )));
            }
        };
        Some(Self::new(scheme, Zeroizing::new(secret)))
    }

    /// This key in DER hex, as the SDK's `toStringDer` writes it.
    pub(crate) fn der_hex(&self) -> Zeroizing<String> {
        let prefix: &[u8] = match self.scheme {
            HederaKeyScheme::Ed25519 => &ED25519_PRIVATE_DER,
            HederaKeyScheme::EcdsaSecp256k1 => &ECDSA_PRIVATE_DER,
        };
        let mut bytes = Zeroizing::new(prefix.to_vec());
        bytes.extend_from_slice(self.secret.as_slice());
        Zeroizing::new(hex::encode(bytes.as_slice()))
    }

    pub(crate) fn public_key(&self) -> HederaPublicKey {
        let bytes = match self.scheme {
            HederaKeyScheme::Ed25519 => SigningKey::from_bytes(&self.secret)
                .verifying_key()
                .to_bytes()
                .to_vec(),
            HederaKeyScheme::EcdsaSecp256k1 => {
                let secret = SecretKey::from_slice(self.secret.as_slice())
                    .expect("checked when the key was made");
                PublicKey::from_secret_key(&Secp256k1::signing_only(), &secret)
                    .serialize()
                    .to_vec()
            }
        };
        HederaPublicKey {
            scheme: self.scheme,
            bytes,
        }
    }

    /// The signature Hedera checks over `message`: Ed25519 over the bytes,
    /// or secp256k1 over their Keccak-256 as `r ‖ s` with a low `s`.
    pub(crate) fn sign(&self, message: &[u8]) -> Vec<u8> {
        match self.scheme {
            HederaKeyScheme::Ed25519 => SigningKey::from_bytes(&self.secret)
                .sign(message)
                .to_bytes()
                .to_vec(),
            HederaKeyScheme::EcdsaSecp256k1 => {
                use sha3::Digest as _;
                let digest: [u8; 32] = sha3::Keccak256::digest(message).into();
                let secret = SecretKey::from_slice(self.secret.as_slice())
                    .expect("checked when the key was made");
                Secp256k1::signing_only()
                    .sign_ecdsa(&Message::from_digest(digest), &secret)
                    .serialize_compact()
                    .to_vec()
            }
        }
    }
}

impl HederaPublicKey {
    /// This key in DER hex, as the SDK's `toStringDer` writes it.
    pub(crate) fn der_hex(&self) -> String {
        let prefix: &[u8] = match self.scheme {
            HederaKeyScheme::Ed25519 => &ED25519_PUBLIC_DER,
            HederaKeyScheme::EcdsaSecp256k1 => &ECDSA_PUBLIC_DER,
        };
        hex::encode([prefix, self.bytes.as_slice()].concat())
    }

    /// A key in the DER hex [`HederaPublicKey::der_hex`] writes.
    pub(crate) fn from_der_hex(text: &str) -> Option<Self> {
        let bytes = hex::decode(text).ok()?;
        let (scheme, rest) = if let Some(rest) = bytes.strip_prefix(&ED25519_PUBLIC_DER) {
            (HederaKeyScheme::Ed25519, rest)
        } else {
            (
                HederaKeyScheme::EcdsaSecp256k1,
                bytes.strip_prefix(&ECDSA_PUBLIC_DER)?,
            )
        };
        Self::from_raw(scheme, rest)
    }

    /// `bytes` as a key of `scheme`, refused unless it is one: a point on
    /// the curve of the scheme's length.
    pub(crate) fn from_raw(scheme: HederaKeyScheme, bytes: &[u8]) -> Option<Self> {
        let valid = match scheme {
            HederaKeyScheme::Ed25519 => <[u8; 32]>::try_from(bytes)
                .ok()
                .and_then(|key| ed25519_dalek::VerifyingKey::from_bytes(&key).ok())
                .is_some(),
            HederaKeyScheme::EcdsaSecp256k1 => {
                bytes.len() == 33 && PublicKey::from_slice(bytes).is_ok()
            }
        };
        valid.then(|| Self {
            scheme,
            bytes: bytes.to_vec(),
        })
    }
}

/// The scheme `profile`'s keys are on Hedera.
pub(crate) fn profile_scheme(profile: DerivationProfile) -> Option<HederaKeyScheme> {
    match profile {
        DerivationProfile::Standard => Some(HederaKeyScheme::Ed25519),
        DerivationProfile::Ecdsa | DerivationProfile::Evm => Some(HederaKeyScheme::EcdsaSecp256k1),
        _ => None,
    }
}

/// The key a phrase holds at `path`, which must be one of `chain`'s
/// profiles: the profile names the scheme, which a custom path would leave
/// to a guess.
pub(crate) fn phrase_key(
    chain: Chain,
    seed_phrase: &str,
    path: &str,
    passphrase: Option<&str>,
) -> Result<HederaPrivateKey, DerivationError> {
    let scheme = crate::derivation::path::derivation_profile_of_path(chain, path)
        .and_then(|choice| profile_scheme(choice.profile))
        .ok_or_else(|| {
            DerivationError::refused(
                "%@ derives along its wallets' paths only: choose a derivation profile.",
                [chain.chain_display_name()],
            )
        })?;
    let secret = match scheme {
        HederaKeyScheme::Ed25519 => {
            let seed = crate::derivation::primitives::derive_bip39_seed(
                seed_phrase,
                passphrase.unwrap_or(""),
                0,
                None,
                None,
            )?;
            crate::derivation::primitives::derive_slip10_ed25519_key(seed.as_ref(), path, None)?
        }
        HederaKeyScheme::EcdsaSecp256k1 => Zeroizing::new(
            crate::derivation::bip32::phrase_key(seed_phrase, passphrase, path)?.secret_bytes(),
        ),
    };
    HederaPrivateKey::new(scheme, secret)
}

/// What a derivation answers for a Hedera key: its public key in DER hex as
/// the address an account is found by and as the public key, and the
/// private key in DER hex, which names its scheme.
fn result(key: &HederaPrivateKey, wants: Wants) -> DerivationResult {
    let public = key.public_key().der_hex();
    let private = key.der_hex();
    DerivationResult {
        address: wants.address.then(|| public.clone()),
        public_key_hex: wants.public_key.then_some(public),
        private_key_hex: wants.private_key.then_some(private),
    }
}

/// Derive a Hedera key from a seed phrase along one of the chain's profiles.
pub(crate) fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let key = phrase_key(chain, seed_phrase, derivation_path, passphrase)?;
    Ok(result(&key, wants))
}

/// A stored Hedera key, in the DER hex an import seals: refused when it is
/// bare hex, which names no scheme.
pub(crate) fn stored_key(private_key: &str) -> Result<HederaPrivateKey, DerivationError> {
    HederaPrivateKey::from_der_hex(private_key.trim()).unwrap_or_else(|| {
        Err(DerivationError::invalid(
            "A Hedera key is Ed25519 or ECDSA; write it in DER, which says which.",
        ))
    })
}

/// Derive what a raw key holds: its public key, as a phrase's derivation
/// answers it.
pub(crate) fn derive_from_private_key(
    private_key: &str,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    Ok(result(&stored_key(private_key)?, wants))
}

/// The keys a raw key typed at import may be: the one its DER names, or for
/// 32 bytes of bare hex, its Ed25519 and its secp256k1 reading, which the
/// account the network holds tells apart.
pub(crate) fn import_key_candidates(
    private_key: &str,
) -> Result<Vec<HederaPrivateKey>, DerivationError> {
    if let Some(key) = HederaPrivateKey::from_der_hex(private_key.trim()) {
        return Ok(vec![key?]);
    }
    let digits = private_key
        .trim()
        .strip_prefix("0x")
        .unwrap_or(private_key.trim());
    let bytes = Zeroizing::new(hex::decode(digits).map_err(DerivationError::invalid)?);
    let secret: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| DerivationError::invalid("A Hedera private key holds 32 bytes."))?;
    let mut keys = vec![HederaPrivateKey::new(
        HederaKeyScheme::Ed25519,
        Zeroizing::new(secret),
    )?];
    // Not every 32 bytes is a secp256k1 scalar; such a key can only be
    // Ed25519.
    if let Ok(key) = HederaPrivateKey::new(HederaKeyScheme::EcdsaSecp256k1, Zeroizing::new(secret))
    {
        keys.push(key);
    }
    Ok(keys)
}

#[cfg(test)]
#[path = "tests/hedera.rs"]
mod tests;

//! A Sui multisig account: a `MultiSigPublicKey` of up to ten weighted keys
//! (Ed25519, secp256k1 or secp256r1) and a threshold their weights meet,
//! whose address is derived from them, so it needs no registration.
//!
//! The policy is written as Sui's SDK takes it to build one:
//! `{"threshold": 2, "publicKeys": [{"publicKey": "<base64 flag ‖ key>",
//! "weight": 1}, …]}`, each key in Sui's public-key form. Order matters: it
//! is the order of the account's members, which the address and every
//! combined signature's bitmap follow.

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::derivation::error::DerivationError;

/// The members one multisig key holds at most.
pub(crate) const MAX_MEMBERS: usize = 10;
/// The scheme flag of a multisig, before its key or its signature.
pub(crate) const MULTISIG_FLAG: u8 = 0x03;

/// A member key's signature scheme, by its Sui flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuiScheme {
    Ed25519,
    Secp256k1,
    Secp256r1,
}

impl SuiScheme {
    pub(crate) fn flag(self) -> u8 {
        match self {
            Self::Ed25519 => 0,
            Self::Secp256k1 => 1,
            Self::Secp256r1 => 2,
        }
    }

    pub(crate) fn of_flag(flag: u8) -> Option<Self> {
        match flag {
            0 => Some(Self::Ed25519),
            1 => Some(Self::Secp256k1),
            2 => Some(Self::Secp256r1),
            _ => None,
        }
    }

    /// The key's length: 32 bytes for Ed25519, a compressed point for the
    /// ECDSA curves.
    pub(crate) fn key_length(self) -> usize {
        match self {
            Self::Ed25519 => 32,
            Self::Secp256k1 | Self::Secp256r1 => 33,
        }
    }
}

/// One member of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuiMember {
    pub scheme: SuiScheme,
    pub public_key: Vec<u8>,
    pub weight: u8,
}

impl SuiMember {
    /// The key as Sui writes it: base64 of its flag and its bytes.
    pub(crate) fn sui_public_key(&self) -> String {
        let mut bytes = vec![self.scheme.flag()];
        bytes.extend(&self.public_key);
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// The address the key alone holds.
    pub(crate) fn address(&self) -> String {
        address_of(&[&[self.scheme.flag()], self.public_key.as_slice()].concat())
    }
}

/// A Sui multisig account's members and threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuiMultisig {
    pub members: Vec<SuiMember>,
    pub threshold: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyText {
    threshold: u16,
    public_keys: Vec<MemberText>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemberText {
    public_key: String,
    weight: u8,
}

fn address_of(bytes: &[u8]) -> String {
    let digest = blake2b_simd::Params::new()
        .hash_length(32)
        .to_state()
        .update(bytes)
        .finalize();
    format!("0x{}", hex::encode(digest.as_bytes()))
}

impl SuiMultisig {
    /// `text` as a multisig policy: between one and ten distinct keys of
    /// the three schemes, each weighted 1 to 255, and a threshold above
    /// zero that their weights reach.
    pub(crate) fn parse(text: &str) -> Result<Self, DerivationError> {
        let not_one = || {
            DerivationError::invalid(
                r#"A Sui multisig is {"threshold": …, "publicKeys": [{"publicKey": …, "weight": …}]}, each key in Sui's base64 form."#,
            )
        };
        let policy: PolicyText = serde_json::from_str(text.trim()).map_err(|_| not_one())?;
        let mut members = Vec::with_capacity(policy.public_keys.len());
        for member in policy.public_keys {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(member.public_key.trim())
                .map_err(|_| not_one())?;
            let (&flag, key) = bytes.split_first().ok_or_else(not_one)?;
            let scheme = SuiScheme::of_flag(flag).ok_or_else(|| {
                DerivationError::invalid(
                    "A Sui multisig's keys are Ed25519, secp256k1 or secp256r1.",
                )
            })?;
            let valid = key.len() == scheme.key_length()
                && match scheme {
                    SuiScheme::Ed25519 => <[u8; 32]>::try_from(key)
                        .ok()
                        .and_then(|key| ed25519_dalek::VerifyingKey::from_bytes(&key).ok())
                        .is_some(),
                    SuiScheme::Secp256k1 => secp256k1::PublicKey::from_slice(key).is_ok(),
                    SuiScheme::Secp256r1 => p256::PublicKey::from_sec1_bytes(key).is_ok(),
                };
            if !valid || member.weight == 0 {
                return Err(not_one());
            }
            members.push(SuiMember {
                scheme,
                public_key: key.to_vec(),
                weight: member.weight,
            });
        }
        let total: u32 = members.iter().map(|member| u32::from(member.weight)).sum();
        if members.is_empty()
            || members.len() > MAX_MEMBERS
            || policy.threshold == 0
            || u32::from(policy.threshold) > total
        {
            return Err(DerivationError::invalid(
                "A Sui multisig holds 1 to 10 keys and a threshold their weights reach.",
            ));
        }
        for (index, member) in members.iter().enumerate() {
            if members[..index]
                .iter()
                .any(|other| other.public_key == member.public_key)
            {
                return Err(DerivationError::invalid(
                    "A Sui multisig's keys must be distinct.",
                ));
            }
        }
        Ok(Self {
            members,
            threshold: policy.threshold,
        })
    }

    /// The policy in the one spelling stored.
    pub(crate) fn canonical(&self) -> String {
        serde_json::to_string(&PolicyText {
            threshold: self.threshold,
            public_keys: self
                .members
                .iter()
                .map(|member| MemberText {
                    public_key: member.sui_public_key(),
                    weight: member.weight,
                })
                .collect(),
        })
        .expect("a policy serializes")
    }

    /// `MultiSigPublicKey` in BCS: each member's scheme, key and weight,
    /// then the threshold.
    pub(crate) fn bcs(&self) -> Vec<u8> {
        let mut out = vec![self.members.len() as u8];
        for member in &self.members {
            out.push(member.scheme.flag());
            out.extend(&member.public_key);
            out.push(member.weight);
        }
        out.extend(self.threshold.to_le_bytes());
        out
    }

    /// The account's address: BLAKE2b-256 of the multisig flag, the
    /// threshold and each member's flag, key and weight.
    pub(crate) fn address(&self) -> String {
        let mut bytes = vec![MULTISIG_FLAG];
        bytes.extend(self.threshold.to_le_bytes());
        for member in &self.members {
            bytes.push(member.scheme.flag());
            bytes.extend(&member.public_key);
            bytes.push(member.weight);
        }
        address_of(&bytes)
    }
}

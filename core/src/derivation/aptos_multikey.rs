//! An Aptos MultiKey account: up to 32 keys of mixed schemes (Ed25519 and
//! secp256k1 here), of which a number must sign, held in its authentication
//! key. A fresh account's address is that key, so it needs no registration;
//! an account whose key was rotated since has another address, which is
//! read from the network before anything is signed for it.
//!
//! The policy is written as Aptos's SDK takes it to build one:
//! `{"signaturesRequired": 2, "publicKeys": ["ed25519-pub-0x…",
//! "secp256k1-pub-0x…", …]}`, each key in its AIP-80 form, a secp256k1 key
//! uncompressed. Order matters: it is the order of the account's members,
//! which the authentication key and every signature's bitmap follow.
//!
//! The account-level `0x1::multisig_account`, whose proposals and votes are
//! transactions of their own, is a different scheme and is not one of
//! these.

use serde::{Deserialize, Serialize};

use crate::derivation::error::DerivationError;

/// The keys one MultiKey holds at most.
pub(crate) const MAX_KEYS: usize = 32;
/// The authentication-key scheme of a MultiKey.
const MULTI_KEY_SCHEME: u8 = 0x03;

/// A member key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AptosKey {
    Ed25519([u8; 32]),
    /// Uncompressed, `04 ‖ x ‖ y`.
    Secp256k1([u8; 65]),
}

impl AptosKey {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix("ed25519-pub-0x") {
            let key: [u8; 32] = hex::decode(hex).ok()?.try_into().ok()?;
            ed25519_dalek::VerifyingKey::from_bytes(&key).ok()?;
            return Some(Self::Ed25519(key));
        }
        let hex = text.strip_prefix("secp256k1-pub-0x")?;
        let key = secp256k1::PublicKey::from_slice(&hex::decode(hex).ok()?).ok()?;
        Some(Self::Secp256k1(key.serialize_uncompressed()))
    }

    /// The AIP-80 spelling.
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Ed25519(key) => format!("ed25519-pub-0x{}", hex::encode(key)),
            Self::Secp256k1(key) => format!("secp256k1-pub-0x{}", hex::encode(key)),
        }
    }

    /// `AnyPublicKey` in BCS: its variant, then its bytes.
    pub(crate) fn bcs(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(key) => [&[0u8, 32][..], key].concat(),
            Self::Secp256k1(key) => [&[1u8, 65][..], key].concat(),
        }
    }

    /// The address the key alone holds, as a single-key account: the
    /// legacy Ed25519 scheme for an Ed25519 key, `SingleKey` otherwise.
    pub(crate) fn single_address(&self) -> String {
        use sha3::{Digest, Sha3_256};
        let digest = match self {
            Self::Ed25519(key) => Sha3_256::new()
                .chain_update(key)
                .chain_update([0x00])
                .finalize(),
            Self::Secp256k1(_) => Sha3_256::new()
                .chain_update(self.bcs())
                .chain_update([0x02])
                .finalize(),
        };
        format!("0x{}", hex::encode(digest))
    }
}

/// A MultiKey's members and the signatures it requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AptosMultiKey {
    pub keys: Vec<AptosKey>,
    pub required: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyText {
    signatures_required: u8,
    public_keys: Vec<String>,
}

impl AptosMultiKey {
    /// `text` as a MultiKey policy: one to 32 distinct keys and a number of
    /// required signatures from one to their count.
    pub(crate) fn parse(text: &str) -> Result<Self, DerivationError> {
        let not_one = || {
            DerivationError::invalid(
                r#"An Aptos MultiKey is {"signaturesRequired": …, "publicKeys": ["ed25519-pub-0x…", "secp256k1-pub-0x…"]}."#,
            )
        };
        let policy: PolicyText = serde_json::from_str(text.trim()).map_err(|_| not_one())?;
        let keys = policy
            .public_keys
            .iter()
            .map(|key| AptosKey::parse(key).ok_or_else(not_one))
            .collect::<Result<Vec<_>, _>>()?;
        if keys.is_empty()
            || keys.len() > MAX_KEYS
            || policy.signatures_required == 0
            || usize::from(policy.signatures_required) > keys.len()
        {
            return Err(DerivationError::invalid(
                "An Aptos MultiKey holds 1 to 32 keys and requires between one and all of them.",
            ));
        }
        for (index, key) in keys.iter().enumerate() {
            if keys[..index].contains(key) {
                return Err(DerivationError::invalid(
                    "An Aptos MultiKey's keys must be distinct.",
                ));
            }
        }
        Ok(Self {
            keys,
            required: policy.signatures_required,
        })
    }

    pub(crate) fn canonical(&self) -> String {
        serde_json::to_string(&PolicyText {
            signatures_required: self.required,
            public_keys: self.keys.iter().map(AptosKey::text).collect(),
        })
        .expect("a policy serializes")
    }

    /// `MultiKey` in BCS: its keys, then the signatures required.
    pub(crate) fn bcs(&self) -> Vec<u8> {
        let mut out = vec![self.keys.len() as u8];
        for key in &self.keys {
            out.extend(key.bcs());
        }
        out.push(self.required);
        out
    }

    /// The authentication key: SHA3-256 of the MultiKey and its scheme.
    pub(crate) fn authentication_key(&self) -> String {
        use sha3::{Digest, Sha3_256};
        let digest = Sha3_256::new()
            .chain_update(self.bcs())
            .chain_update([MULTI_KEY_SCHEME])
            .finalize();
        format!("0x{}", hex::encode(digest))
    }
}

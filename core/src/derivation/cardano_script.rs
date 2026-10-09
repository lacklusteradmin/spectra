//! A Cardano native script: the account whose address is the script's
//! hash, and whose spends carry the script and the signatures it asks for.
//! `sig` names a key hash, `all`, `any` and `atLeast` combine scripts, and
//! `after` and `before` bound the slots a spend is valid in.
//!
//! The policy is written as cardano-cli writes a simple script:
//! `{"type": "atLeast", "required": 2, "scripts": [{"type": "sig",
//! "keyHash": "…"}, …]}`. Each cosigner's key is derived along CIP-1854
//! (`m/1854'/1815'/account'/role/index`).

use serde::{Deserialize, Serialize};

use crate::derivation::error::DerivationError;
use crate::registry::Chain;

/// How deep scripts may nest: deeper than any wallet writes.
const MAX_DEPTH: usize = 8;

/// A native script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum NativeScript {
    Sig {
        #[serde(rename = "keyHash")]
        key_hash: String,
    },
    All {
        scripts: Vec<NativeScript>,
    },
    Any {
        scripts: Vec<NativeScript>,
    },
    AtLeast {
        required: u64,
        scripts: Vec<NativeScript>,
    },
    /// Valid from this slot on (`InvalidBefore`).
    After {
        slot: u64,
    },
    /// Valid before this slot (`InvalidHereafter`).
    Before {
        slot: u64,
    },
}

fn cbor_uint(n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    match n {
        0..=23 => out.push(n as u8),
        24..=0xff => out.extend([0x18, n as u8]),
        0x100..=0xffff => {
            out.push(0x19);
            out.extend((n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0x1a);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(0x1b);
            out.extend(n.to_be_bytes());
        }
    }
    out
}

fn cbor_head(major: u8, length: usize) -> Vec<u8> {
    let mut head = cbor_uint(length as u64);
    head[0] |= major << 5;
    head
}

impl NativeScript {
    /// `text` as a native script: well formed, each key hash 28 bytes, each
    /// `atLeast` asking no more than it holds, nested at most eight deep.
    pub(crate) fn parse(text: &str) -> Result<Self, DerivationError> {
        let script: Self = serde_json::from_str(text.trim()).map_err(|_| {
            DerivationError::invalid(
                "A Cardano native script is cardano-cli's JSON: sig, all, any, atLeast, after and before.",
            )
        })?;
        script.check(0)?;
        if script.key_hashes().is_empty() {
            return Err(DerivationError::invalid(
                "A Cardano multisig script names at least one key.",
            ));
        }
        Ok(script)
    }

    fn check(&self, depth: usize) -> Result<(), DerivationError> {
        if depth > MAX_DEPTH {
            return Err(DerivationError::invalid("The script nests too deep."));
        }
        match self {
            Self::Sig { key_hash } => {
                if hex::decode(key_hash)
                    .ok()
                    .filter(|bytes| bytes.len() == 28)
                    .is_none()
                {
                    return Err(DerivationError::invalid(
                        "A script's key hash is 28 bytes of hex.",
                    ));
                }
            }
            Self::All { scripts } | Self::Any { scripts } => {
                for script in scripts {
                    script.check(depth + 1)?;
                }
            }
            Self::AtLeast { required, scripts } => {
                if *required > scripts.len() as u64 {
                    return Err(DerivationError::invalid(
                        "An atLeast script asks for more than it holds.",
                    ));
                }
                for script in scripts {
                    script.check(depth + 1)?;
                }
            }
            Self::After { .. } | Self::Before { .. } => {}
        }
        Ok(())
    }

    /// The policy in the one spelling stored: key hashes lowercase.
    pub(crate) fn canonical(&self) -> String {
        serde_json::to_string(&self.lowercase()).expect("a script serializes")
    }

    fn lowercase(&self) -> Self {
        match self {
            Self::Sig { key_hash } => Self::Sig {
                key_hash: key_hash.to_ascii_lowercase(),
            },
            Self::All { scripts } => Self::All {
                scripts: scripts.iter().map(Self::lowercase).collect(),
            },
            Self::Any { scripts } => Self::Any {
                scripts: scripts.iter().map(Self::lowercase).collect(),
            },
            Self::AtLeast { required, scripts } => Self::AtLeast {
                required: *required,
                scripts: scripts.iter().map(Self::lowercase).collect(),
            },
            other => other.clone(),
        }
    }

    /// The script in CBOR: `[0, keyhash]`, `[1, scripts]`, `[2, scripts]`,
    /// `[3, n, scripts]`, `[4, slot]`, `[5, slot]`.
    pub(crate) fn cbor(&self) -> Vec<u8> {
        let list = |scripts: &[Self]| {
            let mut out = cbor_head(4, scripts.len());
            for script in scripts {
                out.extend(script.cbor());
            }
            out
        };
        let mut out = Vec::new();
        match self {
            Self::Sig { key_hash } => {
                out.extend([0x82, 0x00]);
                let bytes = hex::decode(key_hash).expect("checked");
                out.extend(cbor_head(2, bytes.len()));
                out.extend(bytes);
            }
            Self::All { scripts } => {
                out.extend([0x82, 0x01]);
                out.extend(list(scripts));
            }
            Self::Any { scripts } => {
                out.extend([0x82, 0x02]);
                out.extend(list(scripts));
            }
            Self::AtLeast { required, scripts } => {
                out.extend([0x83, 0x03]);
                out.extend(cbor_uint(*required));
                out.extend(list(scripts));
            }
            Self::After { slot } => {
                out.extend([0x82, 0x04]);
                out.extend(cbor_uint(*slot));
            }
            Self::Before { slot } => {
                out.extend([0x82, 0x05]);
                out.extend(cbor_uint(*slot));
            }
        }
        out
    }

    /// The script's hash: Blake2b-224 of the native-script tag (0) and its
    /// CBOR.
    pub(crate) fn hash(&self) -> [u8; 28] {
        use blake2::Blake2b;
        use blake2::digest::Digest;
        use blake2::digest::consts::U28;
        let mut hasher = Blake2b::<U28>::new();
        hasher.update([0x00]);
        hasher.update(self.cbor());
        hasher.finalize().into()
    }

    /// The account's address on `chain`: a Shelley enterprise address of the
    /// script's hash.
    pub(crate) fn address(&self, chain: Chain) -> Result<String, DerivationError> {
        crate::derivation::cardano::script_enterprise_address(&self.hash(), !chain.is_testnet())
    }

    /// Every key hash the script names, each once, in the order named.
    pub(crate) fn key_hashes(&self) -> Vec<[u8; 28]> {
        let mut found: Vec<[u8; 28]> = Vec::new();
        self.collect_keys(&mut found);
        found
    }

    fn collect_keys(&self, found: &mut Vec<[u8; 28]>) {
        match self {
            Self::Sig { key_hash } => {
                let hash: [u8; 28] = hex::decode(key_hash)
                    .expect("checked")
                    .try_into()
                    .expect("checked");
                if !found.contains(&hash) {
                    found.push(hash);
                }
            }
            Self::All { scripts } | Self::Any { scripts } | Self::AtLeast { scripts, .. } => {
                for script in scripts {
                    script.collect_keys(found);
                }
            }
            Self::After { .. } | Self::Before { .. } => {}
        }
    }

    /// The validity a spend takes so every time bound in the script holds,
    /// whichever branch it satisfies: from the latest `after`, before the
    /// earliest `before`.
    pub(crate) fn validity(&self) -> (Option<u64>, Option<u64>) {
        match self {
            Self::After { slot } => (Some(*slot), None),
            Self::Before { slot } => (None, Some(*slot)),
            Self::Sig { .. } => (None, None),
            Self::All { scripts } | Self::Any { scripts } | Self::AtLeast { scripts, .. } => {
                scripts.iter().map(Self::validity).fold(
                    (None, None),
                    |(start, end), (after, before)| {
                        (
                            start.max(after),
                            match (end, before) {
                                (Some(end), Some(before)) => Some(end.min(before)),
                                (end, before) => end.or(before),
                            },
                        )
                    },
                )
            }
        }
    }

    /// The fewest signatures that can satisfy the script, its time bounds
    /// aside: what it asks of its cosigners, as a threshold.
    pub(crate) fn min_signers(&self) -> u64 {
        match self {
            Self::Sig { .. } => 1,
            Self::After { .. } | Self::Before { .. } => 0,
            Self::All { scripts } => scripts.iter().map(Self::min_signers).sum(),
            Self::Any { scripts } => scripts.iter().map(Self::min_signers).min().unwrap_or(0),
            Self::AtLeast { required, scripts } => {
                let mut needs: Vec<u64> = scripts.iter().map(Self::min_signers).collect();
                needs.sort_unstable();
                needs.iter().take(*required as usize).sum()
            }
        }
    }

    /// Whether `signed` key hashes satisfy the script for a spend valid
    /// from `start` (inclusive) to `ttl` (exclusive).
    pub(crate) fn satisfied(&self, signed: &[[u8; 28]], start: Option<u64>, ttl: u64) -> bool {
        match self {
            Self::Sig { key_hash } => hex::decode(key_hash)
                .ok()
                .and_then(|hash| <[u8; 28]>::try_from(hash).ok())
                .is_some_and(|hash| signed.contains(&hash)),
            Self::All { scripts } => scripts.iter().all(|s| s.satisfied(signed, start, ttl)),
            Self::Any { scripts } => scripts.iter().any(|s| s.satisfied(signed, start, ttl)),
            Self::AtLeast { required, scripts } => {
                scripts
                    .iter()
                    .filter(|s| s.satisfied(signed, start, ttl))
                    .count() as u64
                    >= *required
            }
            Self::After { slot } => start.is_some_and(|start| start >= *slot),
            Self::Before { slot } => ttl <= *slot,
        }
    }
}

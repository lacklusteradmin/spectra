//! A Substrate multisig account (`pallet-multisig`, on Polkadot Asset Hub
//! and Bittensor): signatories and a threshold, from which its account id
//! derives, so it needs no registration.
//!
//! The policy is written as `{"threshold": 2, "signatories": ["<SS58>",
//! …]}`, each signatory an address on the account's network. Order does not
//! matter: the pallet sorts the signatories before deriving the account, and
//! so does the policy stored.

use parity_scale_codec::Encode;
use serde::{Deserialize, Serialize};

use crate::derivation::error::DerivationError;
use crate::registry::Chain;

/// The signatories an account takes at most: both runtimes'
/// `MaxSignatories`, read again from the runtime before each approval.
pub(crate) const MAX_SIGNATORIES: usize = 100;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Written {
    threshold: u16,
    signatories: Vec<String>,
}

/// A multisig account's signatories, sorted, and its threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubstrateMultisig {
    pub threshold: u16,
    pub signatories: Vec<[u8; 32]>,
    prefix: u16,
}

impl SubstrateMultisig {
    /// `text` as a policy on `chain`: each signatory once, an address on
    /// that network, and a threshold of at least two that they can meet. A
    /// threshold of one is any signatory's own account; the pallet spends
    /// it through another call, which Spectra does not make.
    pub(crate) fn parse(chain: Chain, text: &str) -> Result<Self, DerivationError> {
        let prefix = chain
            .ss58_prefix()
            .ok_or_else(|| DerivationError::invalid("Not a Substrate network."))?;
        let written: Written = serde_json::from_str(text.trim()).map_err(|_| {
            DerivationError::invalid(
                r#"A Substrate multisig is {"threshold": n, "signatories": [addresses]}."#,
            )
        })?;
        let mut signatories = written
            .signatories
            .iter()
            .map(|address| {
                crate::derivation::primitives::decode_ss58(address.trim(), Some(prefix))
                    .map(|(_, key)| key)
                    .map_err(|_| {
                        DerivationError::refused(
                            "A signatory is not an address on %@: %@",
                            [chain.chain_display_name(), address.trim()],
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        signatories.sort_unstable();
        if signatories.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(DerivationError::invalid("A signatory is named twice."));
        }
        if !(2..=MAX_SIGNATORIES).contains(&signatories.len()) {
            return Err(DerivationError::invalid(
                "A Substrate multisig has from 2 to 100 signatories.",
            ));
        }
        if written.threshold < 2 || usize::from(written.threshold) > signatories.len() {
            return Err(DerivationError::invalid(
                "The threshold is at least 2 and at most the signatories named.",
            ));
        }
        Ok(Self {
            threshold: written.threshold,
            signatories,
            prefix,
        })
    }

    /// The policy in the one spelling stored: the signatories sorted.
    pub(crate) fn canonical(&self) -> String {
        serde_json::to_string(&Written {
            threshold: self.threshold,
            signatories: self
                .signatories
                .iter()
                .map(|key| self.address_of(key))
                .collect(),
        })
        .expect("a policy serializes")
    }

    /// `account` as an address on the account's network.
    pub(crate) fn address_of(&self, account: &[u8; 32]) -> String {
        crate::derivation::primitives::encode_ss58(account, self.prefix)
    }

    /// The account id: `blake2_256("modlpy/utilisuba" ‖ (signatories,
    /// threshold))`, SCALE-encoded, the signatories sorted.
    pub(crate) fn account_id(&self) -> [u8; 32] {
        use blake2::digest::consts::U32;
        use blake2::{Blake2b, Digest};
        let mut hasher = Blake2b::<U32>::new();
        hasher.update(b"modlpy/utilisuba");
        hasher.update((&self.signatories, self.threshold).encode());
        hasher.finalize().into()
    }

    pub(crate) fn address(&self) -> String {
        self.address_of(&self.account_id())
    }

    /// The signatories other than `signatory`, sorted, as each approval
    /// names them.
    pub(crate) fn others(&self, signatory: &[u8; 32]) -> Vec<[u8; 32]> {
        self.signatories
            .iter()
            .filter(|key| *key != signatory)
            .copied()
            .collect()
    }
}

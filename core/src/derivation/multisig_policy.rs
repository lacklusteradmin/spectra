//! A multisig account's policy on whichever network it is, read from the
//! text a watch import gives: the network's own form, parsed by the module
//! that owns it. The policy is what the account's address derives from, so
//! it is stored canonically with the wallet, and every session is judged
//! against it.

use crate::derivation::aptos_multikey::AptosMultiKey;
use crate::derivation::cardano_script::NativeScript;
use crate::derivation::error::DerivationError;
use crate::derivation::multisig::MultisigPolicy;
use crate::derivation::substrate_multisig::SubstrateMultisig;
use crate::derivation::sui_multisig::SuiMultisig;
use crate::registry::Chain;

/// A policy whose address derives from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccountPolicy {
    /// A UTXO network's `sortedmulti` descriptor.
    Utxo(MultisigPolicy),
    Sui(SuiMultisig),
    Aptos(AptosMultiKey),
    Cardano(NativeScript),
    Substrate(SubstrateMultisig),
}

impl AccountPolicy {
    /// `text` as a multisig policy on `chain`, refused on a network whose
    /// multisig accounts are not derived from one.
    pub(crate) fn parse(chain: Chain, text: &str) -> Result<Self, DerivationError> {
        if chain.utxo_multisig_script().is_some() {
            return MultisigPolicy::parse(chain, text).map(Self::Utxo);
        }
        match chain.mainnet_counterpart() {
            Chain::Sui => SuiMultisig::parse(text).map(Self::Sui),
            Chain::Aptos => AptosMultiKey::parse(text).map(Self::Aptos),
            Chain::Cardano => NativeScript::parse(text).map(Self::Cardano),
            Chain::Polkadot | Chain::Bittensor => {
                SubstrateMultisig::parse(chain, text).map(Self::Substrate)
            }
            _ => Err(DerivationError::refused(
                "%@ has no multisig wallets.",
                [chain.chain_display_name()],
            )),
        }
    }

    /// The policy in the one spelling stored.
    pub(crate) fn canonical(&self) -> String {
        match self {
            Self::Utxo(policy) => policy.descriptor(),
            Self::Sui(policy) => policy.canonical(),
            Self::Aptos(policy) => policy.canonical(),
            Self::Cardano(script) => script.canonical(),
            Self::Substrate(policy) => policy.canonical(),
        }
    }

    /// The account's address: a UTXO account's first receive address.
    pub(crate) fn address(&self, chain: Chain) -> Result<String, DerivationError> {
        match self {
            Self::Utxo(policy) => policy.address(chain, (0, 0)),
            Self::Sui(policy) => Ok(policy.address()),
            Self::Aptos(policy) => Ok(policy.authentication_key()),
            Self::Cardano(script) => script.address(chain),
            Self::Substrate(policy) => Ok(policy.address()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 2-of-3 policy on `chain` over the multisig fixture's three
    /// phrases: a UTXO network's descriptor, and elsewhere each phrase's key
    /// on the network's default path.
    pub(crate) fn policy_of_phrases(chain: Chain) -> String {
        if chain.utxo_multisig_script().is_some() {
            return crate::derivation::multisig::tests::descriptor_of_phrases(chain);
        }
        let phrases = crate::derivation::multisig::tests::fixture()["phrases"].clone();
        if chain.mainnet_counterpart() == Chain::Cardano {
            let keys: Vec<serde_json::Value> = phrases
                .as_array()
                .unwrap()
                .iter()
                .map(|phrase| {
                    let (_, _, hash) =
                        crate::derivation::cardano::cosigner_keys(phrase.as_str().unwrap(), "", 1)
                            .unwrap()
                            .remove(0);
                    serde_json::json!({"type": "sig", "keyHash": hex::encode(hash)})
                })
                .collect();
            return serde_json::json!({"type": "atLeast", "required": 2, "scripts": keys})
                .to_string();
        }
        let keys: Vec<[u8; 32]> = crate::derivation::multisig::tests::fixture()["phrases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|phrase| {
                let derived = crate::derivation::dispatch::derive_for_chain(
                    chain,
                    phrase.as_str().unwrap(),
                    &crate::derivation::path::default_path_from_catalog(chain).unwrap(),
                    None,
                    None,
                    None,
                    false,
                    true,
                    false,
                )
                .unwrap();
                hex::decode(derived.public_key_hex.unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap()
            })
            .collect();
        match chain.mainnet_counterpart() {
            Chain::Sui => {
                use base64::Engine;
                let keys: Vec<serde_json::Value> = keys
                    .iter()
                    .map(|key| {
                        serde_json::json!({
                            "publicKey": base64::engine::general_purpose::STANDARD
                                .encode([&[0u8][..], key].concat()),
                            "weight": 1,
                        })
                    })
                    .collect();
                serde_json::json!({"threshold": 2, "publicKeys": keys}).to_string()
            }
            Chain::Aptos => {
                let keys: Vec<String> = keys
                    .iter()
                    .map(|key| format!("ed25519-pub-0x{}", hex::encode(key)))
                    .collect();
                serde_json::json!({"signaturesRequired": 2, "publicKeys": keys}).to_string()
            }
            Chain::Polkadot | Chain::Bittensor => {
                let signatories: Vec<String> = keys
                    .iter()
                    .map(|key| {
                        crate::derivation::primitives::encode_ss58(
                            key,
                            chain.ss58_prefix().unwrap(),
                        )
                    })
                    .collect();
                serde_json::json!({"threshold": 2, "signatories": signatories}).to_string()
            }
            _ => panic!("{chain} has no multisig policy"),
        }
    }
}

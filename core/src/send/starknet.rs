//! Starknet: an ERC-20 `transfer` from a Ready account as an `INVOKE` v3,
//! preceded by the account's `DEPLOY_ACCOUNT` v3 while it has none, each
//! hashed with Poseidon as the protocol defines and signed with the Stark
//! key. Reads and submission are `api::starknet_json_rpc`.
//!
//! One artifact holds both transactions: a send from an account that is not
//! deployed yet is its deployment and its transfer, reviewed with both fees,
//! signed together and submitted in order. The deployment takes nonce zero
//! and the transfer nonce one, and the network runs a transfer at nonce one
//! right behind the deployment it waits for. The account's public key, which
//! the deployment names, comes from the wallet; signing checks the key it
//! signs with is that key.

use serde::{Deserialize, Serialize};
use starknet_crypto::Felt;

use crate::api::starknet_json_rpc::{
    ResourceBounds, StarknetDeployAccount, StarknetInvoke, StarknetSignedDeployment,
    StarknetSignedTransfer,
};
use crate::derivation::starknet::{felt_serde, parse_felt};
use crate::registry::Chain;
use crate::send::error::SendError;
use crate::send::stages::units_u128;

/// The deployment a transfer from an undeployed account sends first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedStarknetDeployment {
    /// The owner's public key, which is also the address's salt.
    #[serde(with = "felt_serde")]
    pub(crate) public_key: Felt,
    #[serde(with = "felt_serde")]
    pub(crate) class_hash: Felt,
    pub(crate) resource_bounds: ResourceBounds,
}

/// A transfer of an ERC-20 on Starknet — STRK, the network's own coin, or a
/// token — built and ready to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedStarknetTransfer {
    /// The chain id the hashes bind, as its short string.
    pub(crate) chain_id: String,
    #[serde(with = "felt_serde")]
    pub(crate) sender: Felt,
    /// The ERC-20 contract the transfer calls.
    #[serde(with = "felt_serde")]
    pub(crate) token: Felt,
    #[serde(with = "felt_serde")]
    pub(crate) recipient: Felt,
    #[serde(with = "units_u128")]
    pub(crate) amount: u128,
    /// The transfer's nonce: one when the deployment goes first.
    pub(crate) nonce: u64,
    pub(crate) resource_bounds: ResourceBounds,
    pub(crate) deployment: Option<PreparedStarknetDeployment>,
}

/// The terms a Starknet transfer is built from.
pub(crate) struct StarknetTransferTerms<'a> {
    pub sender: &'a str,
    pub token: &'a str,
    pub recipient: &'a str,
    pub amount: u128,
    pub nonce: u64,
    pub resource_bounds: ResourceBounds,
    /// The deployment's public key and bounds, when the account has none.
    pub deployment: Option<(Felt, ResourceBounds)>,
}

/// A prefix the protocol hashes a transaction's kind as.
fn short_string(text: &str) -> Felt {
    Felt::from_bytes_be_slice(text.as_bytes())
}

/// A transfer of `terms.token` on `chain`, refused before anything is
/// signed where the network would refuse it or it would pay the wrong
/// account: addresses that are not Starknet's, a zero amount, a deployment
/// whose key does not own the sender, or a nonce that does not follow it.
pub(crate) fn prepare_transfer(
    chain: Chain,
    terms: StarknetTransferTerms<'_>,
) -> Result<PreparedStarknetTransfer, SendError> {
    let network = chain.starknet_network()?;
    let sender = crate::derivation::starknet::decode_address(chain, terms.sender)?;
    let recipient = crate::derivation::starknet::decode_address(chain, terms.recipient)?;
    let token = crate::derivation::starknet::decode_address(chain, terms.token)?;
    if terms.amount == 0 {
        return Err(SendError::invalid("Amount must be greater than zero"));
    }
    let deployment = match terms.deployment {
        Some((public_key, resource_bounds)) => {
            if crate::derivation::starknet::account_address(chain, &public_key)? != sender {
                return Err(SendError::invalid(
                    "The Starknet account's key does not own its address",
                ));
            }
            if terms.nonce != 1 {
                return Err(SendError::invalid(
                    "A transfer behind its account's deployment takes nonce one",
                ));
            }
            Some(PreparedStarknetDeployment {
                public_key,
                class_hash: parse_felt(network.account_class_hash)
                    .ok_or_else(|| SendError::invalid("Invalid Starknet account class"))?,
                resource_bounds,
            })
        }
        None => None,
    };
    Ok(PreparedStarknetTransfer {
        chain_id: network.chain_id.to_string(),
        sender,
        token,
        recipient,
        amount: terms.amount,
        nonce: terms.nonce,
        resource_bounds: terms.resource_bounds,
        deployment,
    })
}

/// Poseidon over a v3 transaction's resource bounds and tip (zero): each
/// bound as its resource's name, its amount and its price in one felt.
fn fee_field_hash(bounds: &ResourceBounds) -> Felt {
    let resource = |name: &[u8], bound: &crate::api::starknet_json_rpc::ResourceBound| {
        let mut word = [0u8; 32];
        word[8 - name.len()..8].copy_from_slice(name);
        word[8..16].copy_from_slice(&bound.max_amount.to_be_bytes());
        word[16..].copy_from_slice(&bound.max_price_per_unit.to_be_bytes());
        Felt::from_bytes_be(&word)
    };
    starknet_crypto::poseidon_hash_many(&[
        Felt::ZERO,
        resource(b"L1_GAS", &bounds.l1_gas),
        resource(b"L2_GAS", &bounds.l2_gas),
        resource(b"L1_DATA", &bounds.l1_data_gas),
    ])
}

/// The fields every v3 hash starts with, after its kind and version: the
/// account, the fee field, no paymaster, the chain, the nonce and both
/// data-availability modes on L1 (zero).
fn common_fields(
    kind: &str,
    account: Felt,
    bounds: &ResourceBounds,
    chain_id: Felt,
    nonce: u64,
) -> Vec<Felt> {
    vec![
        short_string(kind),
        Felt::THREE,
        account,
        fee_field_hash(bounds),
        starknet_crypto::poseidon_hash_many(&[]),
        chain_id,
        Felt::from(nonce),
        Felt::ZERO,
    ]
}

/// A Stark ECDSA signature over `hash` with an RFC 6979 nonce, retried with
/// a seed counting up from one in the rare case the nonce gives an `r` or
/// `s` outside the curve's bound, as starknet-rs and cairo-lang do.
fn sign_hash(key: &Felt, hash: &Felt) -> Result<[Felt; 2], SendError> {
    let mut seed: Option<Felt> = None;
    loop {
        let k = starknet_crypto::rfc6979_generate_k(hash, key, seed.as_ref());
        match starknet_crypto::sign(key, hash, &k) {
            Ok(signature) => return Ok([signature.r, signature.s]),
            Err(starknet_crypto::SignError::InvalidK) => {
                seed = Some(seed.map_or(Felt::ONE, |seed| seed + Felt::ONE));
            }
            Err(error) => return Err(SendError::invalid(format!("Stark signature: {error}"))),
        }
    }
}

impl PreparedStarknetTransfer {
    fn chain_id_felt(&self) -> Felt {
        short_string(&self.chain_id)
    }

    /// The `__execute__` calldata of one call: `transfer(recipient, u256
    /// amount)` on the token, the amount as its low and high words.
    pub(crate) fn calldata(&self) -> Vec<Felt> {
        vec![
            Felt::ONE,
            self.token,
            crate::derivation::starknet::selector("transfer"),
            Felt::THREE,
            self.recipient,
            Felt::from(self.amount),
            Felt::ZERO,
        ]
    }

    /// The transfer's hash.
    pub(crate) fn invoke_hash(&self) -> Felt {
        let mut fields = common_fields(
            "invoke",
            self.sender,
            &self.resource_bounds,
            self.chain_id_felt(),
            self.nonce,
        );
        fields.push(starknet_crypto::poseidon_hash_many(&[]));
        fields.push(starknet_crypto::poseidon_hash_many(&self.calldata()));
        starknet_crypto::poseidon_hash_many(&fields)
    }

    /// The deployment's hash, when the transfer has one.
    pub(crate) fn deploy_hash(&self) -> Option<Felt> {
        let deployment = self.deployment.as_ref()?;
        let mut fields = common_fields(
            "deploy_account",
            self.sender,
            &deployment.resource_bounds,
            self.chain_id_felt(),
            0,
        );
        fields.push(starknet_crypto::poseidon_hash_many(
            &crate::derivation::starknet::constructor_calldata(&deployment.public_key),
        ));
        fields.push(deployment.class_hash);
        fields.push(deployment.public_key);
        Some(starknet_crypto::poseidon_hash_many(&fields))
    }

    /// The most the deployment and the transfer together let the network
    /// charge, in fri.
    pub(crate) fn max_fee(&self) -> Result<u128, SendError> {
        let overflow = || SendError::invalid("Starknet fee bound overflows");
        let transfer = self.resource_bounds.max_fee().ok_or_else(overflow)?;
        match &self.deployment {
            None => Ok(transfer),
            Some(deployment) => deployment
                .resource_bounds
                .max_fee()
                .and_then(|deploy| deploy.checked_add(transfer))
                .ok_or_else(overflow),
        }
    }

    /// The transactions, unsigned, as a fee estimate simulates them: with
    /// `bounds` (zero in an estimate) and a signature the account checks
    /// and, in an estimate, accepts — `r` a point of the curve, its
    /// generator's x — so the check costs what a real one does.
    pub(crate) fn unsigned(
        &self,
        transfer_bounds: ResourceBounds,
        deploy_bounds: ResourceBounds,
    ) -> (Option<StarknetDeployAccount>, StarknetInvoke) {
        let signature = vec![
            parse_felt("0x1ef15c18599971b7beced415a40f0c7deacfd9b0d1819e03d723d8bc943cfca")
                .expect("the Stark curve's generator x"),
            Felt::ONE,
        ];
        let deploy = self
            .deployment
            .as_ref()
            .map(|deployment| StarknetDeployAccount {
                class_hash: deployment.class_hash,
                contract_address_salt: deployment.public_key,
                constructor_calldata: crate::derivation::starknet::constructor_calldata(
                    &deployment.public_key,
                )
                .to_vec(),
                signature: signature.clone(),
                resource_bounds: deploy_bounds,
            });
        let invoke = StarknetInvoke {
            sender_address: self.sender,
            calldata: self.calldata(),
            signature,
            nonce: self.nonce,
            resource_bounds: transfer_bounds,
        };
        (deploy, invoke)
    }

    /// The signed transactions and the transfer's hash. Refused unless the
    /// prepared network is `chain`'s and `key` owns the sender: the
    /// deployed account's owner, or the key the deployment names.
    pub(crate) fn sign(
        &self,
        chain: Chain,
        key: &Felt,
    ) -> Result<StarknetSignedTransfer, SendError> {
        let network = chain.starknet_network()?;
        if self.chain_id != network.chain_id {
            return Err(SendError::invalid(
                "Starknet transaction was prepared for another network",
            ));
        }
        // The address is the registry's class's for this key, so a
        // deployment of another class would deploy another account.
        let public = crate::derivation::starknet::public_key(key);
        if crate::derivation::starknet::account_address(chain, &public)? != self.sender
            || self.deployment.as_ref().is_some_and(|deployment| {
                deployment.public_key != public
                    || Some(deployment.class_hash) != parse_felt(network.account_class_hash)
            })
        {
            return Err(SendError::invalid(
                "Starknet sender does not match the signing key",
            ));
        }
        let (deploy, mut invoke) = self.unsigned(
            self.resource_bounds,
            self.deployment
                .as_ref()
                .map_or(ResourceBounds::ZERO, |d| d.resource_bounds),
        );
        let invoke_hash = self.invoke_hash();
        invoke.signature = sign_hash(key, &invoke_hash)?.to_vec();
        let deploy_account = match (deploy, self.deploy_hash()) {
            (Some(mut transaction), Some(hash)) => {
                transaction.signature = sign_hash(key, &hash)?.to_vec();
                Some(StarknetSignedDeployment { transaction, hash })
            }
            _ => None,
        };
        Ok(StarknetSignedTransfer {
            deploy_account,
            invoke,
            invoke_hash,
        })
    }
}

#[cfg(test)]
#[path = "tests/starknet.rs"]
mod tests;

//! VeChainThor: a transfer of VET, or of a VIP-180 token (VTHO among them),
//! as the one clause of a dynamic-fee transaction (VIP-251's type `0x51`, in
//! VIP-252's envelope), signed with the sender's secp256k1 key. Reads and
//! submission are `api::thor_rest`.
//!
//! The signed bytes are `0x51 ‖ rlp([chainTag, blockRef, expiration,
//! clauses, maxPriorityFeePerGas, maxFeePerGas, gas, dependsOn, nonce,
//! reserved, signature])`, as thor's `tx/tx_dynamic_fee.go` writes them. The
//! signing hash is BLAKE2b-256 of the same without the signature, the
//! signature 65 bytes of `r ‖ s ‖ recovery id`, and the transaction's id
//! BLAKE2b-256 of the signing hash and the signer's address. A transfer
//! depends on no other transaction and asks for no feature (no fee
//! delegation), so `dependsOn` is empty and `reserved` an empty list.

use alloy_rlp::{Encodable, Header};
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};

use crate::registry::Chain;
use crate::send::error::SendError;
use crate::send::stages::units_u128;

/// VIP-251's dynamic-fee transaction type.
pub(crate) const TYPE_DYNAMIC_FEE: u8 = 0x51;

/// The gas every transaction pays before its clauses run, and each clause
/// that calls an address rather than creating a contract.
const TX_GAS: u64 = 5_000;
const CLAUSE_GAS: u64 = 16_000;
/// The gas each byte of a clause's data costs, zero and non-zero.
const ZERO_BYTE_GAS: u64 = 4;
const NONZERO_BYTE_GAS: u64 = 68;

/// The most gas one transaction may provide (thor's `MaxTxGasLimit`, the
/// EIP-7825 cap).
pub(crate) const MAX_TX_GAS: u64 = 1 << 24;

/// A transfer of VET or of one VIP-180 token, built and ready to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedVechainTransfer {
    /// The network's genesis id's last byte, which the transaction binds.
    pub(crate) chain_tag: u8,
    pub(crate) sender: String,
    /// Who the transfer pays: the clause's address for VET, the argument of
    /// the token's `transfer` for a token.
    pub(crate) recipient: String,
    /// The VIP-180 contract the clause calls; `None` for VET.
    pub(crate) token: Option<String>,
    #[serde(with = "units_u128")]
    pub(crate) amount: u128,
    /// The first eight bytes of the block the transaction is built on: its
    /// number, then the start of its id.
    #[serde(with = "hex")]
    pub(crate) block_ref: [u8; 8],
    /// How many blocks past `block_ref` the transaction may be included in.
    pub(crate) expiration: u32,
    #[serde(with = "units_u128")]
    pub(crate) max_priority_fee_per_gas: u128,
    #[serde(with = "units_u128")]
    pub(crate) max_fee_per_gas: u128,
    pub(crate) gas: u64,
    /// Random: what makes two otherwise equal transfers two transactions.
    #[serde(with = "hex")]
    pub(crate) nonce: [u8; 8],
}

/// The terms a VeChainThor transfer is built from.
pub(crate) struct VechainTransferTerms<'a> {
    pub sender: &'a str,
    pub recipient: &'a str,
    pub token: Option<&'a str>,
    pub amount: u128,
    pub block_ref: [u8; 8],
    pub expiration: u32,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas: u64,
    pub nonce: [u8; 8],
}

/// The 20 bytes an address names, refused unless it is one: `0x` and 40
/// hex digits, in one case or with a correct checksum.
pub(crate) fn address_bytes(chain: Chain, address: &str) -> Result<[u8; 20], SendError> {
    let valid = crate::validation::address::validate_address(chain, address);
    let normalized = valid
        .normalized_value
        .filter(|_| valid.is_valid)
        .ok_or_else(|| {
            SendError::refused(
                "Not a %@ address.",
                [chain.chain_display_name().to_string()],
            )
        })?;
    let bytes = hex::decode(&normalized[2..])
        .map_err(|_| SendError::invalid("Not a VeChainThor address"))?;
    bytes
        .try_into()
        .map_err(|_| SendError::invalid("Not a VeChainThor address"))
}

fn lowercase(bytes: &[u8; 20]) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// The intrinsic gas of a transaction with one clause calling an address
/// with `data`: what it costs before anything runs.
pub(crate) fn intrinsic_gas(data: &[u8]) -> u64 {
    let zeros = data.iter().filter(|byte| **byte == 0).count() as u64;
    let others = data.len() as u64 - zeros;
    TX_GAS + CLAUSE_GAS + zeros * ZERO_BYTE_GAS + others * NONZERO_BYTE_GAS
}

/// A transfer on `chain`, checked for what the network would refuse before
/// any of it is signed: addresses that are not addresses, a zero amount, gas
/// below the clause's intrinsic gas or above the network's cap, a fee cap
/// below its own tip, and no expiration.
pub(crate) fn prepare_transfer(
    chain: Chain,
    terms: VechainTransferTerms<'_>,
) -> Result<PreparedVechainTransfer, SendError> {
    let network = chain.thor_network()?;
    let sender = address_bytes(chain, terms.sender)?;
    let recipient = address_bytes(chain, terms.recipient)?;
    let token = terms
        .token
        .map(|token| address_bytes(chain, token))
        .transpose()?;
    if terms.amount == 0 {
        return Err(SendError::invalid("Amount must be greater than zero"));
    }
    if terms.expiration == 0 {
        return Err(SendError::invalid(
            "A VeChainThor transaction needs an expiration",
        ));
    }
    if terms.max_fee_per_gas < terms.max_priority_fee_per_gas {
        return Err(SendError::invalid(
            "The fee cap is below the priority fee it includes",
        ));
    }
    let prepared = PreparedVechainTransfer {
        chain_tag: network.chain_tag(),
        sender: lowercase(&sender),
        recipient: lowercase(&recipient),
        token: token.as_ref().map(lowercase),
        amount: terms.amount,
        block_ref: terms.block_ref,
        expiration: terms.expiration,
        max_priority_fee_per_gas: terms.max_priority_fee_per_gas,
        max_fee_per_gas: terms.max_fee_per_gas,
        gas: terms.gas,
        nonce: terms.nonce,
    };
    let (_, _, data) = prepared.clause()?;
    if terms.gas < intrinsic_gas(&data) || terms.gas > MAX_TX_GAS {
        return Err(SendError::invalid(
            "Gas is outside what a VeChainThor transfer can provide",
        ));
    }
    Ok(prepared)
}

impl PreparedVechainTransfer {
    /// The one clause: VET to the recipient, or the token's
    /// `transfer(recipient, amount)` with no VET.
    pub(crate) fn clause(&self) -> Result<(String, u128, Vec<u8>), SendError> {
        Ok(match &self.token {
            None => (self.recipient.clone(), self.amount, Vec::new()),
            Some(token) => (
                token.clone(),
                0,
                crate::send::evm::encode_erc20_transfer(&self.recipient, self.amount)?,
            ),
        })
    }

    /// The block the transaction is built on, and the last it may be
    /// included in.
    pub(crate) fn last_block(&self) -> u64 {
        let reference = u32::from_be_bytes(self.block_ref[..4].try_into().expect("4 bytes"));
        u64::from(reference) + u64::from(self.expiration)
    }

    /// The RLP list's items, the signature last when there is one.
    fn encode(&self, signature: Option<&[u8; 65]>) -> Result<Vec<u8>, SendError> {
        let (to, value, data) = self.clause()?;
        let to = hex::decode(&to[2..]).map_err(SendError::invalid)?;
        let mut clause = Vec::new();
        to.as_slice().encode(&mut clause);
        value.encode(&mut clause);
        data.as_slice().encode(&mut clause);
        let mut clauses = Vec::new();
        Header {
            list: true,
            payload_length: clause.len(),
        }
        .encode(&mut clauses);
        clauses.extend(clause);

        let mut payload = Vec::new();
        self.chain_tag.encode(&mut payload);
        u64::from_be_bytes(self.block_ref).encode(&mut payload);
        self.expiration.encode(&mut payload);
        Header {
            list: true,
            payload_length: clauses.len(),
        }
        .encode(&mut payload);
        payload.extend(clauses);
        self.max_priority_fee_per_gas.encode(&mut payload);
        self.max_fee_per_gas.encode(&mut payload);
        self.gas.encode(&mut payload);
        // `dependsOn`: none, an empty string.
        payload.push(alloy_rlp::EMPTY_STRING_CODE);
        u64::from_be_bytes(self.nonce).encode(&mut payload);
        // `reserved`: no features, trimmed to an empty list.
        payload.push(alloy_rlp::EMPTY_LIST_CODE);
        if let Some(signature) = signature {
            signature.as_slice().encode(&mut payload);
        }
        let mut out = vec![TYPE_DYNAMIC_FEE];
        Header {
            list: true,
            payload_length: payload.len(),
        }
        .encode(&mut out);
        out.extend(payload);
        Ok(out)
    }

    /// The bytes the signing hash is taken over: the type byte and the
    /// unsigned list.
    pub(crate) fn unsigned(&self) -> Result<Vec<u8>, SendError> {
        self.encode(None)
    }

    /// BLAKE2b-256 of [`Self::unsigned`].
    pub(crate) fn signing_hash(&self) -> Result<[u8; 32], SendError> {
        Ok(crate::derivation::primitives::blake2b::<32>(&[
            &self.unsigned()?
        ]))
    }

    /// The signed transaction and its id, `0x` and lowercase hex as nodes
    /// and explorers write it. Refused unless the prepared network is
    /// `chain`'s and `key` holds the prepared sender.
    pub(crate) fn sign(
        &self,
        chain: Chain,
        key: &SecretKey,
    ) -> Result<(Vec<u8>, String), SendError> {
        if self.chain_tag != chain.thor_network()?.chain_tag() {
            return Err(SendError::invalid(
                "VeChainThor transaction was prepared for another network",
            ));
        }
        let public = PublicKey::from_secret_key(&Secp256k1::new(), key);
        let hash = crate::derivation::evm::keccak256(&public.serialize_uncompressed()[1..]);
        let origin: [u8; 20] = hash[12..].try_into().expect("20 bytes");
        if lowercase(&origin) != self.sender {
            return Err(SendError::invalid(
                "VeChainThor sender does not match the signing key",
            ));
        }
        let signing_hash = self.signing_hash()?;
        // libsecp256k1 signs deterministically (RFC 6979) with a low S,
        // which a node's pool requires.
        let (recovery, compact) = Secp256k1::signing_only()
            .sign_ecdsa_recoverable(&Message::from_digest(signing_hash), key)
            .serialize_compact();
        let mut signature = [0u8; 65];
        signature[..64].copy_from_slice(&compact);
        signature[64] = u8::try_from(recovery.to_i32()).expect("a recovery id is 0 to 3");
        let raw = self.encode(Some(&signature))?;
        let id = crate::derivation::primitives::blake2b::<32>(&[&signing_hash, &origin]);
        Ok((raw, format!("0x{}", hex::encode(id))))
    }
}

#[cfg(test)]
#[path = "tests/vechain.rs"]
mod tests;

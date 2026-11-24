//! Tezos: a transfer of tez, led by the `reveal` of the sender's key while
//! that key is not on chain, forged here in the encoding of the protocol
//! that will apply it and signed with the sender's Ed25519 key. Reads,
//! simulation and injection are `api::tezos_rpc`.
//!
//! The bytes signed are this module's, never a node's `forge` answer: the
//! branch block's 32-byte hash, then each operation —
//!
//! - reveal: `6b`, the source's 21-byte key hash, then its fee, counter, gas
//!   limit and storage limit as Zarith naturals, the public key (`00` and
//!   its 32 bytes) and `00` for no BLS proof;
//! - transaction: `6c`, the same five fields, the amount, the destination's
//!   22 bytes and `00` for no parameters: a plain transfer, to an account or
//!   to a contract's default entrypoint.
//!
//! The signature is Ed25519 over BLAKE2b-256 of `03`, the generic-operation
//! watermark, and those bytes, and is appended as its 64 bytes; the
//! operation's hash is BLAKE2b-256 of the signed bytes, written `o…`.
//!
//! The prepared transfer holds every term the review binds. The reveal's
//! key is written at signing, when the key is at hand: the sender's address
//! is that key's hash, so it adds no term.

use serde::{Deserialize, Serialize};

use crate::api::tezos_rpc::TezosContent;
use crate::derivation::primitives::blake2b;
use crate::derivation::tezos::{
    TezosAddress, TezosCurve, base58check, decode_address, decode_base58check, prefix,
    public_key_string,
};
use crate::registry::{Chain, TezosNetwork, TezosProtocol};
use crate::send::error::SendError;
use crate::send::keys::Ed25519Seed;
use crate::send::stages::units_u128;

const REVEAL_TAG: u8 = 0x6b;
const TRANSACTION_TAG: u8 = 0x6c;
/// An Ed25519 public key's tag in a reveal.
const ED25519_KEY_TAG: u8 = 0x00;
/// The watermark an operation is signed under: a generic manager operation.
const OPERATION_WATERMARK: u8 = 0x03;
const SIGNATURE_BYTES: u64 = 64;
/// The most a Tezos amount or fee can be: `Tez_repr` is a signed 64-bit
/// count of mutez.
const MAX_MUTEZ: u128 = i64::MAX as u128;

/// One manager operation's fee, counter and limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TezosLimits {
    /// What it pays the baker, in mutez.
    pub(crate) fee: u64,
    pub(crate) counter: u64,
    pub(crate) gas_limit: u64,
    /// The most storage, in bytes, it may burn for.
    pub(crate) storage_limit: u64,
}

/// A transfer of tez, built and ready to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedTezosTransfer {
    /// The chain id a node confirmed before the transfer was built.
    pub(crate) chain_id: String,
    /// The protocol whose encoding the operation is forged in.
    pub(crate) protocol: String,
    /// The block the operation is branched from.
    pub(crate) branch: String,
    /// The first level the operation can no longer be included at: its
    /// branch's level and the protocol's `max_operations_ttl`.
    pub(crate) expires_at_level: u64,
    pub(crate) source: String,
    pub(crate) destination: String,
    /// In mutez.
    #[serde(with = "units_u128")]
    pub(crate) amount: u128,
    /// The reveal ahead of the transfer, while the sender's key is not on
    /// chain.
    pub(crate) reveal: Option<TezosLimits>,
    pub(crate) transaction: TezosLimits,
}

/// The terms a transfer is built from.
pub(crate) struct TezosTransferTerms<'a> {
    pub source: &'a str,
    pub destination: &'a str,
    /// In mutez.
    pub amount: u128,
    /// The protocol the node runs, which must be one `registry` lists.
    pub protocol: &'a str,
    pub branch: &'a str,
    pub branch_level: u64,
    /// The sender's counter now; the batch takes the ones after it.
    pub counter: u64,
    /// The reveal's gas limit, when the sender's key is not on chain.
    pub reveal_gas_limit: Option<u64>,
    pub gas_limit: u64,
    pub storage_limit: u64,
}

/// A transfer of `chain`'s tez on the terms given, its fees the network's
/// minimal fee for its size and gas limits plus the registry's margin, and
/// refused before anything is signed when the network would refuse it.
pub(crate) fn prepare_transfer(
    chain: Chain,
    terms: TezosTransferTerms<'_>,
) -> Result<PreparedTezosTransfer, SendError> {
    let network = chain.tezos_network()?;
    let protocol = known_protocol(network, terms.protocol)?;
    let first = terms
        .counter
        .checked_add(1)
        .ok_or_else(|| SendError::invalid("Tezos counter overflows"))?;
    let reveal = terms.reveal_gas_limit.map(|gas_limit| TezosLimits {
        fee: 0,
        counter: first,
        gas_limit,
        storage_limit: 0,
    });
    let unpriced = PreparedTezosTransfer {
        chain_id: network.chain_id.to_string(),
        protocol: protocol.hash.to_string(),
        branch: terms.branch.to_string(),
        expires_at_level: terms
            .branch_level
            .checked_add(protocol.max_operations_ttl)
            .ok_or_else(|| SendError::invalid("Tezos level overflows"))?,
        source: terms.source.to_string(),
        destination: terms.destination.to_string(),
        amount: terms.amount,
        reveal,
        transaction: TezosLimits {
            fee: 0,
            counter: first + u64::from(reveal.is_some()),
            gas_limit: terms.gas_limit,
            storage_limit: terms.storage_limit,
        },
    };
    let prepared = unpriced.priced(network)?;
    prepared.validate(chain)?;
    Ok(prepared)
}

fn known_protocol(network: &TezosNetwork, hash: &str) -> Result<&'static TezosProtocol, SendError> {
    network.protocol(hash).ok_or_else(|| {
        SendError::refused(
            "Tezos runs protocol %@, whose operations Spectra does not yet sign.",
            [hash.to_string()],
        )
    })
}

/// `value` as a Zarith natural: little-endian groups of seven bits, the
/// high bit set on every byte but the last.
fn zarith(value: u64, out: &mut Vec<u8>) {
    crate::send::leb128::write(value, out);
}

impl TezosLimits {
    fn write(&self, source: &[u8; 21], fee: u64, out: &mut Vec<u8>) {
        out.extend_from_slice(source);
        zarith(fee, out);
        zarith(self.counter, out);
        zarith(self.gas_limit, out);
        zarith(self.storage_limit, out);
    }
}

impl PreparedTezosTransfer {
    /// The implicit account the batch is from: only a key's account signs.
    fn source_address(&self) -> Result<TezosAddress, SendError> {
        match decode_address(&self.source)? {
            address @ TezosAddress::Implicit { .. } => Ok(address),
            TezosAddress::Originated(_) => Err(SendError::invalid(
                "A Tezos operation is signed from an implicit account",
            )),
        }
    }

    /// Every operation in the batch, the reveal first.
    fn operations(&self) -> impl Iterator<Item = &TezosLimits> {
        self.reveal.iter().chain(std::iter::once(&self.transaction))
    }

    fn total_gas(&self) -> Option<u64> {
        self.operations()
            .try_fold(0u64, |total, op| total.checked_add(op.gas_limit))
    }

    /// What the batch pays bakers, in mutez.
    pub(crate) fn total_fee(&self) -> u128 {
        self.operations().map(|op| u128::from(op.fee)).sum()
    }

    /// The most the batch can burn for the storage it adds, in mutez: its
    /// storage limits at the protocol's cost per byte.
    pub(crate) fn max_burn(&self, chain: Chain) -> Result<u128, SendError> {
        let protocol = known_protocol(chain.tezos_network()?, &self.protocol)?;
        Ok(self
            .operations()
            .map(|op| u128::from(op.storage_limit) * u128::from(protocol.cost_per_byte))
            .sum())
    }

    /// The most the sender's balance can fall by: the amount, the fees and
    /// the most it can burn.
    pub(crate) fn most_debited(&self, chain: Chain) -> Result<u128, SendError> {
        self.amount
            .checked_add(self.total_fee())
            .and_then(|total| total.checked_add(self.max_burn(chain).ok()?))
            .ok_or_else(|| SendError::invalid("Tezos amount overflows"))
    }

    /// The forged operation, with `public_key` in its reveal.
    pub(crate) fn forge(&self, public_key: &[u8; 32]) -> Result<Vec<u8>, SendError> {
        let branch = decode_base58check(&self.branch, &prefix::BLOCK, 32)
            .ok_or_else(|| SendError::invalid("Not a Tezos block hash"))?;
        let source = self
            .source_address()?
            .public_key_hash_bytes()
            .expect("an implicit account has a key hash");
        let destination = decode_address(&self.destination)?.contract_bytes();
        let amount = u64::try_from(self.amount)
            .ok()
            .filter(|amount| u128::from(*amount) <= MAX_MUTEZ)
            .ok_or_else(|| SendError::invalid("Tezos amount out of range"))?;
        let mut out = branch;
        if let Some(reveal) = &self.reveal {
            out.push(REVEAL_TAG);
            reveal.write(&source, reveal.fee, &mut out);
            out.push(ED25519_KEY_TAG);
            out.extend_from_slice(public_key);
            // No proof of possession: that is a BLS key's.
            out.push(0x00);
        }
        out.push(TRANSACTION_TAG);
        self.transaction
            .write(&source, self.transaction.fee, &mut out);
        zarith(amount, &mut out);
        out.extend_from_slice(&destination);
        // No parameters: the default entrypoint, with `Unit`.
        out.push(0x00);
        Ok(out)
    }

    /// How many bytes the signed operation is, the field the fee rule
    /// prices.
    fn signed_size(&self) -> Result<u64, SendError> {
        Ok(self.forge(&[0; 32])?.len() as u64 + SIGNATURE_BYTES)
    }

    /// This transfer with fees that meet `network`'s minimal fee for its
    /// gas limits and signed size, plus its margin. The reveal pays for its
    /// own gas and bytes; the transaction pays the rest.
    fn priced(mut self, network: &TezosNetwork) -> Result<Self, SendError> {
        let gas = self
            .total_gas()
            .ok_or_else(|| SendError::invalid("Tezos gas limit overflows"))?;
        // A fee's own Zarith length is in the size it pays for, so the
        // fee is raised until it covers the bytes it adds.
        for _ in 0..8 {
            let needed = network
                .minimal_fee(gas, self.signed_size()?)
                .and_then(|fee| fee.checked_add(network.fee_margin_mutez))
                .ok_or_else(|| SendError::invalid("Tezos fee overflows"))?;
            if self.total_fee() >= u128::from(needed) {
                return Ok(self);
            }
            if let Some(reveal) = &mut self.reveal {
                let mut bytes = vec![REVEAL_TAG];
                reveal.write(&[0; 21], needed, &mut bytes);
                // The public key and the proof's absence.
                let bytes = bytes.len() as u128 + 34;
                let nanotez = u128::from(reveal.gas_limit) * u128::from(network.nanotez_per_gas)
                    + bytes * u128::from(network.nanotez_per_byte);
                reveal.fee = u64::try_from(nanotez.div_ceil(1_000))
                    .unwrap_or(u64::MAX)
                    .min(needed);
            }
            self.transaction.fee = needed - self.reveal.map_or(0, |reveal| reveal.fee);
        }
        Err(SendError::invalid("Tezos fee did not settle"))
    }

    /// Refuse what the network would: a protocol `registry` does not list,
    /// another network, a branch that is no block hash, a sender that is not
    /// a tz1 account, a destination that is no Tezos account, a zero or
    /// oversized amount, counters out of order, limits beyond the
    /// protocol's, a reveal that names storage, and fees under the minimum.
    pub(crate) fn validate(&self, chain: Chain) -> Result<&'static TezosProtocol, SendError> {
        let network = chain.tezos_network()?;
        let protocol = known_protocol(network, &self.protocol)?;
        if self.chain_id != network.chain_id {
            return Err(SendError::invalid(
                "Tezos operation was prepared for another network",
            ));
        }
        if !matches!(
            self.source_address()?,
            TezosAddress::Implicit {
                curve: TezosCurve::Ed25519,
                ..
            }
        ) {
            return Err(SendError::invalid(
                "A Tezos transfer is signed from a tz1 account",
            ));
        }
        decode_address(&self.destination)?;
        if self.amount == 0 || self.amount > MAX_MUTEZ {
            return Err(SendError::invalid("Amount must be greater than zero"));
        }
        for op in self.operations() {
            if op.gas_limit == 0
                || op.gas_limit > protocol.hard_gas_limit_per_operation
                || op.storage_limit > protocol.hard_storage_limit_per_operation
                || u128::from(op.fee) > MAX_MUTEZ
            {
                return Err(SendError::invalid(
                    "Tezos operation limits are out of range",
                ));
            }
        }
        if let Some(reveal) = &self.reveal
            && (reveal.storage_limit != 0
                || reveal.counter.checked_add(1) != Some(self.transaction.counter))
        {
            return Err(SendError::invalid(
                "Tezos reveal does not lead its transfer",
            ));
        }
        let gas = self
            .total_gas()
            .filter(|gas| *gas <= protocol.hard_gas_limit_per_block)
            .ok_or_else(|| SendError::invalid("Tezos gas limits exceed a block's"))?;
        let minimal = network
            .minimal_fee(gas, self.signed_size()?)
            .ok_or_else(|| SendError::invalid("Tezos fee overflows"))?;
        if self.total_fee() < u128::from(minimal) {
            return Err(SendError::invalid(
                "Tezos fee is under the network's minimum",
            ));
        }
        Ok(protocol)
    }

    /// The batch as a node's simulation takes it, `public_key` in its
    /// reveal.
    pub(crate) fn contents(&self, public_key: &[u8; 32]) -> Vec<TezosContent> {
        let reveal = self.reveal.map(|reveal| TezosContent::Reveal {
            source: self.source.clone(),
            fee: reveal.fee,
            counter: reveal.counter,
            gas_limit: reveal.gas_limit,
            storage_limit: reveal.storage_limit,
            public_key: public_key_string(public_key),
        });
        let transaction = TezosContent::Transaction {
            source: self.source.clone(),
            fee: self.transaction.fee,
            counter: self.transaction.counter,
            gas_limit: self.transaction.gas_limit,
            storage_limit: self.transaction.storage_limit,
            amount: u64::try_from(self.amount).unwrap_or(u64::MAX),
            destination: self.destination.clone(),
        };
        reveal
            .into_iter()
            .chain(std::iter::once(transaction))
            .collect()
    }

    /// The signed operation a node injects, and its hash. Refused unless
    /// the transfer is one `chain` would take and `key` holds its sender.
    pub(crate) fn sign(
        &self,
        chain: Chain,
        key: &Ed25519Seed,
    ) -> Result<(Vec<u8>, String), SendError> {
        self.validate(chain)?;
        let public_key = key.public_key();
        if crate::derivation::tezos::address_from_public_key(&public_key) != self.source {
            return Err(SendError::invalid(
                "Tezos sender does not match the signing key",
            ));
        }
        let mut signed = self.forge(&public_key)?;
        let digest = blake2b::<32>(&[&[OPERATION_WATERMARK], &signed]);
        signed.extend_from_slice(&key.sign(&digest));
        let hash = base58check(&prefix::OPERATION, &blake2b::<32>(&[&signed]));
        Ok((signed, hash))
    }
}

#[cfg(test)]
#[path = "tests/tezos.rs"]
mod tests;

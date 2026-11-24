//! Algorand: a `pay` transaction of ALGO, encoded as canonical msgpack and
//! signed in Ed25519 over `"TX" ‖ msgpack`. Reads and submission are
//! `api::algod`.
//!
//! Canonical msgpack is go-algorand's encoding: a map whose keys are sorted
//! and whose zero values are left out, each integer in its shortest form.
//! The transaction id is the base32 of SHA-512/256 over the signed bytes,
//! and a signed transaction is the map `{sig, txn}`. The validity window
//! (`fv`..`lv`) gives a signed transaction its expiry; the genesis id and
//! hash (`gen`, `gh`) bind it to its network. A payment carries no note:
//! no exchange asks for one on ALGO deposits.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512_256};
use zeroize::Zeroizing;

use crate::registry::Chain;
use crate::send::error::SendError;
use crate::send::stages::units_u128;

/// What signing adds to a transaction's msgpack: the outer map's header,
/// the `sig` and `txn` keys and the 64-byte signature with its header.
/// algosdk prices a fee per byte over this size.
const SIGNATURE_OVERHEAD: u64 = 75;

/// A payment of ALGO, built and ready to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedAlgorandPayment {
    pub(crate) sender: String,
    pub(crate) receiver: String,
    /// MicroAlgos, at most `u64::MAX`; a string in the artifact, exact in
    /// every JSON reader.
    #[serde(with = "units_u128")]
    pub(crate) amount: u128,
    pub(crate) fee: u64,
    pub(crate) first_valid: u64,
    /// The last round the transaction may be committed in.
    pub(crate) last_valid: u64,
    pub(crate) genesis_id: String,
    #[serde(with = "hex::serde")]
    pub(crate) genesis_hash: [u8; 32],
}

/// The terms a payment is built from.
pub(crate) struct AlgorandPaymentTerms<'a> {
    pub sender: &'a str,
    pub receiver: &'a str,
    pub amount: u128,
    pub fee: u64,
    pub first_valid: u64,
    pub last_valid: u64,
}

/// A payment on `chain`, refused before anything is signed where the
/// network would refuse it: an address that is not Algorand's, a zero
/// amount or one past `u64`, no fee, and a validity window that is empty or
/// longer than the network's 1,000 rounds.
pub(crate) fn prepare_payment(
    chain: Chain,
    terms: AlgorandPaymentTerms<'_>,
) -> Result<PreparedAlgorandPayment, SendError> {
    let network = chain.algorand_network()?;
    let canonical = |address: &str| -> Result<String, SendError> {
        Ok(crate::derivation::algorand::address_from_public_key(
            &crate::derivation::algorand::decode_address(address)?,
        ))
    };
    if terms.amount == 0 {
        return Err(SendError::invalid("Amount must be greater than zero"));
    }
    if terms.amount > u128::from(u64::MAX) {
        return Err(SendError::invalid("Amount exceeds protocol range"));
    }
    if terms.fee == 0 {
        return Err(SendError::invalid("An Algorand transaction pays a fee"));
    }
    if terms.first_valid == 0
        || terms.last_valid < terms.first_valid
        || terms.last_valid - terms.first_valid > crate::registry::ALGORAND_MAX_TXN_LIFE
    {
        return Err(SendError::invalid(
            "An Algorand transaction is valid for one to 1,000 rounds",
        ));
    }
    Ok(PreparedAlgorandPayment {
        sender: canonical(terms.sender)?,
        receiver: canonical(terms.receiver)?,
        amount: terms.amount,
        fee: terms.fee,
        first_valid: terms.first_valid,
        last_valid: terms.last_valid,
        genesis_id: network.genesis_id.to_string(),
        genesis_hash: network.genesis_hash,
    })
}

/// The fee a payment of `terms` pays when the network asks `fee_per_byte`
/// and at least `min_fee`: the per-byte fee over the transaction's size once
/// signed, as algosdk prices it, never under the minimum. A payment's fee
/// field holds the per-byte fee while it is measured.
pub(crate) fn fee_for(
    chain: Chain,
    terms: AlgorandPaymentTerms<'_>,
    fee_per_byte: u64,
    min_fee: u64,
) -> Result<u64, SendError> {
    let measured = prepare_payment(
        chain,
        AlgorandPaymentTerms {
            fee: fee_per_byte.max(1),
            ..terms
        },
    )?;
    let size = PreparedAlgorandPayment {
        fee: fee_per_byte,
        ..measured
    }
    .transaction_bytes()
    .len() as u64
        + SIGNATURE_OVERHEAD;
    Ok(fee_per_byte
        .checked_mul(size)
        .ok_or_else(|| SendError::invalid("Algorand fee overflows"))?
        .max(min_fee))
}

// ── Canonical msgpack ────────────────────────────────────────────────────

fn uint(out: &mut Vec<u8>, value: u64) {
    match value {
        0..=0x7f => out.push(value as u8),
        0x80..=0xff => out.extend([0xcc, value as u8]),
        0x100..=0xffff => {
            out.push(0xcd);
            out.extend((value as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xce);
            out.extend((value as u32).to_be_bytes());
        }
        _ => {
            out.push(0xcf);
            out.extend(value.to_be_bytes());
        }
    }
}

/// A string of fewer than 256 bytes: every key and value written here.
fn text(out: &mut Vec<u8>, value: &str) {
    match value.len() {
        0..=31 => out.push(0xa0 | value.len() as u8),
        length => out.extend([0xd9, length as u8]),
    }
    out.extend(value.as_bytes());
}

/// Bytes, fewer than 256 of them: a key, a hash or a signature.
fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    out.extend([0xc4, value.len() as u8]);
    out.extend(value);
}

/// One map entry, of a value not at its zero.
enum Field<'a> {
    Uint(u64),
    Text(&'a str),
    Bytes(&'a [u8]),
}

impl Field<'_> {
    fn is_zero(&self) -> bool {
        match self {
            Self::Uint(value) => *value == 0,
            Self::Text(value) => value.is_empty(),
            Self::Bytes(value) => value.iter().all(|byte| *byte == 0),
        }
    }
}

/// A map of `fields`, whose keys are already in order, with the zero ones
/// left out.
fn map(out: &mut Vec<u8>, fields: &[(&str, Field<'_>)]) {
    let present: Vec<_> = fields
        .iter()
        .filter(|(_, value)| !value.is_zero())
        .collect();
    out.push(0x80 | present.len() as u8);
    for (key, value) in present {
        text(out, key);
        match value {
            Field::Uint(value) => uint(out, *value),
            Field::Text(value) => text(out, value),
            Field::Bytes(value) => bytes(out, value),
        }
    }
}

/// The 32-byte key an address the payment already holds names.
fn key_of(address: &str) -> Result<[u8; 32], SendError> {
    Ok(crate::derivation::algorand::decode_address(address)?)
}

impl PreparedAlgorandPayment {
    /// The transaction's canonical msgpack: `amt fee fv gen gh lv rcv snd
    /// type`, the order go-algorand sorts them in.
    pub(crate) fn transaction_bytes(&self) -> Vec<u8> {
        let receiver = key_of(&self.receiver).unwrap_or_default();
        let sender = key_of(&self.sender).unwrap_or_default();
        let mut out = Vec::with_capacity(160);
        map(
            &mut out,
            &[
                ("amt", Field::Uint(self.amount as u64)),
                ("fee", Field::Uint(self.fee)),
                ("fv", Field::Uint(self.first_valid)),
                ("gen", Field::Text(&self.genesis_id)),
                ("gh", Field::Bytes(&self.genesis_hash)),
                ("lv", Field::Uint(self.last_valid)),
                ("rcv", Field::Bytes(&receiver)),
                ("snd", Field::Bytes(&sender)),
                ("type", Field::Text("pay")),
            ],
        );
        out
    }

    /// What a signature covers: `"TX"` and the transaction's msgpack.
    pub(crate) fn bytes_to_sign(&self) -> Vec<u8> {
        let mut message = b"TX".to_vec();
        message.extend(self.transaction_bytes());
        message
    }

    /// The transaction id: base32, without padding, of SHA-512/256 over
    /// [`Self::bytes_to_sign`].
    pub(crate) fn txid(&self) -> String {
        data_encoding::BASE32_NOPAD.encode(&Sha512_256::digest(self.bytes_to_sign()))
    }

    /// The signed transaction's msgpack and its id. Refused unless the
    /// payment is bound to `chain`'s network and `key` holds its sender.
    pub(crate) fn sign(
        &self,
        chain: Chain,
        key: &AlgorandKey,
    ) -> Result<(Vec<u8>, String), SendError> {
        let network = chain.algorand_network()?;
        if self.genesis_id != network.genesis_id || self.genesis_hash != network.genesis_hash {
            return Err(SendError::invalid(
                "Algorand transaction was prepared for another network",
            ));
        }
        if self.amount == 0 || self.amount > u128::from(u64::MAX) {
            return Err(SendError::invalid("Amount must be greater than zero"));
        }
        let public = key.public_key()?;
        if crate::derivation::algorand::address_from_public_key(&public) != self.sender {
            return Err(SendError::invalid(
                "Algorand sender does not match the signing key",
            ));
        }
        key_of(&self.receiver)?;
        let signature = key.sign(&public, &self.bytes_to_sign())?;
        let mut signed = Vec::with_capacity(250);
        signed.push(0x82);
        text(&mut signed, "sig");
        bytes(&mut signed, &signature);
        text(&mut signed, "txn");
        signed.extend(self.transaction_bytes());
        Ok((signed, self.txid()))
    }
}

/// An Algorand account's signing key: a 25-word mnemonic's or a raw key's
/// 32-byte Ed25519 seed, or a BIP-39 account's ARC-52 extended key.
pub(crate) enum AlgorandKey {
    Seed(crate::send::keys::Ed25519Seed),
    Extended(Zeroizing<[u8; 64]>),
}

impl AlgorandKey {
    /// The key `bytes` hold, by their length.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, SendError> {
        match bytes.len() {
            32 => Ok(Self::Seed(crate::send::keys::Ed25519Seed::from_hex(
                &Zeroizing::new(hex::encode(bytes)),
            )?)),
            64 => {
                let mut key = Zeroizing::new([0u8; 64]);
                key.copy_from_slice(bytes);
                Ok(Self::Extended(key))
            }
            _ => Err(SendError::invalid(
                "An Algorand key is a 32-byte seed or a 64-byte extended key",
            )),
        }
    }

    pub(crate) fn public_key(&self) -> Result<[u8; 32], SendError> {
        Ok(match self {
            Self::Seed(seed) => seed.public_key(),
            Self::Extended(key) => crate::derivation::ed25519_extended::public_key(key)?,
        })
    }

    fn sign(&self, public: &[u8; 32], message: &[u8]) -> Result<[u8; 64], SendError> {
        Ok(match self {
            Self::Seed(seed) => seed.sign(message),
            Self::Extended(key) => crate::derivation::ed25519_extended::sign(key, public, message)?,
        })
    }
}

#[cfg(test)]
#[path = "tests/algorand.rs"]
mod tests;

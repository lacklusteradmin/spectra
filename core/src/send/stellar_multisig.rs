//! Stellar multisig: a payment of lumens an account's signers authorize
//! together.
//!
//! An account names up to twenty signers with weights from 1 to 255, and
//! low, medium and high thresholds; a payment is a medium-threshold
//! operation. The envelope carries every signature, each an ed25519
//! signature of the transaction hash with the signer key's last four bytes
//! as its hint; time bounds set the deadline and the sequence number is
//! fixed when the payment is built.
//!
//! An envelope read from elsewhere is decoded and encoded again as written
//! here: one that does not come out byte for byte the same carries
//! something no review here shows, and is refused.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::derivation::stellar::{address_from_public_key, decode_stellar_address};
use crate::send::error::SendError;
use crate::send::payment_memo::StellarMemo;

/// The most signers an account holds beside its master key.
pub(crate) const MAX_SIGNERS: usize = 20;

/// A payment of lumens, every field but the signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarPayment {
    pub source: [u8; 32],
    pub fee: u32,
    pub sequence: i64,
    pub min_time: u64,
    pub max_time: u64,
    pub memo: Option<OwnedMemo>,
    pub destination: [u8; 32],
    pub stroops: i64,
}

/// A memo a payment carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnedMemo {
    Text(String),
    Id(u64),
}

impl OwnedMemo {
    pub(crate) fn of(memo: Option<StellarMemo<'_>>) -> Option<Self> {
        memo.map(|memo| match memo {
            StellarMemo::Text(text) => Self::Text(text.to_string()),
            StellarMemo::Id(id) => Self::Id(id),
        })
    }

    pub(crate) fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Id(id) => id.to_string(),
        }
    }
}

/// One `DecoratedSignature`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarSignature {
    pub hint: [u8; 4],
    pub signature: Vec<u8>,
}

/// An account's signers and thresholds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StellarSigners {
    /// `(G… key, weight)` for every ed25519 signer, the master key among
    /// them while its weight is above zero.
    pub keys: Vec<(String, u64)>,
    /// Signers that are not keys (a hash or a pre-authorized transaction),
    /// by their Horizon key, with their weights: they count, but nothing
    /// here signs as them.
    pub other: Vec<(String, u64)>,
    pub low: u64,
    pub medium: u64,
    pub high: u64,
}

impl StellarSigners {
    pub(crate) fn weight_of(&self, address: &str) -> u64 {
        self.keys
            .iter()
            .filter(|(key, _)| key == address)
            .map(|(_, weight)| *weight)
            .sum()
    }

    /// The weight a payment needs: the medium threshold, and at least one
    /// signature's.
    pub(crate) fn payment_threshold(&self) -> u64 {
        self.medium.max(1)
    }
}

fn xdr_bytes(out: &mut Vec<u8>, data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(data);
    out.resize(out.len() + (4 - data.len() % 4) % 4, 0);
}

impl StellarPayment {
    /// The `Transaction` XDR.
    pub(crate) fn tx(&self) -> Result<Vec<u8>, SendError> {
        let mut tx = Vec::new();
        tx.extend(0u32.to_be_bytes()); // MuxedAccount KEY_TYPE_ED25519
        tx.extend(self.source);
        tx.extend(self.fee.to_be_bytes());
        tx.extend(self.sequence.to_be_bytes());
        tx.extend(1u32.to_be_bytes()); // PRECOND_TIME
        tx.extend(self.min_time.to_be_bytes());
        tx.extend(self.max_time.to_be_bytes());
        match &self.memo {
            None => tx.extend(0u32.to_be_bytes()),
            Some(OwnedMemo::Text(text)) => {
                if text.is_empty() || text.len() > 28 {
                    return Err(SendError::invalid("A text memo is 1 to 28 bytes of text."));
                }
                tx.extend(1u32.to_be_bytes());
                xdr_bytes(&mut tx, text.as_bytes());
            }
            Some(OwnedMemo::Id(id)) => {
                tx.extend(2u32.to_be_bytes());
                tx.extend(id.to_be_bytes());
            }
        }
        tx.extend(1u32.to_be_bytes()); // one operation
        tx.extend(0u32.to_be_bytes()); // no operation source
        tx.extend(1u32.to_be_bytes()); // PAYMENT
        tx.extend(0u32.to_be_bytes()); // destination: KEY_TYPE_ED25519
        tx.extend(self.destination);
        tx.extend(0u32.to_be_bytes()); // ASSET_TYPE_NATIVE
        if self.stroops <= 0 {
            return Err(SendError::invalid("A payment's amount is above zero."));
        }
        tx.extend(self.stroops.to_be_bytes());
        tx.extend(0u32.to_be_bytes()); // ext
        Ok(tx)
    }

    /// The hash each signer signs, on the network `passphrase` names.
    pub(crate) fn hash(&self, passphrase: &str) -> Result<[u8; 32], SendError> {
        let mut payload = Sha256::digest(passphrase.as_bytes()).to_vec();
        payload.extend(2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
        payload.extend(self.tx()?);
        Ok(Sha256::digest(&payload).into())
    }

    /// The envelope with `signatures`, as the network takes it.
    pub(crate) fn envelope(&self, signatures: &[StellarSignature]) -> Result<Vec<u8>, SendError> {
        if signatures.len() > MAX_SIGNERS {
            return Err(SendError::invalid(
                "An envelope carries at most 20 signatures.",
            ));
        }
        let mut envelope = 2u32.to_be_bytes().to_vec(); // ENVELOPE_TYPE_TX
        envelope.extend(self.tx()?);
        envelope.extend((signatures.len() as u32).to_be_bytes());
        for signature in signatures {
            envelope.extend(signature.hint);
            xdr_bytes(&mut envelope, &signature.signature);
        }
        Ok(envelope)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], SendError> {
        if length > self.bytes.len() {
            return Err(unread());
        }
        let (taken, rest) = self.bytes.split_at(length);
        self.bytes = rest;
        Ok(taken)
    }

    fn u32(&mut self) -> Result<u32, SendError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64, SendError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }

    fn key(&mut self) -> Result<[u8; 32], SendError> {
        if self.u32()? != 0 {
            return Err(unread());
        }
        Ok(self.take(32)?.try_into().expect("32 bytes"))
    }

    fn opaque(&mut self, limit: usize) -> Result<&'a [u8], SendError> {
        let length = self.u32()? as usize;
        if length > limit {
            return Err(unread());
        }
        let data = self.take(length)?;
        self.take((4 - length % 4) % 4)?;
        Ok(data)
    }
}

fn unread() -> SendError {
    SendError::invalid("The envelope is not a lumens payment as Spectra writes one.")
}

/// `envelope` as a payment and its signatures, refused unless encoding them
/// again gives `envelope` byte for byte.
pub(crate) fn decode(
    envelope: &[u8],
) -> Result<(StellarPayment, Vec<StellarSignature>), SendError> {
    let mut reader = Reader { bytes: envelope };
    if reader.u32()? != 2 {
        return Err(unread());
    }
    let source = reader.key()?;
    let fee = reader.u32()?;
    let sequence = reader.u64()? as i64;
    if reader.u32()? != 1 {
        return Err(unread());
    }
    let (min_time, max_time) = (reader.u64()?, reader.u64()?);
    let memo = match reader.u32()? {
        0 => None,
        1 => Some(OwnedMemo::Text(
            String::from_utf8(reader.opaque(28)?.to_vec()).map_err(|_| unread())?,
        )),
        2 => Some(OwnedMemo::Id(reader.u64()?)),
        _ => return Err(unread()),
    };
    if reader.u32()? != 1 || reader.u32()? != 0 || reader.u32()? != 1 {
        return Err(unread());
    }
    let destination = reader.key()?;
    if reader.u32()? != 0 {
        return Err(unread());
    }
    let stroops = reader.u64()? as i64;
    if reader.u32()? != 0 {
        return Err(unread());
    }
    let count = reader.u32()? as usize;
    if count > MAX_SIGNERS {
        return Err(unread());
    }
    let mut signatures = Vec::with_capacity(count);
    for _ in 0..count {
        let hint = reader.take(4)?.try_into().expect("4 bytes");
        let signature = reader.opaque(64)?.to_vec();
        signatures.push(StellarSignature { hint, signature });
    }
    if !reader.bytes.is_empty() {
        return Err(unread());
    }
    let payment = StellarPayment {
        source,
        fee,
        sequence,
        min_time,
        max_time,
        memo,
        destination,
        stroops,
    };
    if payment.envelope(&signatures)? != envelope {
        return Err(unread());
    }
    Ok((payment, signatures))
}

/// Sign `payment` with a signer's ed25519 seed.
pub(crate) fn sign(
    payment: &StellarPayment,
    passphrase: &str,
    seed: &[u8; 32],
) -> Result<StellarSignature, SendError> {
    let key = SigningKey::from_bytes(seed);
    let public = key.verifying_key().to_bytes();
    Ok(StellarSignature {
        hint: public[28..].try_into().expect("4 bytes"),
        signature: key.sign(&payment.hash(passphrase)?).to_bytes().to_vec(),
    })
}

/// The signer key whose signature `signature` is: one of `signers`' keys
/// whose last four bytes are its hint and which it verifies under.
fn signer_of(
    hash: &[u8; 32],
    signers: &StellarSigners,
    signature: &StellarSignature,
) -> Option<String> {
    let bytes: [u8; 64] = signature.signature.as_slice().try_into().ok()?;
    let signed = ed25519_dalek::Signature::from_bytes(&bytes);
    signers.keys.iter().find_map(|(address, _)| {
        let public = decode_stellar_address(address).ok()?;
        (public[28..] == signature.hint
            && VerifyingKey::from_bytes(&public)
                .ok()?
                .verify(hash, &signed)
                .is_ok())
        .then(|| address.clone())
    })
}

/// `signatures` judged against `signers`: each a valid signature by one of
/// its keys, no key twice. The signers, and their summed weight.
pub(crate) fn signed_weight(
    payment: &StellarPayment,
    passphrase: &str,
    signers: &StellarSigners,
    signatures: &[StellarSignature],
) -> Result<(Vec<String>, u64), SendError> {
    let hash = payment.hash(passphrase)?;
    let mut addresses: Vec<String> = Vec::new();
    for signature in signatures {
        let address = signer_of(&hash, signers, signature).ok_or_else(|| {
            SendError::invalid(
                "A signature in the envelope is not a valid one by any of the account's signers.",
            )
        })?;
        if addresses.contains(&address) {
            return Err(SendError::refused("%@ signed twice.", [address.as_str()]));
        }
        addresses.push(address);
    }
    let weight = addresses
        .iter()
        .map(|address| signers.weight_of(address))
        .sum();
    Ok((addresses, weight))
}

pub(crate) fn address_of(key: &[u8; 32]) -> String {
    address_from_public_key(key)
}

#[cfg(test)]
#[path = "tests/stellar_multisig.rs"]
mod tests;

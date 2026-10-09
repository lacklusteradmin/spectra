//! A Sui multisig account's transactions: a SUI transfer from the account's
//! address, each member's signature over its intent digest gathered as
//! data, verified against the member's key, and combined into one `MultiSig`
//! signature once the members' weights meet the threshold.
//!
//! A transaction read from elsewhere is decoded and encoded again as
//! `send::sui` writes a transfer: one that does not come out byte for byte
//! the same carries something no review here shows, and is refused.

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::derivation::sui_multisig::{MULTISIG_FLAG, SuiMultisig, SuiScheme};
use crate::send::error::SendError;
use crate::send::sui::{GasCoin, encode_transfer};

/// A SUI transfer, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuiTransfer {
    pub sender: [u8; 32],
    pub recipient: [u8; 32],
    pub amount: u64,
    /// The gas objects by id, version and digest; their balances are not
    /// part of the transaction.
    pub gas: Vec<([u8; 32], u64, [u8; 32])>,
    pub gas_price: u64,
    pub gas_budget: u64,
}

impl SuiTransfer {
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let coins: Vec<GasCoin> = self
            .gas
            .iter()
            .map(|(id, version, digest)| GasCoin {
                id: *id,
                version: *version,
                digest: *digest,
                balance: 0,
            })
            .collect();
        encode_transfer(
            &self.sender,
            &self.recipient,
            self.amount,
            &coins,
            self.gas_price,
            self.gas_budget,
        )
    }
}

fn blake2b(parts: &[&[u8]]) -> [u8; 32] {
    let mut state = blake2b_simd::Params::new().hash_length(32).to_state();
    for part in parts {
        state.update(part);
    }
    state.finalize().as_bytes().try_into().expect("32 bytes")
}

/// What each member signs: BLAKE2b-256 of the transaction intent and the
/// transaction.
pub(crate) fn intent_digest(bytes: &[u8]) -> [u8; 32] {
    blake2b(&[&[0, 0, 0], bytes])
}

/// The transaction's digest, as Sui names it.
pub(crate) fn transaction_digest(bytes: &[u8]) -> String {
    bs58::encode(blake2b(&[b"TransactionData::", bytes])).into_string()
}

fn unread() -> SendError {
    SendError::invalid("The transaction is not a SUI transfer as Spectra writes one.")
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

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], SendError> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }

    fn u64(&mut self) -> Result<u64, SendError> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }

    fn uleb(&mut self) -> Result<usize, SendError> {
        let mut value = 0usize;
        for shift in (0..35).step_by(7) {
            let byte = self.take(1)?[0];
            value |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(unread())
    }
}

/// `bytes` as a SUI transfer, refused unless encoding it again gives
/// `bytes` byte for byte.
pub(crate) fn decode(bytes: &[u8]) -> Result<SuiTransfer, SendError> {
    let mut reader = Reader { bytes };
    // The fixed head up to the amount.
    if reader.take(5)? != [0, 0, 2, 0, 8] {
        return Err(unread());
    }
    let amount = reader.u64()?;
    if reader.take(2)? != [0, 32] {
        return Err(unread());
    }
    let recipient = reader.fixed()?;
    if reader.take(17)? != [2, 2, 0, 1, 1, 0, 0, 1, 1, 3, 0, 0, 0, 0, 1, 1, 0] {
        return Err(unread());
    }
    let sender = reader.fixed()?;
    let count = reader.uleb()?;
    if count == 0 || count > 256 {
        return Err(unread());
    }
    let mut gas = Vec::with_capacity(count);
    for _ in 0..count {
        let id = reader.fixed()?;
        let version = reader.u64()?;
        if reader.take(1)? != [32] {
            return Err(unread());
        }
        gas.push((id, version, reader.fixed()?));
    }
    let _owner: [u8; 32] = reader.fixed()?;
    let gas_price = reader.u64()?;
    let gas_budget = reader.u64()?;
    let transfer = SuiTransfer {
        sender,
        recipient,
        amount,
        gas,
        gas_price,
        gas_budget,
    };
    if transfer.bytes() != bytes {
        return Err(unread());
    }
    Ok(transfer)
}

/// One member's signature, as Sui serializes it: flag, signature, key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberSignature {
    pub member: usize,
    pub signature: [u8; 64],
}

/// `serialized` (base64 flag ‖ signature ‖ key) as a valid signature of
/// `bytes` by one of `policy`'s members.
pub(crate) fn member_signature(
    policy: &SuiMultisig,
    bytes: &[u8],
    serialized: &str,
) -> Result<MemberSignature, SendError> {
    let refused =
        || SendError::invalid("A signature is not a valid one by any of the account's keys.");
    let raw = base64::engine::general_purpose::STANDARD
        .decode(serialized.trim())
        .map_err(|_| refused())?;
    let (&flag, rest) = raw.split_first().ok_or_else(refused)?;
    let scheme = SuiScheme::of_flag(flag).ok_or_else(refused)?;
    if rest.len() != 64 + scheme.key_length() {
        return Err(refused());
    }
    let (signature, key) = rest.split_at(64);
    let member = policy
        .members
        .iter()
        .position(|member| member.scheme == scheme && member.public_key == key)
        .ok_or_else(refused)?;
    let signature: [u8; 64] = signature.try_into().expect("64 bytes");
    if !verify(scheme, key, &intent_digest(bytes), &signature) {
        return Err(refused());
    }
    Ok(MemberSignature { member, signature })
}

/// Whether `signature` is `key`'s over `digest`: Ed25519 over the digest
/// itself, ECDSA (low-S) over its SHA-256 on either curve.
fn verify(scheme: SuiScheme, key: &[u8], digest: &[u8; 32], signature: &[u8; 64]) -> bool {
    match scheme {
        SuiScheme::Ed25519 => {
            let Ok(key) = <[u8; 32]>::try_from(key) else {
                return false;
            };
            ed25519_dalek::VerifyingKey::from_bytes(&key).is_ok_and(|key| {
                key.verify_strict(digest, &ed25519_dalek::Signature::from_bytes(signature))
                    .is_ok()
            })
        }
        SuiScheme::Secp256k1 => {
            let message = secp256k1::Message::from_digest(Sha256::digest(digest).into());
            let (Ok(key), Ok(signature)) = (
                secp256k1::PublicKey::from_slice(key),
                secp256k1::ecdsa::Signature::from_compact(signature),
            ) else {
                return false;
            };
            let mut normalized = signature;
            normalized.normalize_s();
            normalized == signature
                && secp256k1::Secp256k1::verification_only()
                    .verify_ecdsa(&message, &signature, &key)
                    .is_ok()
        }
        SuiScheme::Secp256r1 => {
            use p256::ecdsa::signature::Verifier;
            let (Ok(key), Ok(signature)) = (
                p256::ecdsa::VerifyingKey::from_sec1_bytes(key),
                p256::ecdsa::Signature::from_slice(signature),
            ) else {
                return false;
            };
            signature.normalize_s().is_none() && key.verify(digest, &signature).is_ok()
        }
    }
}

/// An Ed25519 member's signature of `bytes`, serialized as Sui writes one.
pub(crate) fn sign_ed25519(bytes: &[u8], seed: &crate::send::keys::Ed25519Seed) -> String {
    let mut serialized = vec![SuiScheme::Ed25519.flag()];
    serialized.extend(seed.sign(&intent_digest(bytes)));
    serialized.extend(seed.public_key());
    base64::engine::general_purpose::STANDARD.encode(serialized)
}

/// The members who signed and their summed weight, each member once.
pub(crate) fn signed_weight(
    policy: &SuiMultisig,
    signatures: &[MemberSignature],
) -> Result<(Vec<usize>, u64), SendError> {
    let mut members: Vec<usize> = signatures
        .iter()
        .map(|signature| signature.member)
        .collect();
    members.sort_unstable();
    if members.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SendError::invalid("A member signed twice."));
    }
    let weight = members
        .iter()
        .map(|member| u64::from(policy.members[*member].weight))
        .sum();
    Ok((members, weight))
}

/// The one signature the network takes: the multisig flag, then in BCS the
/// members' signatures in member order, the bitmap of who signed, and the
/// multisig public key.
pub(crate) fn combine(
    policy: &SuiMultisig,
    signatures: &[MemberSignature],
) -> Result<Vec<u8>, SendError> {
    let (members, weight) = signed_weight(policy, signatures)?;
    if weight < u64::from(policy.threshold) {
        return Err(SendError::invalid(
            "The members' weights do not yet meet the threshold.",
        ));
    }
    let mut ordered = signatures.to_vec();
    ordered.sort_by_key(|signature| signature.member);
    let mut out = vec![MULTISIG_FLAG, ordered.len() as u8];
    for signature in &ordered {
        out.push(policy.members[signature.member].scheme.flag());
        out.extend(signature.signature);
    }
    let bitmap: u16 = members.iter().map(|member| 1u16 << member).sum();
    out.extend(bitmap.to_le_bytes());
    out.extend(policy.bcs());
    Ok(out)
}

#[cfg(test)]
#[path = "tests/sui_multisig.rs"]
mod tests;

//! An Aptos MultiKey account's transactions: an APT transfer from the
//! account, each member's signature over its signing message gathered as
//! data and verified against the member's key, then one `MultiKey`
//! authenticator carrying the required signatures with a bitmap of who
//! signed.
//!
//! A transaction read from elsewhere is decoded and encoded again as
//! `send::aptos` writes an APT transfer: one that does not come out byte for
//! byte the same carries something no review here shows, and is refused.

use sha2::Digest as _;
use sha3::Sha3_256;

use crate::derivation::aptos_multikey::{AptosKey, AptosMultiKey};
use crate::send::error::SendError;

/// An account address, `0x` and up to 64 hex digits, as 32 bytes.
pub(crate) fn account_address(text: &str) -> Result<[u8; 32], SendError> {
    super::bcs::address(text)
}

/// The APT coin type every transfer here names.
pub(crate) const APT: &str = "0x1::aptos_coin::AptosCoin";

/// An APT transfer, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AptosTransfer {
    pub sender: [u8; 32],
    pub sequence: u64,
    pub recipient: [u8; 32],
    pub amount: u64,
    pub max_gas: u64,
    pub gas_price: u64,
    pub expiration: u64,
    pub chain_id: u8,
}

impl AptosTransfer {
    /// The signing message: SHA3-256 of `APTOS::RawTransaction`, then the
    /// raw transaction.
    pub(crate) fn message(&self) -> Result<Vec<u8>, SendError> {
        Ok(crate::send::aptos::prepare_token_transfer(
            &format!("0x{}", hex::encode(self.sender)),
            &format!("0x{}", hex::encode(self.recipient)),
            self.amount,
            self.sequence,
            self.gas_price,
            self.max_gas,
            self.expiration,
            self.chain_id,
            APT,
        )?
        .message)
    }

    /// The `RawTransaction` in BCS.
    pub(crate) fn raw(&self) -> Result<Vec<u8>, SendError> {
        Ok(self.message()?[32..].to_vec())
    }
}

fn unread() -> SendError {
    SendError::invalid("The transaction is not an APT transfer as Spectra writes one.")
}

/// `raw` as an APT transfer, refused unless encoding it again gives `raw`
/// byte for byte.
pub(crate) fn decode(raw: &[u8]) -> Result<AptosTransfer, SendError> {
    let take = |at: usize, length: usize| raw.get(at..at + length).ok_or_else(unread);
    let u64_at = |at: usize| -> Result<u64, SendError> {
        Ok(u64::from_le_bytes(
            take(at, 8)?.try_into().expect("8 bytes"),
        ))
    };
    let sender: [u8; 32] = take(0, 32)?.try_into().expect("32 bytes");
    let sequence = u64_at(32)?;
    // The entry function `0x1::coin::transfer<APT>` and its two arguments'
    // lengths are fixed; what follows them is the recipient and the amount.
    let head = take(40, 1 + 32 + 5 + 9 + 1 + 1 + 32 + 11 + 10 + 1 + 1 + 1)?;
    let recipient: [u8; 32] = take(40 + head.len(), 32)?.try_into().expect("32 bytes");
    let after = 40 + head.len() + 32;
    if take(after, 1)? != [8] {
        return Err(unread());
    }
    let amount = u64_at(after + 1)?;
    let transfer = AptosTransfer {
        sender,
        sequence,
        recipient,
        amount,
        max_gas: u64_at(after + 9)?,
        gas_price: u64_at(after + 17)?,
        expiration: u64_at(after + 25)?,
        chain_id: *take(after + 33, 1)?.first().expect("one byte"),
    };
    if transfer.raw()? != raw {
        return Err(unread());
    }
    Ok(transfer)
}

/// One member's signature, by its place among the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberSignature {
    pub member: usize,
    pub signature: [u8; 64],
}

/// Whether `signature` is `key`'s over `message`: Ed25519 over the message,
/// ECDSA (low-S) over its SHA3-256 for secp256k1.
pub(crate) fn verify(key: &AptosKey, message: &[u8], signature: &[u8; 64]) -> bool {
    match key {
        AptosKey::Ed25519(key) => ed25519_dalek::VerifyingKey::from_bytes(key).is_ok_and(|key| {
            key.verify_strict(message, &ed25519_dalek::Signature::from_bytes(signature))
                .is_ok()
        }),
        AptosKey::Secp256k1(key) => {
            let digest: [u8; 32] = Sha3_256::digest(message).into();
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
                    .verify_ecdsa(&secp256k1::Message::from_digest(digest), &signature, &key)
                    .is_ok()
        }
    }
}

/// `signature` (64 bytes) as member `member`'s valid signature of
/// `transfer`.
pub(crate) fn member_signature(
    policy: &AptosMultiKey,
    transfer: &AptosTransfer,
    member: usize,
    signature: &[u8],
) -> Result<MemberSignature, SendError> {
    let refused = || SendError::invalid("A signature is not a valid one by the key it names.");
    let key = policy.keys.get(member).ok_or_else(refused)?;
    let signature: [u8; 64] = signature.try_into().map_err(|_| refused())?;
    if !verify(key, &transfer.message()?, &signature) {
        return Err(refused());
    }
    Ok(MemberSignature { member, signature })
}

/// An Ed25519 member's signature of `transfer`.
pub(crate) fn sign_ed25519(
    transfer: &AptosTransfer,
    seed: &crate::send::keys::Ed25519Seed,
) -> Result<[u8; 64], SendError> {
    Ok(seed.sign(&transfer.message()?))
}

/// The members who signed, each once.
pub(crate) fn signers(signatures: &[MemberSignature]) -> Result<Vec<usize>, SendError> {
    let mut members: Vec<usize> = signatures
        .iter()
        .map(|signature| signature.member)
        .collect();
    members.sort_unstable();
    if members.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SendError::invalid("A member signed twice."));
    }
    Ok(members)
}

/// The signed transaction the network takes: the raw transaction, then a
/// single-sender authenticator holding the MultiKey, its signatures in
/// member order and the bitmap of who signed (member 0 the highest bit of
/// the first of four bytes).
pub(crate) fn signed_transaction(
    policy: &AptosMultiKey,
    transfer: &AptosTransfer,
    signatures: &[MemberSignature],
) -> Result<Vec<u8>, SendError> {
    let members = signers(signatures)?;
    if members.len() < usize::from(policy.required) {
        return Err(SendError::invalid(
            "The account does not yet carry the signatures it requires.",
        ));
    }
    let mut ordered = signatures.to_vec();
    ordered.sort_by_key(|signature| signature.member);
    let mut out = transfer.raw()?;
    out.push(4); // TransactionAuthenticator::SingleSender
    out.push(3); // AccountAuthenticator::MultiKey
    out.extend(policy.bcs());
    out.push(ordered.len() as u8);
    for signature in &ordered {
        out.push(match policy.keys[signature.member] {
            AptosKey::Ed25519(_) => 0,
            AptosKey::Secp256k1(_) => 1,
        });
        out.push(64);
        out.extend(signature.signature);
    }
    let mut bitmap = [0u8; 4];
    for member in &members {
        bitmap[member / 8] |= 0x80 >> (member % 8);
    }
    out.push(4);
    out.extend(bitmap);
    Ok(out)
}

/// The committed transaction's hash: SHA3-256 of SHA3-256 of
/// `APTOS::Transaction`, the user-transaction variant and the signed
/// transaction.
pub(crate) fn transaction_hash(signed: &[u8]) -> String {
    let prefix = Sha3_256::digest(b"APTOS::Transaction");
    let digest = Sha3_256::new()
        .chain_update(prefix)
        .chain_update([0])
        .chain_update(signed)
        .finalize();
    format!("0x{}", hex::encode(digest))
}

#[cfg(test)]
#[path = "tests/aptos_multikey.rs"]
mod tests;

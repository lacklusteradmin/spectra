//! XRP Ledger multi-signing: a Payment of XRP an account's signer list
//! authorizes.
//!
//! Every field is fixed before the first signature, so what a signer
//! reviews is the transaction itself. Each signer signs the transaction's
//! signing fields under the `SMT\0` prefix followed by its own account id,
//! with its account's master key; the signatures go in `Signers`, sorted by
//! account id, and the transaction carries an empty `SigningPubKey`. The fee
//! is the base fee times one more than the signatures it may carry, and
//! `LastLedgerSequence` bounds how long signatures can be gathered.
//!
//! A blob read from elsewhere is decoded and encoded again as written here:
//! one that does not come out byte for byte the same carries something no
//! review here shows, and is refused. A signature made with a signer's
//! regular key cannot be checked without reading that account, so only a
//! signer's master key is accepted.

use secp256k1::{Message, PublicKey, Secp256k1, SecretKey, ecdsa::Signature};
use serde::{Deserialize, Serialize};

use crate::derivation::xrp::{account_id_of_key, address_of_account_id, decode_xrp_address};
use crate::send::error::SendError;
use crate::send::xrp::{push_vl, sha512_half};

/// `lsfDisableMaster`: the account's master key no longer signs.
pub(crate) const LSF_DISABLE_MASTER: u32 = 0x0010_0000;
/// The most entries a signer list holds.
pub(crate) const MAX_SIGNERS: usize = 32;

/// A Payment of XRP, every field but the signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpPayment {
    pub account: [u8; 20],
    pub destination: [u8; 20],
    pub destination_tag: Option<u32>,
    pub amount_drops: u64,
    pub fee_drops: u64,
    pub sequence: u32,
    pub last_ledger_sequence: u32,
}

/// One signer's entry in `Signers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpSigner {
    pub account: [u8; 20],
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

/// An account's signer list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XrpSignerList {
    pub quorum: u64,
    /// `(classic address, weight)`, as the ledger lists them.
    pub entries: Vec<(String, u64)>,
}

impl XrpSignerList {
    pub(crate) fn weight_of(&self, address: &str) -> u64 {
        self.entries
            .iter()
            .filter(|(entry, _)| entry == address)
            .map(|(_, weight)| *weight)
            .sum()
    }
}

fn drops(out: &mut Vec<u8>, header: u8, value: u64) -> Result<(), SendError> {
    if value > 100_000_000_000_000_000 {
        return Err(SendError::invalid(
            "An XRP amount is past the ledger's limit.",
        ));
    }
    out.push(header);
    out.extend((0x4000_0000_0000_0000 | value).to_be_bytes());
    Ok(())
}

impl XrpPayment {
    /// The transaction's fields in canonical order; `signers` adds the
    /// `Signers` array, which signing leaves out.
    fn fields(&self, signers: Option<&[XrpSigner]>) -> Result<Vec<u8>, SendError> {
        let mut out = vec![0x12, 0x00, 0x00]; // TransactionType: Payment
        out.push(0x22); // Flags
        out.extend(0u32.to_be_bytes());
        out.push(0x24); // Sequence
        out.extend(self.sequence.to_be_bytes());
        if let Some(tag) = self.destination_tag {
            out.push(0x2e); // DestinationTag
            out.extend(tag.to_be_bytes());
        }
        out.extend([0x20, 0x1b]); // LastLedgerSequence
        out.extend(self.last_ledger_sequence.to_be_bytes());
        drops(&mut out, 0x61, self.amount_drops)?; // Amount
        drops(&mut out, 0x68, self.fee_drops)?; // Fee
        out.extend([0x73, 0x00]); // SigningPubKey: empty, as a multi-signed one has
        out.push(0x81); // Account
        push_vl(&mut out, &self.account);
        out.push(0x83); // Destination
        push_vl(&mut out, &self.destination);
        if let Some(signers) = signers.filter(|signers| !signers.is_empty()) {
            out.push(0xf3); // Signers
            for signer in signers {
                out.extend([0xe0, 0x10]); // Signer
                out.push(0x73);
                push_vl(&mut out, &signer.public_key);
                out.push(0x74);
                push_vl(&mut out, &signer.signature);
                out.push(0x81);
                push_vl(&mut out, &signer.account);
                out.push(0xe1);
            }
            out.push(0xf1);
        }
        Ok(out)
    }

    /// What `signer` signs: `SMT\0`, the signing fields, its account id.
    pub(crate) fn multisigning_data(&self, signer: &[u8; 20]) -> Result<Vec<u8>, SendError> {
        let mut data = b"SMT\0".to_vec();
        data.extend(self.fields(None)?);
        data.extend(signer);
        Ok(data)
    }

    /// The transaction with `signers`, sorted by account id, as the ledger
    /// takes it.
    pub(crate) fn blob(&self, signers: &[XrpSigner]) -> Result<Vec<u8>, SendError> {
        let mut signers = signers.to_vec();
        signers.sort_by_key(|signer| signer.account);
        self.fields(Some(&signers))
    }

    /// The transaction's id once `blob` is what the ledger holds:
    /// SHA-512Half of `TXN\0` and the blob.
    pub(crate) fn hash(blob: &[u8]) -> [u8; 32] {
        let mut data = b"TXN\0".to_vec();
        data.extend(blob);
        sha512_half(&data)
    }

    /// What signing commits to, the same for every signer: SHA-512Half of
    /// the signing fields.
    pub(crate) fn digest(&self) -> Result<[u8; 32], SendError> {
        let mut data = b"SMT\0".to_vec();
        data.extend(self.fields(None)?);
        Ok(sha512_half(&data))
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

    fn vl(&mut self) -> Result<&'a [u8], SendError> {
        let first = self.take(1)?[0] as usize;
        let length = match first {
            0..=192 => first,
            193..=240 => 193 + (first - 193) * 256 + self.take(1)?[0] as usize,
            _ => return Err(unread()),
        };
        self.take(length)
    }

    fn u32(&mut self) -> Result<u32, SendError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn drops(&mut self) -> Result<u64, SendError> {
        let value = u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes"));
        if value & 0xc000_0000_0000_0000 != 0x4000_0000_0000_0000 {
            return Err(unread());
        }
        Ok(value & !0x4000_0000_0000_0000)
    }

    fn account(&mut self) -> Result<[u8; 20], SendError> {
        self.vl()?.try_into().map_err(|_| unread())
    }

    fn expect(&mut self, bytes: &[u8]) -> Result<(), SendError> {
        (self.take(bytes.len())? == bytes)
            .then_some(())
            .ok_or_else(unread)
    }

    fn next_is(&self, bytes: &[u8]) -> bool {
        self.bytes.starts_with(bytes)
    }
}

fn unread() -> SendError {
    SendError::invalid("The transaction is not an XRP payment as Spectra writes one.")
}

/// `blob` as a multi-signed XRP payment and its signers, refused unless
/// encoding them again gives `blob` byte for byte.
pub(crate) fn decode(blob: &[u8]) -> Result<(XrpPayment, Vec<XrpSigner>), SendError> {
    let mut reader = Reader { bytes: blob };
    reader.expect(&[0x12, 0x00, 0x00, 0x22, 0, 0, 0, 0, 0x24])?;
    let sequence = reader.u32()?;
    let destination_tag = if reader.next_is(&[0x2e]) {
        reader.take(1)?;
        Some(reader.u32()?)
    } else {
        None
    };
    reader.expect(&[0x20, 0x1b])?;
    let last_ledger_sequence = reader.u32()?;
    reader.expect(&[0x61])?;
    let amount_drops = reader.drops()?;
    reader.expect(&[0x68])?;
    let fee_drops = reader.drops()?;
    reader.expect(&[0x73, 0x00, 0x81])?;
    let account = reader.account()?;
    reader.expect(&[0x83])?;
    let destination = reader.account()?;
    let mut signers = Vec::new();
    if reader.next_is(&[0xf3]) {
        reader.take(1)?;
        while reader.next_is(&[0xe0, 0x10]) {
            reader.take(2)?;
            reader.expect(&[0x73])?;
            let public_key = reader.vl()?.to_vec();
            reader.expect(&[0x74])?;
            let signature = reader.vl()?.to_vec();
            reader.expect(&[0x81])?;
            let account = reader.account()?;
            reader.expect(&[0xe1])?;
            signers.push(XrpSigner {
                account,
                public_key,
                signature,
            });
        }
        reader.expect(&[0xf1])?;
    }
    if !reader.bytes.is_empty() {
        return Err(unread());
    }
    let payment = XrpPayment {
        account,
        destination,
        destination_tag,
        amount_drops,
        fee_drops,
        sequence,
        last_ledger_sequence,
    };
    if payment.blob(&signers)? != blob {
        return Err(unread());
    }
    Ok((payment, signers))
}

/// Sign `payment` as `signer`'s account with its master key.
pub(crate) fn sign(payment: &XrpPayment, private_key: &[u8]) -> Result<XrpSigner, SendError> {
    let secp = Secp256k1::new();
    let secret =
        SecretKey::from_slice(private_key).map_err(|_| SendError::invalid("invalid XRP key"))?;
    let public = PublicKey::from_secret_key(&secp, &secret);
    let account = account_id_of_key(&public);
    let digest = sha512_half(&payment.multisigning_data(&account)?);
    let signature = secp.sign_ecdsa(&Message::from_digest(digest), &secret);
    Ok(XrpSigner {
        account,
        public_key: public.serialize().to_vec(),
        signature: signature.serialize_der().to_vec(),
    })
}

/// Whether `signer` is a valid signature of `payment` by its account's
/// master key: the key is the account's own, and the signature verifies,
/// low-S, over the account's signing data.
pub(crate) fn verify(payment: &XrpPayment, signer: &XrpSigner) -> bool {
    let Ok(key) = PublicKey::from_slice(&signer.public_key) else {
        return false;
    };
    if account_id_of_key(&key) != signer.account {
        return false;
    }
    let Ok(signature) = Signature::from_der(&signer.signature) else {
        return false;
    };
    let mut normalized = signature;
    normalized.normalize_s();
    if normalized != signature {
        return false;
    }
    let Ok(data) = payment.multisigning_data(&signer.account) else {
        return false;
    };
    Secp256k1::verification_only()
        .verify_ecdsa(&Message::from_digest(sha512_half(&data)), &signature, &key)
        .is_ok()
}

/// `signers` judged against `list`: each a valid master-key signature by a
/// listed account, none twice. The signers' addresses and summed weight.
pub(crate) fn signed_weight(
    payment: &XrpPayment,
    list: &XrpSignerList,
    signers: &[XrpSigner],
) -> Result<(Vec<String>, u64), SendError> {
    let mut addresses: Vec<String> = Vec::new();
    for signer in signers {
        let address = address_of_account_id(&signer.account)?;
        if list.weight_of(&address) == 0 {
            return Err(SendError::refused(
                "%@ signed, but is not on the account's signer list.",
                [address.as_str()],
            ));
        }
        if !verify(payment, signer) {
            return Err(SendError::refused(
                "%@'s signature is not a valid one by its master key.",
                [address.as_str()],
            ));
        }
        if addresses.contains(&address) {
            return Err(SendError::refused("%@ signed twice.", [address.as_str()]));
        }
        addresses.push(address);
    }
    let weight = addresses
        .iter()
        .map(|address| list.weight_of(address))
        .sum();
    Ok((addresses, weight))
}

/// A classic address's account id.
pub(crate) fn account_id(address: &str) -> Result<[u8; 20], SendError> {
    decode_xrp_address(address)?
        .try_into()
        .map_err(|_| SendError::invalid("Not an XRP address."))
}

#[cfg(test)]
#[path = "tests/xrp_multisig.rs"]
mod tests;

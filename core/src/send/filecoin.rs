//! Filecoin: a message paying FIL from a secp256k1 account, serialized as
//! the chain's DAG-CBOR and signed over its CID. Reads and submission are
//! `api::lotus_json_rpc`, which takes the signed message in Lotus's JSON.
//!
//! A message carries no expiry and names no network: its nonce alone keeps
//! it from landing twice, and the node it is built through is proven to be
//! mainnet before it is signed.

use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};

use crate::api::lotus_json_rpc::{FilecoinGas, FilecoinMessage, FilecoinSignedMessage};
use crate::derivation::filecoin::{FilecoinAddress, FilecoinProtocol, decode_address};
use crate::registry::Chain;
use crate::send::error::SendError;
use crate::send::stages::units_u128;

/// A bare value transfer: the recipient's code does not run.
pub(crate) const METHOD_SEND: u64 = 0;

/// FRC-42's `InvokeEVM`: an EVM contract runs its receive or fallback code,
/// and an Ethereum-style account or a placeholder takes the value.
pub(crate) const METHOD_INVOKE_EVM: u64 = 3_844_450_837;

/// The most a message may pay for gas, in attoFIL: Lotus's default
/// `MaxFee`, 0.07 FIL. An estimate is asked to stay within it and refused
/// past it.
pub(crate) const MAX_FEE: u128 = 70_000_000_000_000_000;

/// The block gas limit: no message uses more.
const BLOCK_GAS_LIMIT: u64 = 10_000_000_000;

/// The method a transfer of FIL to `recipient` calls. A key's account takes
/// a bare send. An Ethereum-style recipient — an `f410f` address, or an ID
/// or actor address whose actor has one — takes `InvokeEVM`, so a contract
/// runs its code instead of being credited FIL it cannot account for, as
/// Filecoin's documentation advises and Lotus does for an `0x` recipient.
/// Any other actor takes a bare send.
pub(crate) fn transfer_method(recipient: FilecoinProtocol, ethereum_actor: bool) -> u64 {
    match recipient {
        FilecoinProtocol::Delegated => METHOD_INVOKE_EVM,
        FilecoinProtocol::Id | FilecoinProtocol::Actor if ethereum_actor => METHOD_INVOKE_EVM,
        _ => METHOD_SEND,
    }
}

/// A transfer of FIL, built and ready to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedFilecoinMessage {
    pub(crate) sender: String,
    pub(crate) recipient: String,
    pub(crate) nonce: u64,
    /// What the recipient receives, in attoFIL.
    #[serde(with = "units_u128")]
    pub(crate) value: u128,
    pub(crate) method: u64,
    pub(crate) gas_limit: u64,
    #[serde(with = "units_u128")]
    pub(crate) gas_fee_cap: u128,
    #[serde(with = "units_u128")]
    pub(crate) gas_premium: u128,
}

/// The terms a transfer is built from.
pub(crate) struct FilecoinTransferTerms<'a> {
    pub sender: &'a str,
    pub recipient: &'a str,
    pub value: u128,
    pub method: u64,
    pub nonce: u64,
    pub gas: FilecoinGas,
}

/// A transfer on `chain`, refused before anything is signed where the
/// network would refuse it or where it would not do what a transfer means:
/// a sender that is not a secp256k1 account, a recipient that is not an
/// address here, a method the recipient does not take a transfer with, no
/// value, and gas the network would refuse or that costs more than
/// [`MAX_FEE`].
pub(crate) fn prepare_transfer(
    chain: Chain,
    terms: FilecoinTransferTerms<'_>,
) -> Result<PreparedFilecoinMessage, SendError> {
    if decode_address(chain, terms.sender)?.protocol() != FilecoinProtocol::Secp256k1 {
        return Err(SendError::invalid(
            "A Filecoin transfer is signed from an f1 account",
        ));
    }
    let recipient = decode_address(chain, terms.recipient)?;
    let methods: &[u64] = match recipient.protocol() {
        FilecoinProtocol::Secp256k1 | FilecoinProtocol::Bls => &[METHOD_SEND],
        FilecoinProtocol::Delegated => &[METHOD_INVOKE_EVM],
        FilecoinProtocol::Id | FilecoinProtocol::Actor => &[METHOD_SEND, METHOD_INVOKE_EVM],
    };
    if !methods.contains(&terms.method) {
        return Err(SendError::invalid(
            "The recipient does not take a transfer with this method",
        ));
    }
    if terms.value == 0 {
        return Err(SendError::invalid("Amount must be greater than zero"));
    }
    let gas = terms.gas;
    if gas.limit == 0
        || gas.limit > BLOCK_GAS_LIMIT
        || gas.fee_cap == 0
        || gas.premium > gas.fee_cap
    {
        return Err(SendError::invalid("Filecoin gas estimate is out of range"));
    }
    if u128::from(gas.limit)
        .checked_mul(gas.fee_cap)
        .is_none_or(|fee| fee > MAX_FEE)
    {
        return Err(SendError::invalid(
            "Filecoin gas estimate costs more than the 0.07 FIL a message may pay",
        ));
    }
    Ok(PreparedFilecoinMessage {
        sender: terms.sender.to_string(),
        recipient: terms.recipient.to_string(),
        nonce: terms.nonce,
        value: terms.value,
        method: terms.method,
        gas_limit: gas.limit,
        gas_fee_cap: gas.fee_cap,
        gas_premium: gas.premium,
    })
}

/// The unsigned message a node estimates gas for: `value` from `sender` to
/// `recipient` with `method`, at `nonce`, with no gas set.
pub(crate) fn estimate_message(
    sender: &str,
    recipient: &str,
    value: u128,
    method: u64,
    nonce: u64,
) -> FilecoinMessage {
    FilecoinMessage {
        to: recipient.to_string(),
        from: sender.to_string(),
        nonce,
        value: value.to_string(),
        gas_limit: 0,
        gas_fee_cap: "0".into(),
        gas_premium: "0".into(),
        method,
    }
}

/// A token amount as the chain's big integer bytes: none for zero,
/// otherwise a positive sign byte and the magnitude, big-endian.
fn big_int(value: u128) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }
    let magnitude = value.to_be_bytes();
    let first = magnitude.iter().position(|byte| *byte != 0).unwrap_or(15);
    std::iter::once(0)
        .chain(magnitude[first..].iter().copied())
        .collect()
}

fn cbor(value: &ciborium::Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).expect("a CBOR value writes to memory");
    out
}

impl PreparedFilecoinMessage {
    /// The most the message can cost: every unit of gas it may use at its
    /// fee cap, and its value.
    pub(crate) fn maximum_cost(&self) -> Option<u128> {
        self.maximum_fee()?.checked_add(self.value)
    }

    /// The most its gas can cost: its limit at its fee cap.
    pub(crate) fn maximum_fee(&self) -> Option<u128> {
        u128::from(self.gas_limit).checked_mul(self.gas_fee_cap)
    }

    /// The message as the chain serializes it: version, recipient, sender,
    /// nonce, value, gas limit, fee cap, premium, method and no parameters.
    fn cbor_value(&self, chain: Chain) -> Result<ciborium::Value, SendError> {
        use ciborium::Value;
        let address =
            |text: &str| -> Result<FilecoinAddress, SendError> { Ok(decode_address(chain, text)?) };
        Ok(Value::Array(vec![
            Value::Integer(0.into()),
            Value::Bytes(address(&self.recipient)?.bytes().to_vec()),
            Value::Bytes(address(&self.sender)?.bytes().to_vec()),
            Value::Integer(self.nonce.into()),
            Value::Bytes(big_int(self.value)),
            Value::Integer(self.gas_limit.into()),
            Value::Bytes(big_int(self.gas_fee_cap)),
            Value::Bytes(big_int(self.gas_premium)),
            Value::Integer(self.method.into()),
            Value::Bytes(Vec::new()),
        ]))
    }

    /// The message's bytes as the chain serializes it.
    pub(crate) fn message_bytes(&self, chain: Chain) -> Result<Vec<u8>, SendError> {
        Ok(cbor(&self.cbor_value(chain)?))
    }

    /// The signed message, as Lotus takes it, and its CID: what the chain
    /// finds it by. The signature is secp256k1's, recoverable, over
    /// BLAKE2b-256 of the message's CID bytes. Refused unless `key` holds the
    /// prepared sender on `chain`.
    pub(crate) fn sign(
        &self,
        chain: Chain,
        key: &SecretKey,
    ) -> Result<(FilecoinSignedMessage, String), SendError> {
        let public = PublicKey::from_secret_key(&Secp256k1::new(), key);
        if crate::derivation::filecoin::address_from_public_key(chain, &public)? != self.sender {
            return Err(SendError::invalid(
                "Filecoin sender does not match the signing key",
            ));
        }
        let (cid, _) = crate::derivation::filecoin::message_cid(&self.message_bytes(chain)?);
        let digest = crate::derivation::primitives::blake2b::<32>(&[&cid]);
        let (recovery, compact) = Secp256k1::signing_only()
            .sign_ecdsa_recoverable(&Message::from_digest(digest), key)
            .serialize_compact();
        let mut signature = compact.to_vec();
        signature.push(recovery.to_i32() as u8);
        let signed_cid = self.signed_cid(chain, &signature)?;
        Ok((
            FilecoinSignedMessage {
                message: FilecoinMessage {
                    to: self.recipient.clone(),
                    from: self.sender.clone(),
                    nonce: self.nonce,
                    value: self.value.to_string(),
                    gas_limit: self.gas_limit,
                    gas_fee_cap: self.gas_fee_cap.to_string(),
                    gas_premium: self.gas_premium.to_string(),
                    method: self.method,
                },
                signature,
            },
            signed_cid,
        ))
    }

    /// The CID of the message signed with `signature`, 65 recoverable
    /// secp256k1 bytes: that of the signed message, the message and the
    /// signature's type byte before its bytes.
    pub(crate) fn signed_cid(&self, chain: Chain, signature: &[u8]) -> Result<String, SendError> {
        let mut signature_bytes = vec![1];
        signature_bytes.extend_from_slice(signature);
        let signed = ciborium::Value::Array(vec![
            self.cbor_value(chain)?,
            ciborium::Value::Bytes(signature_bytes),
        ]);
        Ok(crate::derivation::filecoin::message_cid(&cbor(&signed)).1)
    }
}

#[cfg(test)]
#[path = "tests/filecoin.rs"]
mod tests;

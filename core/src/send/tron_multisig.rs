//! Tron account permissions and transactions several keys sign.
//!
//! A Tron account's owner permission (id 0) and its active permissions (id
//! 2 and up) each name up to five keys with weights and a threshold their
//! summed weights must meet; an active permission covers only the contract
//! types its `operations` bits name. A transaction names the permission it
//! is signed under (`Permission_id`, absent for the owner's), and each of
//! that permission's keys signs the transaction id, the SHA-256 of its
//! `raw_data`.
//!
//! A transaction read from elsewhere is decoded from its `raw_data` bytes
//! and encoded again as `send::tron` writes it: one that does not come out
//! byte for byte the same carries something no review here shows, and is
//! refused. The JSON a node reads is the one written from those bytes, never
//! the JSON that came with them.

use secp256k1::{Message, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::send::error::SendError;
use crate::send::tron::{PreparedTronTransfer, RawReference, Transfer};

/// `TransferContract`, `TransferAssetContract` and `TriggerSmartContract`.
pub(crate) const TRANSFER_CONTRACT: u64 = 1;
pub(crate) const TRANSFER_ASSET_CONTRACT: u64 = 2;
pub(crate) const TRIGGER_SMART_CONTRACT: u64 = 31;
/// The longest a Tron transaction may wait between its timestamp and its
/// expiration: java-tron refuses one that expires more than a day later.
pub(crate) const MAX_LIFETIME_MS: u64 = 24 * 60 * 60 * 1000;
/// The keys one permission may name.
const MAX_KEYS: usize = 5;

/// One key of a permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TronKey {
    /// Base58, `T…`.
    pub address: String,
    pub weight: u64,
}

/// One of an account's permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TronPermission {
    /// 0 for the owner's, 2 and up for an active one.
    pub id: u8,
    pub name: String,
    pub threshold: u64,
    /// The contract types an active permission covers, as java-tron's
    /// 32-byte bit set: type `n` is bit `n % 8` of byte `n / 8`. `None` for
    /// the owner's, which covers every contract.
    pub operations: Option<String>,
    pub keys: Vec<TronKey>,
}

impl TronPermission {
    /// Whether the permission authorizes a contract of `contract_type`.
    pub(crate) fn covers(&self, contract_type: u64) -> bool {
        let Some(operations) = &self.operations else {
            return true;
        };
        let Ok(bits) = hex::decode(operations) else {
            return false;
        };
        bits.get(contract_type as usize / 8)
            .is_some_and(|byte| byte & (1 << (contract_type % 8)) != 0)
    }

    /// The weight `address` carries here, 0 when it is none of the keys.
    pub(crate) fn weight_of(&self, address: &str) -> u64 {
        self.keys
            .iter()
            .filter(|key| key.address == address)
            .map(|key| key.weight)
            .sum()
    }

    fn check(&self) -> Result<(), SendError> {
        let total: u64 = self.keys.iter().map(|key| key.weight).sum();
        if self.keys.is_empty()
            || self.keys.len() > MAX_KEYS
            || self.threshold == 0
            || total < self.threshold
        {
            return Err(SendError::invalid(
                "The account reports a permission its keys cannot meet.",
            ));
        }
        Ok(())
    }
}

/// An account's owner and active permissions, as `wallet/getaccount`
/// reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TronPermissions {
    pub owner: TronPermission,
    pub actives: Vec<TronPermission>,
}

impl TronPermissions {
    /// An account that never changed its permissions: its own key alone,
    /// under the owner's and the default active permission alike. What an
    /// account not yet on the network has.
    pub(crate) fn single(address: &str) -> Self {
        let key = vec![TronKey {
            address: address.to_string(),
            weight: 1,
        }];
        Self {
            owner: TronPermission {
                id: 0,
                name: "owner".into(),
                threshold: 1,
                operations: None,
                keys: key.clone(),
            },
            actives: Vec::new(),
        }
    }

    pub(crate) fn checked(self) -> Result<Self, SendError> {
        self.owner.check()?;
        for active in &self.actives {
            active.check()?;
            if active.id < 2 {
                return Err(SendError::invalid(
                    "The account reports an active permission with an owner's id.",
                ));
            }
        }
        Ok(self)
    }

    pub(crate) fn all(&self) -> impl Iterator<Item = &TronPermission> {
        std::iter::once(&self.owner).chain(&self.actives)
    }

    pub(crate) fn by_id(&self, id: u8) -> Option<&TronPermission> {
        self.all().find(|permission| permission.id == id)
    }

    /// The permission `address`'s key alone signs `contract_type` under:
    /// the owner's where its weight meets that threshold, else the first
    /// active permission covering the contract that it meets alone.
    pub(crate) fn alone(&self, address: &str, contract_type: u64) -> Option<u8> {
        self.all()
            .find(|permission| {
                permission.covers(contract_type)
                    && permission.weight_of(address) >= permission.threshold
            })
            .map(|permission| permission.id)
    }

    /// The permission a spend of `contract_type` gathers signatures under:
    /// the first active permission that covers it, else the owner's.
    pub(crate) fn for_contract(&self, contract_type: u64) -> &TronPermission {
        self.actives
            .iter()
            .find(|permission| permission.covers(contract_type))
            .unwrap_or(&self.owner)
    }
}

/// What a transaction pays, decoded from its contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TronPayment {
    Trx {
        to: [u8; 21],
        amount: u64,
    },
    Trc10 {
        asset_id: String,
        to: [u8; 21],
        amount: u64,
    },
    Trc20 {
        contract: [u8; 21],
        to: [u8; 21],
        amount: u128,
        fee_limit: u64,
    },
}

impl TronPayment {
    pub(crate) fn contract_type(&self) -> u64 {
        match self {
            Self::Trx { .. } => TRANSFER_CONTRACT,
            Self::Trc10 { .. } => TRANSFER_ASSET_CONTRACT,
            Self::Trc20 { .. } => TRIGGER_SMART_CONTRACT,
        }
    }
}

/// A transaction decoded from its `raw_data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedTronTransaction {
    pub owner: [u8; 21],
    pub payment: TronPayment,
    pub reference: RawReference,
}

impl DecodedTronTransaction {
    pub(crate) fn owner_address(&self) -> String {
        base58(&self.owner)
    }
}

pub(crate) fn base58(address: &[u8; 21]) -> String {
    bs58::encode(address).with_check().into_string()
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Result<u64, SendError> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = self
                .bytes
                .split_first()
                .ok_or_else(|| SendError::invalid("The transaction's bytes end early."))?;
            self.bytes = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(SendError::invalid(
            "The transaction holds an overlong number.",
        ))
    }

    /// The next field: its number, and its value as a number (wire type 0)
    /// or bytes (wire type 2). Any other wire type is refused.
    fn field(&mut self) -> Result<Option<(u64, Field<'a>)>, SendError> {
        if self.bytes.is_empty() {
            return Ok(None);
        }
        let tag = self.varint()?;
        let value = match tag & 7 {
            0 => Field::Number(self.varint()?),
            2 => {
                let length = usize::try_from(self.varint()?).map_err(SendError::invalid)?;
                if length > self.bytes.len() {
                    return Err(SendError::invalid("The transaction's bytes end early."));
                }
                let (value, rest) = self.bytes.split_at(length);
                self.bytes = rest;
                Field::Bytes(value)
            }
            _ => {
                return Err(SendError::invalid(
                    "The transaction holds a field no review reads.",
                ));
            }
        };
        Ok(Some((tag >> 3, value)))
    }
}

enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}

fn unread() -> SendError {
    SendError::invalid(
        "The transaction is not a TRX, TRC-10 or TRC-20 transfer as Spectra writes one.",
    )
}

fn address_field(bytes: &[u8]) -> Result<[u8; 21], SendError> {
    let address: [u8; 21] = bytes.try_into().map_err(|_| unread())?;
    if address[0] != 0x41 {
        return Err(unread());
    }
    Ok(address)
}

/// `raw` as a transfer, refused unless encoding the decoded fields again
/// gives `raw` byte for byte.
pub(crate) fn decode(raw: &[u8]) -> Result<DecodedTronTransaction, SendError> {
    let mut reader = Reader { bytes: raw };
    let (mut ref_bytes, mut ref_hash, mut expiration, mut timestamp) = (None, None, None, None);
    let (mut contract, mut fee_limit) = (None, 0u64);
    while let Some((number, value)) = reader.field()? {
        match (number, value) {
            (1, Field::Bytes(bytes)) => {
                ref_bytes = Some(<[u8; 2]>::try_from(bytes).map_err(|_| unread())?)
            }
            (4, Field::Bytes(bytes)) => {
                ref_hash = Some(<[u8; 8]>::try_from(bytes).map_err(|_| unread())?)
            }
            (8, Field::Number(value)) => expiration = Some(value),
            (11, Field::Bytes(bytes)) if contract.is_none() => contract = Some(bytes),
            (14, Field::Number(value)) => timestamp = Some(value),
            (18, Field::Number(value)) => fee_limit = value,
            _ => return Err(unread()),
        }
    }
    let contract = contract.ok_or_else(unread)?;
    let mut reader = Reader { bytes: contract };
    let (mut kind, mut parameter, mut permission_id) = (None, None, 0u64);
    while let Some((number, value)) = reader.field()? {
        match (number, value) {
            (1, Field::Number(value)) => kind = Some(value),
            (2, Field::Bytes(bytes)) => parameter = Some(bytes),
            (5, Field::Number(value)) => permission_id = value,
            _ => return Err(unread()),
        }
    }
    let mut reader = Reader {
        bytes: parameter.ok_or_else(unread)?,
    };
    let (mut type_url, mut value) = (None, None);
    while let Some((number, field)) = reader.field()? {
        match (number, field) {
            (1, Field::Bytes(bytes)) => type_url = Some(bytes),
            (2, Field::Bytes(bytes)) => value = Some(bytes),
            _ => return Err(unread()),
        }
    }
    let (kind, value) = (kind.ok_or_else(unread)?, value.ok_or_else(unread)?);
    let _ = type_url.ok_or_else(unread)?;
    let mut fields: Vec<(u64, Field)> = Vec::new();
    let mut reader = Reader { bytes: value };
    while let Some(field) = reader.field()? {
        fields.push(field);
    }
    let bytes_of = |wanted: u64| {
        fields.iter().find_map(|(number, field)| match field {
            Field::Bytes(bytes) if *number == wanted => Some(*bytes),
            _ => None,
        })
    };
    let number_of = |wanted: u64| {
        fields.iter().find_map(|(number, field)| match field {
            Field::Number(value) if *number == wanted => Some(*value),
            _ => None,
        })
    };
    let (owner, payment) = match kind {
        TRANSFER_CONTRACT => (
            address_field(bytes_of(1).ok_or_else(unread)?)?,
            TronPayment::Trx {
                to: address_field(bytes_of(2).ok_or_else(unread)?)?,
                amount: number_of(3).ok_or_else(unread)?,
            },
        ),
        TRANSFER_ASSET_CONTRACT => (
            address_field(bytes_of(2).ok_or_else(unread)?)?,
            TronPayment::Trc10 {
                asset_id: String::from_utf8(bytes_of(1).ok_or_else(unread)?.to_vec())
                    .map_err(|_| unread())?,
                to: address_field(bytes_of(3).ok_or_else(unread)?)?,
                amount: number_of(4).ok_or_else(unread)?,
            },
        ),
        TRIGGER_SMART_CONTRACT => {
            let data = bytes_of(4).ok_or_else(unread)?;
            if data.len() != 68
                || data[..4] != [0xa9, 0x05, 0x9c, 0xbb]
                || data[4..16] != [0; 12]
                || data[36..52] != [0; 16]
            {
                return Err(unread());
            }
            let mut to = [0x41; 21];
            to[1..].copy_from_slice(&data[16..36]);
            (
                address_field(bytes_of(1).ok_or_else(unread)?)?,
                TronPayment::Trc20 {
                    contract: address_field(bytes_of(2).ok_or_else(unread)?)?,
                    to,
                    amount: u128::from_be_bytes(data[52..68].try_into().expect("16 bytes")),
                    fee_limit,
                },
            )
        }
        _ => return Err(unread()),
    };
    let decoded = DecodedTronTransaction {
        owner,
        payment,
        reference: RawReference {
            ref_bytes: ref_bytes.ok_or_else(unread)?,
            ref_hash: ref_hash.ok_or_else(unread)?,
            expiration: expiration.ok_or_else(unread)?,
            timestamp: timestamp.ok_or_else(unread)?,
            permission_id: u8::try_from(permission_id).map_err(|_| unread())?,
        },
    };
    if decoded.prepared()?.raw != raw {
        return Err(unread());
    }
    Ok(decoded)
}

impl DecodedTronTransaction {
    /// The transaction as `send::tron` writes it: its bytes and the JSON a
    /// node reads.
    pub(crate) fn prepared(&self) -> Result<PreparedTronTransfer, SendError> {
        let owner = base58(&self.owner);
        let (to, asset, contract);
        let transfer = match &self.payment {
            TronPayment::Trx {
                to: recipient,
                amount,
            } => {
                to = base58(recipient);
                Transfer::Native {
                    to: &to,
                    amount: *amount,
                }
            }
            TronPayment::Trc10 {
                asset_id,
                to: recipient,
                amount,
            } => {
                to = base58(recipient);
                asset = asset_id.clone();
                Transfer::Trc10 {
                    asset_id: &asset,
                    to: &to,
                    amount: *amount,
                }
            }
            TronPayment::Trc20 {
                contract: token,
                to: recipient,
                amount,
                fee_limit,
            } => {
                to = base58(recipient);
                contract = base58(token);
                Transfer::Token {
                    contract: &contract,
                    to: &to,
                    amount: *amount,
                    fee_limit: *fee_limit,
                }
            }
        };
        crate::send::tron::encode(&owner, transfer, &self.reference)
    }
}

/// The transaction id: SHA-256 of `raw`.
pub(crate) fn transaction_id(raw: &[u8]) -> [u8; 32] {
    Sha256::digest(raw).into()
}

/// A key's signature of `raw`, as TronWeb writes one: `r ‖ s ‖ v`, `v` 27
/// or 28.
pub(crate) fn sign(raw: &[u8], key: &[u8]) -> Result<[u8; 65], SendError> {
    let secret = SecretKey::from_slice(key).map_err(|_| SendError::invalid("invalid Tron key"))?;
    let (recovery, compact) = Secp256k1::signing_only()
        .sign_ecdsa_recoverable(&Message::from_digest(transaction_id(raw)), &secret)
        .serialize_compact();
    let mut signature = [0; 65];
    signature[..64].copy_from_slice(&compact);
    signature[64] = 27 + recovery.to_i32() as u8;
    Ok(signature)
}

/// The account whose key made `signature` of `raw`.
pub(crate) fn signer(raw: &[u8], signature: &[u8]) -> Result<String, SendError> {
    use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
    let refused = || SendError::invalid("A signature in the transaction is not a valid one.");
    if signature.len() != 65 {
        return Err(refused());
    }
    let recovery = match signature[64] {
        v @ 27..=28 => v - 27,
        v @ 0..=1 => v,
        _ => return Err(refused()),
    };
    let signature = RecoverableSignature::from_compact(
        &signature[..64],
        RecoveryId::from_i32(i32::from(recovery)).map_err(|_| refused())?,
    )
    .map_err(|_| refused())?;
    let key = Secp256k1::verification_only()
        .recover_ecdsa(&Message::from_digest(transaction_id(raw)), &signature)
        .map_err(|_| refused())?;
    Ok(crate::derivation::tron::address_from_public_key(&key))
}

/// A transaction's signatures judged against `permission`: each a valid
/// signature by one of its keys, none twice. The signers, and their summed
/// weight.
pub(crate) fn signed_weight(
    raw: &[u8],
    permission: &TronPermission,
    signatures: &[Vec<u8>],
) -> Result<(Vec<String>, u64), SendError> {
    let mut signers: Vec<String> = Vec::new();
    for signature in signatures {
        let signer = signer(raw, signature)?;
        if permission.weight_of(&signer) == 0 {
            return Err(SendError::refused(
                "%@ signed, but holds no weight in the permission the transaction names.",
                [signer.as_str()],
            ));
        }
        if signers.contains(&signer) {
            return Err(SendError::refused("%@ signed twice.", [signer.as_str()]));
        }
        signers.push(signer);
    }
    let weight = signers
        .iter()
        .map(|signer| permission.weight_of(signer))
        .sum();
    Ok((signers, weight))
}

#[cfg(test)]
#[path = "tests/tron_multisig.rs"]
mod tests;

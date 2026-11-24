//! Hedera: a `CryptoTransfer` of HBAR with its memo, encoded as the
//! network's protobuf and signed once per consensus node. Reads go through
//! `api::hedera_mirror`, submission through `api::hedera_hapi`.
//!
//! A transaction body names the one node it is submitted to, so a transfer
//! is signed for a few nodes, each body under the same transaction id: the
//! network runs the first to reach consensus and refuses the rest as
//! duplicates, so a busy node hands over to the next without a new review.
//! The prepared transfer holds the terms a review binds and the nodes; the
//! transaction id is the payer and a valid start set at signing, which adds
//! no term and starts the 120 seconds the signed bodies stay valid.

use prost::Message as _;
use serde::{Deserialize, Serialize};

use crate::derivation::hedera::{HederaAccountId, HederaKeyScheme, HederaPrivateKey};
use crate::registry::Chain;
use crate::send::error::SendError;
use crate::send::stages::units_u128;

/// The network's protobuf messages, field for field. Each scalar the
/// Hedera SDKs write even at zero (shard and realm, `scheduled`,
/// `is_approval`, an empty memo) is an explicit-presence field here, so
/// the bytes are the ones the SDK signs.
pub(crate) mod proto {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Timestamp {
        #[prost(int64, optional, tag = "1")]
        pub seconds: Option<i64>,
        #[prost(int32, optional, tag = "2")]
        pub nanos: Option<i32>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Duration {
        #[prost(int64, optional, tag = "1")]
        pub seconds: Option<i64>,
    }

    /// `AccountID` by number; its `alias` arm is not used.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AccountId {
        #[prost(int64, optional, tag = "1")]
        pub shard_num: Option<i64>,
        #[prost(int64, optional, tag = "2")]
        pub realm_num: Option<i64>,
        #[prost(int64, optional, tag = "3")]
        pub account_num: Option<i64>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TransactionId {
        #[prost(message, optional, tag = "1")]
        pub transaction_valid_start: Option<Timestamp>,
        #[prost(message, optional, tag = "2")]
        pub account_id: Option<AccountId>,
        #[prost(bool, optional, tag = "3")]
        pub scheduled: Option<bool>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AccountAmount {
        #[prost(message, optional, tag = "1")]
        pub account_id: Option<AccountId>,
        #[prost(sint64, optional, tag = "2")]
        pub amount: Option<i64>,
        #[prost(bool, optional, tag = "3")]
        pub is_approval: Option<bool>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TransferList {
        #[prost(message, repeated, tag = "1")]
        pub account_amounts: Vec<AccountAmount>,
    }

    /// `CryptoTransferTransactionBody` of HBAR alone: no token transfers.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CryptoTransferTransactionBody {
        #[prost(message, optional, tag = "1")]
        pub transfers: Option<TransferList>,
    }

    /// `TransactionBody` with its `cryptoTransfer` arm.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TransactionBody {
        #[prost(message, optional, tag = "1")]
        pub transaction_id: Option<TransactionId>,
        #[prost(message, optional, tag = "2")]
        pub node_account_id: Option<AccountId>,
        #[prost(uint64, optional, tag = "3")]
        pub transaction_fee: Option<u64>,
        #[prost(message, optional, tag = "4")]
        pub transaction_valid_duration: Option<Duration>,
        #[prost(string, optional, tag = "6")]
        pub memo: Option<String>,
        #[prost(message, optional, tag = "14")]
        pub crypto_transfer: Option<CryptoTransferTransactionBody>,
    }

    /// A `SignaturePair`: the signer's whole public key as its prefix, and an
    /// Ed25519 or ECDSA secp256k1 signature.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SignaturePair {
        #[prost(bytes = "vec", tag = "1")]
        pub pub_key_prefix: Vec<u8>,
        #[prost(bytes = "vec", optional, tag = "3")]
        pub ed25519: Option<Vec<u8>>,
        #[prost(bytes = "vec", optional, tag = "6")]
        pub ecdsa_secp256k1: Option<Vec<u8>>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SignatureMap {
        #[prost(message, repeated, tag = "1")]
        pub sig_pair: Vec<SignaturePair>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SignedTransaction {
        #[prost(bytes = "vec", tag = "1")]
        pub body_bytes: Vec<u8>,
        #[prost(message, optional, tag = "2")]
        pub sig_map: Option<SignatureMap>,
    }

    /// `Transaction`, carrying its `signedTransactionBytes`: what a node's
    /// `cryptoTransfer` takes.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Transaction {
        #[prost(bytes = "vec", tag = "5")]
        pub signed_transaction_bytes: Vec<u8>,
    }

    /// The SDK's `TransactionList`: one `Transaction` per node, the form
    /// its `Transaction.fromBytes` reads.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TransactionList {
        #[prost(message, repeated, tag = "1")]
        pub transaction_list: Vec<Transaction>,
    }
}

/// A transfer of HBAR, built and ready to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedHederaTransfer {
    /// The account that sends and pays the fee, `0.0.N`.
    pub(crate) payer: String,
    /// The payer's key when the transfer was built, in DER hex: the account
    /// holds it alone, and only it signs.
    pub(crate) public_key: String,
    pub(crate) recipient: String,
    /// Tinybars.
    #[serde(with = "units_u128")]
    pub(crate) amount: u128,
    /// Empty when the transfer carries none.
    pub(crate) memo: String,
    /// The `transactionFee`: the most the network may charge, in tinybars.
    #[serde(with = "units_u128")]
    pub(crate) max_fee: u128,
    /// The consensus nodes a body is signed for, in the order they are tried.
    pub(crate) nodes: Vec<String>,
}

/// The terms a Hedera transfer is built from.
pub(crate) struct HederaTransferTerms<'a> {
    pub payer: &'a str,
    pub public_key: &'a str,
    pub recipient: &'a str,
    pub amount: u128,
    pub memo: &'a str,
    pub max_fee: u128,
    pub nodes: &'a [HederaAccountId],
}

/// A transfer signed for each of its nodes, under one transaction id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SignedHederaTransfer {
    /// `0.0.N-seconds-nanos`, as mirror nodes and explorers write it.
    pub transaction_id: String,
    /// The SDK's `TransactionList` of every node's `Transaction`.
    pub transaction_list: Vec<u8>,
}

/// An account id as `0.0.N` and the protobuf of it.
fn account(chain: Chain, text: &str) -> Result<(HederaAccountId, proto::AccountId), SendError> {
    let id = HederaAccountId::parse(chain, text)?;
    let number = |value: u64| {
        i64::try_from(value).map_err(|_| SendError::invalid("Hedera account number out of range"))
    };
    Ok((
        id,
        proto::AccountId {
            shard_num: Some(number(id.shard)?),
            realm_num: Some(number(id.realm)?),
            account_num: Some(number(id.num)?),
        },
    ))
}

/// A transfer of HBAR on `chain`, checked for what the network would refuse
/// before any of it is signed: accounts that are not this network's, a
/// payment to itself, an amount of zero or past the network's range, a memo
/// over its limit or with a zero byte, no fee bound and no node.
pub(crate) fn prepare_transfer(
    chain: Chain,
    terms: HederaTransferTerms<'_>,
) -> Result<PreparedHederaTransfer, SendError> {
    chain.hedera_network()?;
    let (payer, _) = account(chain, terms.payer)?;
    let (recipient, _) = account(chain, terms.recipient)?;
    if payer == recipient {
        return Err(SendError::invalid(
            "A Hedera transfer cannot pay the account that sends it",
        ));
    }
    if crate::derivation::hedera::HederaPublicKey::from_der_hex(terms.public_key).is_none() {
        return Err(SendError::invalid("Invalid Hedera public key"));
    }
    if terms.amount == 0 || i64::try_from(terms.amount).is_err() {
        return Err(SendError::invalid(
            "Amount must be greater than zero and within the network's range",
        ));
    }
    if terms.max_fee == 0 || u64::try_from(terms.max_fee).is_err() {
        return Err(SendError::invalid("A Hedera transaction needs a fee bound"));
    }
    if chain
        .memo_text_limit()
        .is_none_or(|limit| terms.memo.len() > limit)
        || terms.memo.contains('\0')
    {
        return Err(SendError::invalid("Memo is too long for this network"));
    }
    let mut nodes: Vec<HederaAccountId> = Vec::new();
    for node in terms.nodes {
        if !nodes.contains(node) {
            nodes.push(*node);
        }
    }
    if nodes.is_empty() {
        return Err(SendError::invalid("A Hedera transaction names a node"));
    }
    Ok(PreparedHederaTransfer {
        payer: payer.to_string(),
        public_key: terms.public_key.to_ascii_lowercase(),
        recipient: recipient.to_string(),
        amount: terms.amount,
        memo: terms.memo.to_string(),
        max_fee: terms.max_fee,
        nodes: nodes.iter().map(ToString::to_string).collect(),
    })
}

/// A transaction id as mirror nodes write it: `0.0.N-seconds-nanos`.
pub(crate) fn transaction_id(payer: &str, seconds: i64, nanos: i32) -> String {
    format!("{payer}-{seconds}-{nanos:09}")
}

impl PreparedHederaTransfer {
    /// The body submitted to `node`: the transfer, its fee bound, memo and
    /// validity, under the transaction id of `payer` at `valid_start`.
    /// HBAR transfers are listed by account, as the SDK lists them.
    pub(crate) fn body_bytes(
        &self,
        chain: Chain,
        node: &str,
        valid_start: (i64, i32),
    ) -> Result<Vec<u8>, SendError> {
        let (payer, payer_id) = account(chain, &self.payer)?;
        let (recipient, recipient_id) = account(chain, &self.recipient)?;
        let (_, node_id) = account(chain, node)?;
        let amount = i64::try_from(self.amount)
            .map_err(|_| SendError::invalid("Amount out of the network's range"))?;
        let mut transfers = [
            (payer, payer_id.clone(), -amount),
            (recipient, recipient_id, amount),
        ];
        transfers.sort_by_key(|(id, _, _)| *id);
        let (seconds, nanos) = valid_start;
        if !(0..1_000_000_000).contains(&nanos) || seconds <= 0 {
            return Err(SendError::invalid("Invalid Hedera valid start"));
        }
        Ok(proto::TransactionBody {
            transaction_id: Some(proto::TransactionId {
                transaction_valid_start: Some(proto::Timestamp {
                    seconds: Some(seconds),
                    nanos: Some(nanos),
                }),
                account_id: Some(payer_id),
                scheduled: Some(false),
            }),
            node_account_id: Some(node_id),
            transaction_fee: Some(
                u64::try_from(self.max_fee)
                    .map_err(|_| SendError::invalid("Fee bound out of range"))?,
            ),
            transaction_valid_duration: Some(proto::Duration {
                seconds: Some(crate::registry::HEDERA_TRANSACTION_VALID_SECS),
            }),
            memo: Some(self.memo.clone()),
            crypto_transfer: Some(proto::CryptoTransferTransactionBody {
                transfers: Some(proto::TransferList {
                    account_amounts: transfers
                        .into_iter()
                        .map(|(_, id, amount)| proto::AccountAmount {
                            account_id: Some(id),
                            amount: Some(amount),
                            is_approval: Some(false),
                        })
                        .collect(),
                }),
            }),
        }
        .encode_to_vec())
    }

    /// The transfer signed for each node under the transaction id of the
    /// payer at `valid_start`. Refused unless `key` is the key the transfer
    /// was built for.
    pub(crate) fn sign(
        &self,
        chain: Chain,
        key: &HederaPrivateKey,
        valid_start: (i64, i32),
    ) -> Result<SignedHederaTransfer, SendError> {
        chain.hedera_network()?;
        let public = key.public_key();
        if public.der_hex() != self.public_key {
            return Err(SendError::invalid(
                "Hedera payer's key does not match the signing key",
            ));
        }
        let mut list = proto::TransactionList::default();
        for node in &self.nodes {
            let body_bytes = self.body_bytes(chain, node, valid_start)?;
            let signature = key.sign(&body_bytes);
            let pair = match public.scheme {
                HederaKeyScheme::Ed25519 => proto::SignaturePair {
                    pub_key_prefix: public.bytes.clone(),
                    ed25519: Some(signature),
                    ecdsa_secp256k1: None,
                },
                HederaKeyScheme::EcdsaSecp256k1 => proto::SignaturePair {
                    pub_key_prefix: public.bytes.clone(),
                    ed25519: None,
                    ecdsa_secp256k1: Some(signature),
                },
            };
            list.transaction_list.push(proto::Transaction {
                signed_transaction_bytes: proto::SignedTransaction {
                    body_bytes,
                    sig_map: Some(proto::SignatureMap {
                        sig_pair: vec![pair],
                    }),
                }
                .encode_to_vec(),
            });
        }
        Ok(SignedHederaTransfer {
            transaction_id: transaction_id(&self.payer, valid_start.0, valid_start.1),
            transaction_list: list.encode_to_vec(),
        })
    }
}

/// One node's signed transaction from a `TransactionList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeTransaction {
    pub node: HederaAccountId,
    /// The `Transaction` bytes a node's `cryptoTransfer` takes.
    pub transaction: Vec<u8>,
    /// The id the body carries, as [`transaction_id`] writes it.
    pub transaction_id: String,
}

/// The node transactions a signed `TransactionList` holds, each with the
/// node its body names. Refused unless every body is a transfer under one
/// transaction id and no node is named twice.
pub(crate) fn node_transactions(list: &[u8]) -> Result<Vec<NodeTransaction>, SendError> {
    let invalid = || SendError::invalid("Not a signed Hedera transfer");
    let list = proto::TransactionList::decode(list).map_err(|_| invalid())?;
    let mut nodes: Vec<NodeTransaction> = Vec::new();
    for transaction in list.transaction_list {
        let signed =
            proto::SignedTransaction::decode(transaction.signed_transaction_bytes.as_slice())
                .map_err(|_| invalid())?;
        let body =
            proto::TransactionBody::decode(signed.body_bytes.as_slice()).map_err(|_| invalid())?;
        let id = |account: &Option<proto::AccountId>| -> Result<HederaAccountId, SendError> {
            let account = account.as_ref().ok_or_else(invalid)?;
            let part =
                |value: Option<i64>| u64::try_from(value.unwrap_or(0)).map_err(|_| invalid());
            Ok(HederaAccountId {
                shard: part(account.shard_num)?,
                realm: part(account.realm_num)?,
                num: part(account.account_num)?,
            })
        };
        let transaction_id = body.transaction_id.as_ref().ok_or_else(invalid)?;
        let start = transaction_id
            .transaction_valid_start
            .as_ref()
            .ok_or_else(invalid)?;
        let node = NodeTransaction {
            node: id(&body.node_account_id)?,
            transaction: transaction.encode_to_vec(),
            transaction_id: self::transaction_id(
                &id(&transaction_id.account_id)?.to_string(),
                start.seconds.unwrap_or(0),
                start.nanos.unwrap_or(0),
            ),
        };
        if body.crypto_transfer.is_none()
            || nodes
                .iter()
                .any(|other| other.node == node.node || other.transaction_id != node.transaction_id)
        {
            return Err(invalid());
        }
        nodes.push(node);
    }
    if nodes.is_empty() {
        return Err(invalid());
    }
    Ok(nodes)
}

#[cfg(test)]
#[path = "tests/hedera.rs"]
mod tests;

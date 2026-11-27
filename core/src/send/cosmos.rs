//! Cosmos SDK: a bank `MsgSend` of the network's own coin with its memo,
//! encoded as the SDK's protobuf and signed in `SIGN_MODE_DIRECT`. Reads and
//! submission are `api::cosmos_rest`.
//!
//! The prepared transfer holds the terms a review binds; the bytes are a
//! function of them and of the signing key's public key, which the
//! transaction's `AuthInfo` names. A wallet stores no public key, so the
//! `AuthInfo` is written when the key is at hand, at signing: it adds no
//! term, only the key whose address the sender already is.

use prost::Message as _;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::registry::{Chain, CosmosKey};
use crate::send::error::SendError;
use crate::send::stages::units_u128;

/// The SDK's protobuf messages a transfer is written in, field for field.
/// prost leaves out a field at its default, as the SDK's own encoder and
/// CosmJS do, so the bytes are the ones a sign document is checked against.
pub(crate) mod proto {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Coin {
        #[prost(string, tag = "1")]
        pub denom: String,
        #[prost(string, tag = "2")]
        pub amount: String,
    }

    /// `cosmos.bank.v1beta1.MsgSend`.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MsgSend {
        #[prost(string, tag = "1")]
        pub from_address: String,
        #[prost(string, tag = "2")]
        pub to_address: String,
        #[prost(message, repeated, tag = "3")]
        pub amount: Vec<Coin>,
    }

    /// `google.protobuf.Any`.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Any {
        #[prost(string, tag = "1")]
        pub type_url: String,
        #[prost(bytes = "vec", tag = "2")]
        pub value: Vec<u8>,
    }

    /// `cosmos.tx.v1beta1.TxBody`, without the extension options a plain
    /// transfer has none of.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TxBody {
        #[prost(message, repeated, tag = "1")]
        pub messages: Vec<Any>,
        #[prost(string, tag = "2")]
        pub memo: String,
        #[prost(uint64, tag = "3")]
        pub timeout_height: u64,
    }

    /// A secp256k1 public key, the SDK's and Ethermint's alike: its 33-byte
    /// compressed point.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PubKey {
        #[prost(bytes = "vec", tag = "1")]
        pub key: Vec<u8>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Single {
        #[prost(int32, tag = "1")]
        pub mode: i32,
    }

    /// `ModeInfo`'s `single` arm of its `sum` oneof, which encodes as this
    /// optional field does.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ModeInfo {
        #[prost(message, optional, tag = "1")]
        pub single: Option<Single>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SignerInfo {
        #[prost(message, optional, tag = "1")]
        pub public_key: Option<Any>,
        #[prost(message, optional, tag = "2")]
        pub mode_info: Option<ModeInfo>,
        #[prost(uint64, tag = "3")]
        pub sequence: u64,
    }

    /// `cosmos.tx.v1beta1.Fee`, paid by the signer with no granter.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Fee {
        #[prost(message, repeated, tag = "1")]
        pub amount: Vec<Coin>,
        #[prost(uint64, tag = "2")]
        pub gas_limit: u64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AuthInfo {
        #[prost(message, repeated, tag = "1")]
        pub signer_infos: Vec<SignerInfo>,
        #[prost(message, optional, tag = "2")]
        pub fee: Option<Fee>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SignDoc {
        #[prost(bytes = "vec", tag = "1")]
        pub body_bytes: Vec<u8>,
        #[prost(bytes = "vec", tag = "2")]
        pub auth_info_bytes: Vec<u8>,
        #[prost(string, tag = "3")]
        pub chain_id: String,
        #[prost(uint64, tag = "4")]
        pub account_number: u64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TxRaw {
        #[prost(bytes = "vec", tag = "1")]
        pub body_bytes: Vec<u8>,
        #[prost(bytes = "vec", tag = "2")]
        pub auth_info_bytes: Vec<u8>,
        #[prost(bytes = "vec", repeated, tag = "3")]
        pub signatures: Vec<Vec<u8>>,
    }
}

/// `SIGN_MODE_DIRECT`: the signature is over the sign document's bytes.
const SIGN_MODE_DIRECT: i32 = 1;

const MSG_SEND: &str = "/cosmos.bank.v1beta1.MsgSend";

/// A transfer of a Cosmos network's own coin, built and ready to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedCosmosTransfer {
    /// The chain id the sign document binds.
    pub(crate) chain_id: String,
    pub(crate) sender: String,
    pub(crate) recipient: String,
    pub(crate) denom: String,
    #[serde(with = "units_u128")]
    pub(crate) amount: u128,
    /// Empty when the transfer carries none.
    pub(crate) memo: String,
    pub(crate) account_number: u64,
    pub(crate) sequence: u64,
    pub(crate) gas_limit: u64,
    /// What the transaction pays, in `denom`'s base units.
    #[serde(with = "units_u128")]
    pub(crate) fee: u128,
    /// The last block the transaction may be included in.
    pub(crate) timeout_height: u64,
}

/// The terms a Cosmos transfer is built from.
pub(crate) struct CosmosTransferTerms<'a> {
    pub sender: &'a str,
    pub recipient: &'a str,
    pub amount: u128,
    pub memo: &'a str,
    pub account_number: u64,
    pub sequence: u64,
    pub gas_limit: u64,
    pub fee: u128,
    pub timeout_height: u64,
}

/// A transfer of `chain`'s own coin, checked for what the network would
/// refuse before any of it is signed: addresses that are not this
/// network's, a zero amount, no gas and a memo over the limit.
pub(crate) fn prepare_transfer(
    chain: Chain,
    terms: CosmosTransferTerms<'_>,
) -> Result<PreparedCosmosTransfer, SendError> {
    let network = chain.cosmos_network()?;
    crate::derivation::cosmos::decode_address(chain, terms.sender)?;
    crate::derivation::cosmos::decode_address(chain, terms.recipient)?;
    if terms.amount == 0 {
        return Err(SendError::invalid("Amount must be greater than zero"));
    }
    if terms.gas_limit == 0 {
        return Err(SendError::invalid("A Cosmos transaction needs a gas limit"));
    }
    if terms.timeout_height == 0 {
        return Err(SendError::invalid(
            "A Cosmos transaction needs a timeout height",
        ));
    }
    if chain
        .memo_text_limit()
        .is_none_or(|limit| terms.memo.len() > limit)
    {
        return Err(SendError::invalid("Memo is too long for this network"));
    }
    Ok(PreparedCosmosTransfer {
        chain_id: network.chain_id.to_string(),
        sender: terms.sender.to_ascii_lowercase(),
        recipient: terms.recipient.to_ascii_lowercase(),
        denom: network.denom.to_string(),
        amount: terms.amount,
        memo: terms.memo.to_string(),
        account_number: terms.account_number,
        sequence: terms.sequence,
        gas_limit: terms.gas_limit,
        fee: terms.fee,
        timeout_height: terms.timeout_height,
    })
}

impl PreparedCosmosTransfer {
    fn coin(&self, amount: u128) -> proto::Coin {
        proto::Coin {
            denom: self.denom.clone(),
            amount: amount.to_string(),
        }
    }

    /// The `TxBody`: one `MsgSend`, the memo and the timeout height.
    pub(crate) fn body_bytes(&self) -> Vec<u8> {
        let send = proto::MsgSend {
            from_address: self.sender.clone(),
            to_address: self.recipient.clone(),
            amount: vec![self.coin(self.amount)],
        };
        proto::TxBody {
            messages: vec![proto::Any {
                type_url: MSG_SEND.into(),
                value: send.encode_to_vec(),
            }],
            memo: self.memo.clone(),
            timeout_height: self.timeout_height,
        }
        .encode_to_vec()
    }

    /// The `AuthInfo`: one direct-mode signer at the prepared sequence,
    /// carrying `public_key` (none in a simulation), and the fee.
    fn auth_info_bytes(&self, public_key: Option<proto::Any>) -> Vec<u8> {
        proto::AuthInfo {
            signer_infos: vec![proto::SignerInfo {
                public_key,
                mode_info: Some(proto::ModeInfo {
                    single: Some(proto::Single {
                        mode: SIGN_MODE_DIRECT,
                    }),
                }),
                sequence: self.sequence,
            }],
            fee: Some(proto::Fee {
                amount: (self.fee > 0)
                    .then(|| self.coin(self.fee))
                    .into_iter()
                    .collect(),
                gas_limit: self.gas_limit,
            }),
        }
        .encode_to_vec()
    }

    /// The transaction as `/cosmos/tx/v1beta1/simulate` takes it: no public
    /// key and one empty signature, which a simulation does not verify and
    /// prices as the network's own key.
    pub(crate) fn simulation_bytes(&self) -> Vec<u8> {
        proto::TxRaw {
            body_bytes: self.body_bytes(),
            auth_info_bytes: self.auth_info_bytes(None),
            signatures: vec![Vec::new()],
        }
        .encode_to_vec()
    }

    /// The signed `TxRaw` a node takes, and the hash it lands under: the
    /// uppercase hex of SHA-256 over those bytes, as nodes and explorers
    /// write it. Refused unless the prepared network is `chain`'s and `key`
    /// holds the prepared sender.
    pub(crate) fn sign(
        &self,
        chain: Chain,
        key: &SecretKey,
    ) -> Result<(Vec<u8>, String), SendError> {
        let network = chain.cosmos_network()?;
        if self.chain_id != network.chain_id || self.denom != network.denom {
            return Err(SendError::invalid(
                "Cosmos transaction was prepared for another network",
            ));
        }
        let public = PublicKey::from_secret_key(&Secp256k1::new(), key);
        if crate::derivation::cosmos::address_from_public_key(chain, &public)? != self.sender {
            return Err(SendError::invalid(
                "Cosmos sender does not match the signing key",
            ));
        }
        let body_bytes = self.body_bytes();
        let auth_info_bytes = self.auth_info_bytes(Some(proto::Any {
            type_url: network.key.type_url().into(),
            value: proto::PubKey {
                key: public.serialize().to_vec(),
            }
            .encode_to_vec(),
        }));
        let sign_doc = proto::SignDoc {
            body_bytes: body_bytes.clone(),
            auth_info_bytes: auth_info_bytes.clone(),
            chain_id: self.chain_id.clone(),
            account_number: self.account_number,
        }
        .encode_to_vec();
        let digest: [u8; 32] = match network.key {
            CosmosKey::Secp256k1 => sha2::Sha256::digest(&sign_doc).into(),
            CosmosKey::EthSecp256k1 { .. } => sha3::Keccak256::digest(&sign_doc).into(),
        };
        // libsecp256k1 signs with a low S, which both key types require.
        let signature = Secp256k1::signing_only()
            .sign_ecdsa(&Message::from_digest(digest), key)
            .serialize_compact();
        let raw = proto::TxRaw {
            body_bytes,
            auth_info_bytes,
            signatures: vec![signature.to_vec()],
        }
        .encode_to_vec();
        let hash = hex::encode_upper(sha2::Sha256::digest(&raw));
        Ok((raw, hash))
    }
}

#[cfg(test)]
#[path = "tests/cosmos.rs"]
mod tests;

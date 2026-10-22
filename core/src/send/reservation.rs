//! What a signed transaction consumes, so no second signed transaction can
//! consume it too.
//!
//! Each artifact derives its own reservations from what it signs; nothing
//! passes them in. Every key is spelled one way per resource: a txid in
//! lowercase, a sender as its chain normalizes it, bytes in lowercase hex.
//! Two artifacts spending one outpoint therefore hold one key, however their
//! indexers spelled it, and the second is refused when it is stored.

use crate::api::utxo::Txid;
use crate::registry::Chain;
use crate::send::error::SendError;
use crate::send::stages::{PreparedPayload, StoredSend, UtxoPreparedInput};

/// A resource one signed transaction consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reservation {
    /// A transaction output: the UTXO family's and Cardano's.
    Outpoint { txid: Txid, index: u32 },
    /// The sender's sequence number or nonce, on a chain with one counter
    /// per account.
    AccountSequence { sender: String, sequence: u64 },
    /// A NEAR access key's nonce: NEAR counts per key, not per account.
    AccessKeyNonce {
        sender: String,
        public_key: [u8; 32],
        nonce: u64,
    },
    /// A Monero input's key image.
    KeyImage([u8; 32]),
    /// An ICP ledger transfer's deduplication identity.
    IcpTransfer {
        sender: String,
        created_at_ns: u64,
        memo: u64,
    },
    /// The neuron an ICP staking flow controls.
    IcpNeuron {
        controller_hex: String,
        subaccount_hex: String,
    },
    /// A Solana account the transaction closes or stakes with.
    SolanaAccount(String),
    /// A Sui object at the version the transaction consumes.
    SuiObject { id: [u8; 32], version: u64 },
    /// An MWEB output the spend consumes.
    MwebOutput([u8; 32]),
}

impl Reservation {
    /// The one spelling `send_reservations` stores this under on `chain`.
    pub(crate) fn key(&self, chain: Chain) -> String {
        let chain_id = chain.str_id();
        let sender = |sender: &str| super::flow::normalize_address(chain, sender);
        match self {
            Self::Outpoint { txid, index } => format!("{chain_id}:outpoint:{txid}:{index}"),
            Self::AccountSequence {
                sender: who,
                sequence,
            } => {
                format!("{chain_id}:sequence:{}:{sequence}", sender(who))
            }
            Self::AccessKeyNonce {
                sender: who,
                public_key,
                nonce,
            } => format!(
                "{chain_id}:access-key:{}:{}:{nonce}",
                sender(who),
                hex::encode(public_key)
            ),
            Self::KeyImage(image) => format!("{chain_id}:key-image:{}", hex::encode(image)),
            Self::IcpTransfer {
                sender: who,
                created_at_ns,
                memo,
            } => format!(
                "{chain_id}:icp-transfer:{}:{created_at_ns}:{memo}",
                sender(who)
            ),
            Self::IcpNeuron {
                controller_hex,
                subaccount_hex,
            } => format!(
                "{chain_id}:neuron:{}:{}",
                controller_hex.to_ascii_lowercase(),
                subaccount_hex.to_ascii_lowercase()
            ),
            Self::SolanaAccount(account) => format!("{chain_id}:account:{account}"),
            Self::SuiObject { id, version } => {
                format!("{chain_id}:object:{}:{version}", hex::encode(id))
            }
            Self::MwebOutput(id) => format!("{chain_id}:mweb:{}", hex::encode(id)),
        }
    }
}

impl StoredSend {
    /// Everything this artifact's transaction consumes, read from what it
    /// signs. A protocol whose transactions consume nothing a second one
    /// could (Tron's, a shielded Zcash spend's) reserves nothing.
    pub(crate) fn reservations(&self) -> Result<Vec<Reservation>, SendError> {
        let sender = || self.view.sender.clone();
        let sequence = |sequence: u64| Reservation::AccountSequence {
            sender: sender(),
            sequence,
        };
        let access_key = |public_key: &[u8; 32], nonce: u64| Reservation::AccessKeyNonce {
            sender: sender(),
            public_key: *public_key,
            nonce,
        };
        let outpoints = |inputs: &[UtxoPreparedInput]| {
            inputs
                .iter()
                .map(|input| Reservation::Outpoint {
                    txid: input.utxo.txid,
                    index: input.utxo.vout,
                })
                .collect()
        };
        Ok(match &self.prepared {
            PreparedPayload::Evm(p) => vec![sequence(p.nonce)],
            PreparedPayload::AccountTransfer(p) => outpoints(&p.inputs),
            PreparedPayload::Litecoin(p) | PreparedPayload::Peercoin(p) => outpoints(&p.inputs),
            PreparedPayload::LitecoinPegIn(p) => outpoints(&p.inputs),
            PreparedPayload::LitecoinMweb(p) => p
                .inputs
                .iter()
                .map(|coin| Reservation::MwebOutput(coin.output_id))
                .collect(),
            PreparedPayload::Cardano(p) => p
                .inputs
                .iter()
                .map(|input| {
                    Ok(Reservation::Outpoint {
                        txid: input.tx_hash.parse()?,
                        index: input.tx_index,
                    })
                })
                .collect::<Result<_, SendError>>()?,
            PreparedPayload::Monero(p) => p
                .input_key_images
                .iter()
                .map(|image| Ok(Reservation::KeyImage(hex::FromHex::from_hex(image)?)))
                .collect::<Result<_, SendError>>()?,
            PreparedPayload::Near {
                public_key, nonce, ..
            } => vec![access_key(public_key, *nonce)],
            PreparedPayload::NearFunctionCall(p) => vec![access_key(&p.public_key, p.nonce)],
            PreparedPayload::NearDeleteKey(p) => vec![access_key(&p.public_key, p.nonce)],
            PreparedPayload::Ton { seqno, .. } => vec![sequence(u64::from(*seqno))],
            PreparedPayload::Xrp { sequence: s, .. }
            | PreparedPayload::XrpAccountDelete { sequence: s, .. } => {
                vec![sequence(u64::from(*s))]
            }
            PreparedPayload::XrpIssuedPayment(p) => vec![sequence(u64::from(p.sequence))],
            PreparedPayload::XrpTrustSet(p) => vec![sequence(u64::from(p.sequence))],
            PreparedPayload::Stellar { sequence: s, .. }
            | PreparedPayload::StellarAccountMerge { sequence: s, .. } => vec![sequence(*s)],
            PreparedPayload::StellarAssetPayment(p) => vec![sequence(p.sequence)],
            PreparedPayload::StellarChangeTrust(p) => vec![sequence(p.sequence)],
            PreparedPayload::Substrate(p) => vec![sequence(u64::from(p.nonce))],
            PreparedPayload::Aptos(p) => vec![sequence(
                p.body["sequence_number"]
                    .as_str()
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| SendError::invalid("Missing Aptos sequence"))?,
            )],
            PreparedPayload::Icp(p) => vec![Reservation::IcpTransfer {
                sender: sender(),
                created_at_ns: p.created_at_time_ns,
                memo: p.memo,
            }],
            PreparedPayload::IcpStaking(p) => vec![Reservation::IcpNeuron {
                controller_hex: p.controller_hex.clone(),
                subaccount_hex: p.subaccount_hex.clone(),
            }],
            PreparedPayload::Solana(p) => match &self.view.staking {
                Some(staking) => vec![Reservation::SolanaAccount(match &p.account_seed {
                    Some(seed) => super::solana::stake_account_address(&self.view.sender, seed)?,
                    None => staking
                        .position_id
                        .clone()
                        .ok_or_else(|| SendError::invalid("Missing Solana stake account"))?,
                })],
                None => Vec::new(),
            },
            PreparedPayload::SolanaAccountClosure(closure) => closure
                .accounts
                .iter()
                .map(|(account, _)| Reservation::SolanaAccount(account.clone()))
                .collect(),
            PreparedPayload::Sui(p) => sui_objects(p),
            PreparedPayload::SuiMerge(merge) => sui_objects(&merge.transaction),
            PreparedPayload::Tron(_) | PreparedPayload::ZcashShielded(_) => Vec::new(),
        })
    }
}

fn sui_objects(transfer: &super::sui::PreparedSuiTransfer) -> Vec<Reservation> {
    transfer
        .objects
        .iter()
        .map(|object| Reservation::SuiObject {
            id: object.id,
            version: object.version,
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/reservation.rs"]
mod tests;

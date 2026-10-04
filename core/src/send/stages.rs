//! Durable, secret-free transaction artifacts. Protocol fields stay typed.

use crate::send::error::SendError;
use serde::{Deserialize, Serialize};

/// Wallet-owned Litecoin source, kept with each reviewed input so signing
/// derives the exact key and checks ownership again after a restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LitecoinSendSource {
    pub address: String,
    pub derivation_path: Option<String>,
    pub script_pubkey: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LitecoinPreparedInput {
    pub source: LitecoinSendSource,
    pub utxo: (String, u32, u64, Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedLitecoinTransaction {
    pub inputs: Vec<LitecoinPreparedInput>,
    pub amount: u64,
    pub fee: u64,
    pub recipient_script: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PreparedPayload {
    Evm(super::evm::PreparedEvmTransaction),
    Zcash(super::zcash_stages::PreparedZcashTransaction),
    Icp(super::icp_stages::PreparedIcpTransaction),
    IcpStaking(super::icp_staking::PreparedIcpStaking),
    Monero(super::monero_local::PreparedMoneroTransaction),
    Decred(super::decred::PreparedDecredTransaction),
    Kaspa(super::kaspa::PreparedKaspaTransaction),
    Litecoin(PreparedLitecoinTransaction),
    Near {
        public_key: [u8; 32],
        nonce: u64,
        block_hash: [u8; 32],
        amount: u128,
        token_contract: Option<String>,
        fee_budget: String,
    },
    NearFunctionCall(super::near::PreparedNearFunctionCall),
    Ton {
        seqno: u32,
        #[serde(with = "units_u128")]
        amount: u128,
        valid_until: u32,
        jetton: Option<super::ton::PreparedJettonTransfer>,
    },
    FixedUtxo {
        inputs: Vec<(String, u32, u64, Vec<u8>)>,
        amount: u64,
        fee: u64,
        recipient_script: Vec<u8>,
    },
    Xrp {
        sequence: u32,
        fee_drops: u64,
        amount_drops: u64,
    },
    Stellar {
        sequence: u64,
        fee_stroops: u64,
        amount_stroops: i64,
    },
    Substrate(super::polkadot::PreparedPolkadotTransaction),
    Cardano {
        inputs: Vec<(String, u32, u64)>,
        amount: u64,
        fee: u64,
        ttl: u64,
    },
    Bitcoin(super::bitcoin::PreparedBitcoinTransaction),
    Solana(super::solana::PreparedSolanaTransaction),
    Tron(super::tron::PreparedTronTransfer),
    Aptos(super::aptos::PreparedAptosTransfer),
    Sui(super::sui::PreparedSuiTransfer),
}

/// Protocol amounts larger than JSON's integer range stay exact in artifacts
/// and every intermediate JSON reader. There are no legacy numeric variants.
mod units_u128 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum, PartialEq, Eq)]
pub enum SendStage {
    Prepared,
    Signed,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum, PartialEq, Eq)]
pub enum SubmissionOutcome {
    Accepted,
    Rejected,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct BroadcastAttempt {
    pub endpoint: String,
    pub attempted_at: f64,
    pub outcome: SubmissionOutcome,
    pub transaction_hash: Option<String>,
    pub detail: String,
}

/// Build-time advisories, bound to the immutable transaction and retained on resume.
#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
pub struct SendArtifactReview {
    pub warnings: Vec<crate::send::flow::HighRiskSendWarning>,
    pub recipient_warnings: Vec<crate::store::EvmRecipientPreflightWarning>,
    pub requires_self_send_confirmation: bool,
    pub staking: Option<crate::staking::StakingReview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct SendArtifact {
    pub id: String,
    pub revision: u64,
    pub stage: SendStage,
    pub wallet_id: String,
    pub chain_id: crate::registry::Chain,
    pub sender: String,
    pub recipient: String,
    pub amount: String,
    /// Native symbol or exact token contract/mint identity.
    pub asset: String,
    /// What the asset is called on screen: the coin's symbol, the known
    /// token's symbol, or the contract when no known token claims it.
    pub symbol: String,
    pub staking: Option<crate::staking::StakingRequest>,
    pub created_at: f64,
    /// Fingerprint of the complete immutable prepared content, confirmed by Sign.
    pub review_digest: String,
    pub review: SendArtifactReview,
    pub prepared_details: String,
    pub signing_payload_hex: String,
    pub signed_payload: Option<String>,
    pub transaction_hash: Option<String>,
    pub attempts: Vec<BroadcastAttempt>,
    pub selected_endpoints: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredSend {
    pub view: SendArtifact,
    pub request: super::SendExecutionRequest,
    pub prepared: PreparedPayload,
    pub submission: Option<super::payload::PreparedSubmission>,
    pub signed_digest: Option<String>,
    pub substrate_verified_through: Option<u64>,
    pub icp_staking_receipts: Vec<super::icp_staking::IcpStakingReceipt>,
}

impl StoredSend {
    pub fn digest(&self) -> Result<String, SendError> {
        use sha2::Digest;
        let bytes = serde_json::to_vec(&(
            &self.request,
            &self.prepared,
            &self.view.sender,
            &self.view.id,
            self.view.created_at,
            &self.view.signing_payload_hex,
            &self.view.asset,
            &self.view.symbol,
            &self.view.staking,
            &self.view.review,
        ))?;
        Ok(hex::encode(sha2::Sha256::digest(bytes)))
    }
    pub fn submission_digest(&self) -> Result<Option<String>, SendError> {
        use sha2::Digest;
        self.submission
            .as_ref()
            .map(|s| serde_json::to_vec(s).map(|bytes| hex::encode(sha2::Sha256::digest(bytes))))
            .transpose()
            .map_err(SendError::invalid)
    }
    pub fn validate(&self) -> Result<(), SendError> {
        let mut receipt_ids = std::collections::HashSet::new();
        for receipt in &self.icp_staking_receipts {
            if !receipt_ids.insert(&receipt.request_id) {
                return Err(SendError::invalid("Duplicate ICP execution proof"));
            }
            receipt.validate()?;
            let PreparedPayload::IcpStaking(prepared) = &self.prepared else {
                return Err(SendError::invalid(
                    "ICP execution proof attached to another protocol",
                ));
            };
            let active: Vec<super::icp_staking::SignedIcpStakingCall> = self
                .submission
                .as_ref()
                .map(|s| serde_json::from_str(&s.payload))
                .transpose()?
                .unwrap_or_default();
            let controller = hex::decode(&prepared.controller_hex)?;
            for call in active
                .iter()
                .chain(&prepared.completed_calls)
                .chain(&prepared.prior_calls)
            {
                call.validated_argument(&controller)?;
            }
            if !active
                .iter()
                .chain(&prepared.completed_calls)
                .chain(&prepared.prior_calls)
                .any(|call| receipt.matches(call))
            {
                return Err(SendError::invalid(
                    "ICP execution proof is not bound to this artifact",
                ));
            }
        }
        if self.request.password.is_some() || self.digest()? != self.view.review_digest {
            return Err(SendError::Invalid(
                "Prepared transaction was altered; build and review again".into(),
            ));
        }
        if self.request.wallet_id != self.view.wallet_id
            || self.request.chain_id != self.view.chain_id
            || self.request.to_address != self.view.recipient
            || self.request.amount_str != self.view.amount
        {
            return Err(SendError::Invalid(
                "Transaction identity was altered".into(),
            ));
        }
        if self.view.staking.as_ref().is_some_and(|intent| {
            intent.wallet_id != self.view.wallet_id || intent.chain_id != self.view.chain_id
        }) {
            return Err(SendError::invalid("Staking wallet or network was altered"));
        }
        if self.view.prepared_details != serde_json::to_string_pretty(&self.prepared)?
            || self.submission_digest()? != self.signed_digest
        {
            return Err(SendError::Invalid(
                "Transaction artifact content was altered".into(),
            ));
        }
        match (&self.submission, &self.view.stage) {
            (None, SendStage::Prepared)
                if self.view.signed_payload.is_none() && self.view.transaction_hash.is_none() => {}
            (Some(signed), SendStage::Signed)
                if self.view.signed_payload.as_ref() == Some(&signed.payload)
                    && self.view.transaction_hash == signed.transaction_hash => {}
            _ => {
                return Err(SendError::Invalid(
                    "Transaction stage does not match its signed content".into(),
                ));
            }
        }
        Ok(())
    }
}

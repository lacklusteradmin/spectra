//! Durable, secret-free transaction artifacts. Protocol fields stay typed.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PreparedPayload {
    Evm(super::evm::PreparedEvmTransaction),
    Zcash(super::zcash::PreparedZcashTransaction),
    Icp(super::icp::PreparedIcpTransaction),
    Monero(super::monero_local::PreparedMoneroTransaction),
    Decred(super::decred::PreparedDecredTransaction),
    Kaspa(super::kaspa::PreparedKaspaTransaction),
    Near {
        public_key: [u8; 32],
        nonce: u64,
        block_hash: [u8; 32],
        amount: u128,
        token_contract: Option<String>,
    },
    Ton {
        seqno: u32,
        amount: u64,
        valid_until: u32,
    },
    FixedUtxo {
        inputs: Vec<(String, u32, u64, Vec<u8>)>,
        amount: u64,
        fee: u64,
        recipient_script: Vec<u8>,
        extension: Vec<u8>,
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
    Substrate {
        nonce: u32,
        spec_version: u32,
        transaction_version: u32,
        genesis_hash: String,
        block_hash: String,
        amount: u128,
    },
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
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct SendArtifact {
    pub id: String,
    pub revision: u64,
    pub stage: SendStage,
    pub wallet_id: String,
    pub chain_id: String,
    pub sender: String,
    pub recipient: String,
    pub amount: String,
    /// Native symbol or exact token contract/mint identity.
    pub asset: String,
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
}

impl StoredSend {
    pub fn digest(&self) -> Result<String, String> {
        use sha2::Digest;
        let bytes = serde_json::to_vec(&(
            &self.request,
            &self.prepared,
            &self.view.sender,
            &self.view.id,
            self.view.created_at,
            &self.view.signing_payload_hex,
            &self.view.asset,
            &self.view.review,
        ))
        .map_err(|e| e.to_string())?;
        Ok(hex::encode(sha2::Sha256::digest(bytes)))
    }
    pub fn submission_digest(&self) -> Result<Option<String>, String> {
        use sha2::Digest;
        self.submission
            .as_ref()
            .map(|s| {
                serde_json::to_vec(s)
                    .map(|bytes| hex::encode(sha2::Sha256::digest(bytes)))
                    .map_err(|e| e.to_string())
            })
            .transpose()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.request.password.is_some() || self.digest()? != self.view.review_digest {
            return Err("Prepared transaction was altered; build and review again".into());
        }
        if self.request.wallet_id != self.view.wallet_id
            || self.request.chain_id != self.view.chain_id
            || self.request.to_address != self.view.recipient
            || self.request.amount_str != self.view.amount
        {
            return Err("Transaction identity was altered".into());
        }
        if self.view.prepared_details
            != serde_json::to_string_pretty(&self.prepared).map_err(|e| e.to_string())?
            || self.submission_digest()? != self.signed_digest
        {
            return Err("Transaction artifact content was altered".into());
        }
        match (&self.submission, &self.view.stage) {
            (None, SendStage::Prepared)
                if self.view.signed_payload.is_none() && self.view.transaction_hash.is_none() => {}
            (Some(signed), SendStage::Signed)
                if self.view.signed_payload.as_ref() == Some(&signed.payload)
                    && self.view.transaction_hash == signed.transaction_hash => {}
            _ => return Err("Transaction stage does not match its signed content".into()),
        }
        Ok(())
    }
}

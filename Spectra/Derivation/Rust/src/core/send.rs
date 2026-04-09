use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub chain_name: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: String,
    pub asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlan {
    pub chain_name: String,
    pub estimated_fee: String,
    pub signing_payload_hex: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedTransfer {
    pub chain_name: String,
    pub raw_transaction_hex: String,
    pub txid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastReceipt {
    pub chain_name: String,
    pub txid: String,
    pub source_id: String,
}

pub trait TransferPlanner: Send + Sync {
    fn build_plan(&self, request: &TransferRequest) -> Result<TransferPlan, String>;
}

pub trait TransactionBroadcaster: Send + Sync {
    fn broadcast(&self, signed_transfer: &SignedTransfer) -> Result<BroadcastReceipt, String>;
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRequest {
    pub chain_name: String,
    pub address: String,
    pub asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSnapshot {
    pub chain_name: String,
    pub address: String,
    pub asset_id: Option<String>,
    pub amount: String,
    pub block_height: Option<u64>,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRequest {
    pub chain_name: String,
    pub address: String,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedTransaction {
    pub txid: String,
    pub chain_name: String,
    pub status: String,
    pub sent_amount: Option<String>,
    pub received_amount: Option<String>,
    pub fee_amount: Option<String>,
    pub timestamp_unix: Option<u64>,
}

pub trait BalanceProvider: Send + Sync {
    fn fetch_balance(&self, request: &BalanceRequest) -> Result<BalanceSnapshot, String>;
}

pub trait HistoryProvider: Send + Sync {
    fn fetch_history(&self, request: &HistoryRequest)
        -> Result<Vec<NormalizedTransaction>, String>;
}

//! Bounded history queries and the small summary used outside the history screen.
use super::*;
use crate::store::persistence_models::CorePersistedTransactionRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum HistoryQueryFilter {
    #[default]
    All,
    Send,
    Receive,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    pub wallet_id: Option<String>,
    pub filter: HistoryQueryFilter,
    pub search: String,
    pub oldest_first: bool,
    pub offset: u64,
    pub limit: u32,
}
impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            wallet_id: None,
            filter: HistoryQueryFilter::All,
            search: String::new(),
            oldest_first: false,
            offset: 0,
            limit: 20,
        }
    }
}
#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub records: Vec<CorePersistedTransactionRecord>,
    pub has_more: bool,
    pub next_offset: u64,
}
#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionSnapshot {
    pub revision: u64,
    pub recent_and_pending: Vec<CorePersistedTransactionRecord>,
    pub replaceable: Vec<super::history_derived::ReplaceableSend>,
    pub earliest: Vec<crate::store::WalletEarliestTransactionDate>,
    pub total_count: u64,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn history_page(
        &self,
        query: HistoryQuery,
    ) -> Result<HistoryPage, SpectraBridgeError> {
        if query.limit == 0 || query.limit > 200 || query.offset > i64::MAX as u64 {
            return Err(
                "history query limit must be 1...200 and offset must fit an integer".into(),
            );
        }
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_page(&database, &query))
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
            .map_err(Into::into)
    }
    pub async fn transaction_snapshot(&self) -> Result<TransactionSnapshot, SpectraBridgeError> {
        let database = self.bound_database().await?;
        let sequence = self.projection_sequence.clone();
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_snapshot(&database, &sequence)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(e.to_string()))?
        .map_err(Into::into)
    }
    pub async fn transaction(
        &self,
        id: String,
    ) -> Result<Option<CorePersistedTransactionRecord>, SpectraBridgeError> {
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_find(&database, &id))
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
            .map_err(Into::into)
    }
}

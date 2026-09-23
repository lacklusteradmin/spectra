//! Action availability is derived from the same checks used before execution.
use crate::store::persistence_models::CorePersistedTransactionRecord;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionActions {
    /// None means available; otherwise this explains why execution is refused.
    pub recheck_unavailable_reason: Option<String>,
    pub rebroadcast_unavailable_reason: Option<String>,
}
impl Default for TransactionActions {
    fn default() -> Self {
        Self {
            recheck_unavailable_reason: Some("Transaction has not been evaluated.".into()),
            rebroadcast_unavailable_reason: Some("Transaction has not been evaluated.".into()),
        }
    }
}
impl CorePersistedTransactionRecord {
    pub(crate) fn with_actions(mut self) -> Self {
        self.actions = TransactionActions {
            recheck_unavailable_reason: super::transaction_recheck::recheck_chain(&self).err(),
            rebroadcast_unavailable_reason: super::send_records::rebroadcast_input(&self)
                .err()
                .map(|e| e.to_string()),
        };
        self
    }
}

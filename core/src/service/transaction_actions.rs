//! Action availability is derived from the same checks used before execution.
use crate::store::persistence_models::TransactionRecord;

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
impl TransactionRecord {
    /// `signed_here`: whether this device stored the send's signed
    /// transaction, which is what a rebroadcast submits.
    pub(crate) fn with_actions(mut self, signed_here: bool) -> Self {
        self.actions = TransactionActions {
            recheck_unavailable_reason: super::transaction_recheck::recheck_chain(&self)
                .err()
                .map(|e| e.to_string()),
            rebroadcast_unavailable_reason: super::send_records::rebroadcast_unavailable(
                &self,
                signed_here,
            )
            .map(str::to_string),
        };
        self
    }
}

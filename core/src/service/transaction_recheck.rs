//! Explicit status reads target a stored transaction, not a caller-built poll plan.
use crate::error::SpectraBridgeError;
use crate::registry::{Chain, PendingStatusPoll};
use crate::service::WalletService;
use crate::store::TransactionStatusChange;
use crate::store::persistence_models::TransactionRecord;

pub(super) fn recheck_chain(record: &TransactionRecord) -> Result<Chain, SpectraBridgeError> {
    let chain = record.chain_id;
    let PendingStatusPoll::Utxo { require_send_kind } = chain.pending_status_poll() else {
        return Err(SpectraBridgeError::invalid(
            "Status recheck is not available for this transaction.",
        ));
    };
    if require_send_kind && !record.kind.is_submitted() {
        return Err(SpectraBridgeError::invalid(
            "Status recheck is not available for this transaction.",
        ));
    }
    let hash = record
        .transaction_hash
        .as_deref()
        .ok_or_else(|| SpectraBridgeError::invalid("This transaction has no hash to recheck."))?;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(SpectraBridgeError::invalid(
            "This transaction has no valid hash to recheck.",
        ));
    }
    Ok(chain)
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Recheck one stored UTXO transaction even if automatic polling has stopped.
    /// Failed reads leave both the saved status and its poll tracker untouched,
    /// and are recorded in the operational log.
    pub async fn recheck_transaction_status(
        &self,
        transaction_id: String,
    ) -> Result<TransactionStatusChange, SpectraBridgeError> {
        let this = self.clone();
        crate::service::worker::run(async move {
            let this = &this;
            let result = this.recheck_stored_status(transaction_id).await;
            if let Err(error) = &result {
                this.record_event(
                    crate::service::DiagnosticLogLevel::Error,
                    "Pending Transactions",
                    format!("Status recheck failed: {error}"),
                    None,
                    None,
                )
                .await;
            }
            result
        })
        .await
    }
}

impl WalletService {
    /// The poll's own reading and write, for one transaction the poll may
    /// have stopped watching: a pending answer resumes polling.
    async fn recheck_stored_status(
        &self,
        transaction_id: String,
    ) -> Result<TransactionStatusChange, SpectraBridgeError> {
        let expected = self
            .transactions()
            .await?
            .into_iter()
            .find(|row| row.id.eq_ignore_ascii_case(&transaction_id))
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: "Transaction not found.".into(),
            })?;
        let chain = recheck_chain(&expected)?;
        let hash = expected.transaction_hash.clone().unwrap_or_default();
        let status = self.fetch_utxo_tx_status(chain, hash).await?;
        let resolution = super::pending_status::utxo_resolution(&expected, status)?;
        self.apply_polled_pending_statuses(chain, vec![resolution], Some(vec![expected]))
            .await?
            .pop()
            .ok_or_else(|| {
                SpectraBridgeError::failure(
                    "Transaction changed during status recheck; check it again.",
                )
            })
    }
}

#[cfg(test)]
#[path = "tests/transaction_recheck.rs"]
mod tests;

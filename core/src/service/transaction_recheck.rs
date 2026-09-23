//! Explicit status reads target a stored transaction, not a caller-built poll plan.
use crate::registry::{Chain, PendingStatusPoll};
use crate::service::WalletService;
use crate::store::persistence_models::CorePersistedTransactionRecord;
use crate::store::{TransactionStatusChange, TransactionStatusPollConfig};
use crate::SpectraBridgeError;

pub(super) fn recheck_chain(record: &CorePersistedTransactionRecord) -> Result<Chain, String> {
    let chain = Chain::from_display_name(&record.chain_name)
        .ok_or("Status recheck is not available for this transaction.")?;
    let PendingStatusPoll::Utxo { require_send_kind } = chain.pending_status_poll() else {
        return Err("Status recheck is not available for this transaction.".into());
    };
    if require_send_kind && record.kind != crate::store::wallet_domain::CoreTransactionKind::Send {
        return Err("Status recheck is not available for this transaction.".into());
    }
    let hash = record
        .transaction_hash
        .as_deref()
        .ok_or("This transaction has no hash to recheck.")?;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("This transaction has no valid hash to recheck.".into());
    }
    Ok(chain)
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Recheck one stored UTXO transaction even if automatic polling has stopped.
    /// Failed reads leave both the saved status and its poll tracker untouched.
    pub async fn recheck_transaction_status(
        &self,
        transaction_id: String,
    ) -> Result<TransactionStatusChange, SpectraBridgeError> {
        let database = self.bound_database().await?;
        let expected = self
            .transactions()
            .await?
            .into_iter()
            .find(|row| row.id.eq_ignore_ascii_case(&transaction_id))
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: "Transaction not found.".into(),
            })?;
        let chain = recheck_chain(&expected)
            .map_err(|message| SpectraBridgeError::InvalidInput { message })?;
        let status = self
            .fetch_utxo_tx_status(
                chain.str_id().into(),
                expected.transaction_hash.clone().unwrap(),
            )
            .await?;
        if !status
            .txid
            .eq_ignore_ascii_case(expected.transaction_hash.as_deref().unwrap())
        {
            return Err(SpectraBridgeError::from(
                "Provider returned a different transaction hash.",
            ));
        }
        let confirmations = if status.confirmed {
            status
                .confirmations
                .map(u32::try_from)
                .transpose()
                .map_err(|_| SpectraBridgeError::from("Confirmation count is out of range."))?
        } else {
            Some(0)
        };
        let block = if status.confirmed {
            status
                .block_height
                .map(i64::try_from)
                .transpose()
                .map_err(|_| SpectraBridgeError::from("Block height is out of range."))?
        } else {
            None
        };
        let confirmed = status.confirmed;
        let (change, tracker) = tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_update_chain(&database, chain.chain_display_name(), |rows| {
                let mut row = rows
                    .into_iter()
                    .find(|row| row.payload.id.eq_ignore_ascii_case(&expected.id))
                    .ok_or("Transaction was deleted during status recheck.")?;
                let current = &mut row.payload;
                if current.transaction_hash != expected.transaction_hash
                    || current.wallet_id != expected.wallet_id
                    || current.kind != expected.kind
                    || current.chain_name != expected.chain_name
                {
                    return Err("Transaction changed during status recheck; check it again.".into());
                }
                recheck_chain(current)?;
                let now = crate::wallet_db::now_secs() as f64;
                let config = TransactionStatusPollConfig::default();
                let mut trackers = std::collections::HashMap::from([(
                    current.id.clone(),
                    crate::store::transaction_status_after_successful_poll(
                        None, confirmed, now, config,
                    ),
                )]);
                let current_status = current.status;
                let old_status = current_status.as_raw().to_string();
                let new_status = if confirmed {
                    crate::store::wallet_domain::CoreTransactionStatus::Confirmed
                } else {
                    crate::store::wallet_domain::CoreTransactionStatus::Pending
                };
                let decision = crate::store::apply_resolved_pending_transaction_statuses(
                    vec![crate::store::ResolvedPendingTransactionInput {
                        id: current.id.clone(),
                        old_status: old_status.clone(),
                        old_failure_reason: current.failure_reason.clone(),
                        resolution: Some(crate::store::ResolvedPendingStatusInput {
                            status: new_status.as_raw().to_string(),
                        }),
                        is_stale_failure: false,
                    }],
                    &mut trackers,
                    now,
                    config,
                )
                .remove(0);
                current.status = new_status;
                current.failure_reason = None;
                current.receipt_block_number = block;
                current.confirmation_count = confirmations.map(i64::from);
                if !confirmed {
                    current.confirmed_network_fee = None;
                }
                let change = TransactionStatusChange {
                    id: current.id.clone(),
                    chain_name: current.chain_name.clone(),
                    transaction_hash: current.transaction_hash.clone(),
                    old_status: current_status,
                    new_status,
                    status_changed: decision.status_changed,
                };
                let tracker = trackers.remove(&current.id).unwrap();
                // Keep the indexed timestamp and unrelated metadata from the latest row.
                Ok((vec![row], (change, tracker)))
            })
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("status recheck task: {e}")))??;
        self.status_trackers
            .write()
            .await
            .insert(change.id.clone(), tracker);
        Ok(change)
    }
}

#[cfg(test)]
#[path = "transaction_recheck_tests.rs"]
mod tests;

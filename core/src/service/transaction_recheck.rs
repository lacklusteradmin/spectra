//! Explicit status reads target a stored transaction, not a caller-built poll plan.
use crate::registry::{Chain, PendingStatusPoll};
use crate::service::WalletService;
use crate::store::persistence_models::CorePersistedTransactionRecord;
use crate::store::{TransactionStatusChange, TransactionStatusPollConfig};
use crate::SpectraBridgeError;

fn recheck_chain(record: &CorePersistedTransactionRecord) -> Result<(Chain, bool), String> {
    let chain = Chain::from_display_name(&record.chain_name)
        .ok_or("Status recheck is not available for this transaction.")?;
    let PendingStatusPoll::Utxo {
        tracks_finality,
        require_send_kind,
    } = chain.pending_status_poll()
    else {
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
    Ok((chain, tracks_finality))
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Recheck one stored UTXO transaction even if automatic polling has stopped.
    /// Failed reads leave both the saved status and its poll tracker untouched.
    pub async fn recheck_transaction_status(
        &self,
        transaction_id: String,
    ) -> Result<TransactionStatusChange, SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        let expected = self
            .transactions()
            .await?
            .into_iter()
            .find(|row| row.id.eq_ignore_ascii_case(&transaction_id))
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: "Transaction not found.".into(),
            })?;
        let (chain, tracks_finality) = recheck_chain(&expected)
            .map_err(|message| SpectraBridgeError::InvalidInput { message })?;
        let status = self
            .fetch_utxo_tx_status_typed(
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
        let confirmations = if tracks_finality {
            Some(if status.confirmed {
                u32::try_from(status.confirmations.unwrap_or(0))
                    .map_err(|_| SpectraBridgeError::from("Confirmation count is out of range."))?
            } else {
                0
            })
        } else {
            None
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
            crate::wallet_db::history_update_chain(&db_path, chain.chain_display_name(), |rows| {
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
                let now = crate::store::wallet_db::now_secs() as f64;
                let config = TransactionStatusPollConfig::default();
                let mut trackers = std::collections::HashMap::from([(
                    current.id.clone(),
                    crate::store::plan_transaction_status_poll_success(
                        None,
                        confirmed,
                        !confirmed,
                        confirmations,
                        now,
                        config.clone(),
                    ),
                )]);
                let old_status = super::history_derived::status_string(current.status);
                let new_status = if confirmed { "confirmed" } else { "pending" };
                let decision = crate::store::plan_apply_resolved_pending_transaction_statuses(
                    vec![crate::store::ResolvedPendingTransactionInput {
                        id: current.id.clone(),
                        old_status: old_status.clone(),
                        old_failure_reason: current.failure_reason.clone(),
                        old_confirmations: current
                            .confirmation_count
                            .and_then(|v| u32::try_from(v).ok()),
                        resolution: Some(crate::store::ResolvedPendingStatusInput {
                            status: new_status.into(),
                            confirmations,
                        }),
                        is_stale_failure: false,
                    }],
                    &mut trackers,
                    now,
                    config,
                )
                .remove(0);
                current.status = super::history_derived::parse_status(new_status);
                current.failure_reason = None;
                current.receipt_block_number = block;
                current.confirmation_count = confirmations.map(i64::from);
                if !confirmed {
                    current.dogecoin_confirmed_network_fee_doge = None;
                }
                let change = TransactionStatusChange {
                    id: current.id.clone(),
                    chain_name: current.chain_name.clone(),
                    transaction_hash: current.transaction_hash.clone(),
                    old_status,
                    new_status: new_status.into(),
                    status_changed: decision.status_changed,
                    send_status_notification: decision.send_status_notification,
                    emit_event_code: decision.emit_event_code,
                    reached_finality_confirmations: decision.reached_finality_confirmations,
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
mod tests;

//! Transaction commands and confirmation tracking.
use super::*;

impl WalletService {
    /// Every stored history row. Internal: front ends read `transactions`.
    pub async fn fetch_all_history_records(
        &self,
    ) -> Result<Vec<crate::wallet_db::HistoryRecord>, SpectraBridgeError> {
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&database))
            .await
            .map_err(|e| SpectraBridgeError::failure(format!("spawn_blocking: {e}")))?
            .map_err(Into::into)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Change a command made to the transaction store. Ids, not records —
    /// callers re-read only what they need.
    pub async fn apply_transaction_command(
        &self,
        command: TransactionCommand,
    ) -> Result<TransactionChange, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let database = this.bound_database().await?;

            tokio::task::spawn_blocking(move || -> Result<TransactionChange, SpectraBridgeError> {
                match command {
                    TransactionCommand::Upsert { records } => {
                        if records.is_empty() {
                            return Ok(TransactionChange::default());
                        }
                        let rows: Vec<crate::wallet_db::HistoryRecord> = records
                            .into_iter()
                            .map(crate::wallet_db::history_record_from_payload)
                            .collect();
                        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
                        let existing = crate::wallet_db::history_existing_ids(&database, &ids)?;
                        crate::wallet_db::history_upsert_batch(&database, &rows)?;
                        let existing: std::collections::HashSet<String> =
                            existing.into_iter().collect();
                        let (updated, added): (Vec<String>, Vec<String>) =
                            ids.into_iter().partition(|id| existing.contains(id));
                        Ok(TransactionChange {
                            added,
                            updated,
                            removed: Vec::new(),
                        })
                    }
                    TransactionCommand::Merge {
                        incoming,
                        chain_id,
                        preserve_created_at_sentinel_unix,
                    } => {
                        let chain = chain_id;
                        crate::wallet_db::history_update_chain(&database, chain_id, |existing| {
                            merge_history_rows(
                                existing,
                                incoming,
                                chain,
                                preserve_created_at_sentinel_unix,
                            )
                        })
                    }

                    TransactionCommand::Remove { ids } => {
                        if ids.is_empty() {
                            return Ok(TransactionChange::default());
                        }
                        let ids: Vec<String> = ids.iter().map(|id| id.to_lowercase()).collect();
                        let removed = crate::wallet_db::history_existing_ids(&database, &ids)?;
                        crate::wallet_db::history_delete(&database, &ids)?;
                        Ok(TransactionChange {
                            removed,
                            ..TransactionChange::default()
                        })
                    }
                    TransactionCommand::RemoveForWallet { wallet_id } => {
                        let removed: Vec<String> =
                            crate::wallet_db::history_fetch_for_wallet(&database, &wallet_id)?
                                .into_iter()
                                .map(|record| record.id)
                                .collect();
                        crate::wallet_db::history_delete_for_wallet(&database, &wallet_id)?;
                        Ok(TransactionChange {
                            removed,
                            ..TransactionChange::default()
                        })
                    }
                    TransactionCommand::Clear => {
                        let removed: Vec<String> = crate::wallet_db::history_fetch_all(&database)?
                            .into_iter()
                            .map(|record| record.id)
                            .collect();
                        crate::wallet_db::history_clear(&database)?;
                        Ok(TransactionChange {
                            removed,
                            ..TransactionChange::default()
                        })
                    }
                }
            })
            .await
            .map_err(|e| SpectraBridgeError::failure(format!("spawn_blocking: {e}")))?
        })
        .await
    }

    /// Every stored transaction, newest first.
    pub async fn transactions(
        &self,
    ) -> Result<
        Vec<crate::store::persistence_models::CorePersistedTransactionRecord>,
        SpectraBridgeError,
    > {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let database = this.bound_database().await?;
            tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&database))
                .await
                .map_err(|e| SpectraBridgeError::failure(format!("spawn_blocking: {e}")))?
                .map(|rows| rows.into_iter().map(|row| row.payload).collect())
                .map_err(Into::into)
        })
        .await
    }

    /// What to tell the user about a send, from its stored record.
    ///
    /// A missing record answers "no notice": nothing is known to say.
    pub async fn send_verification_notice(
        &self,
        transaction_id: String,
    ) -> Result<crate::send::verification::SendVerificationNotice, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let record = this.transaction(transaction_id).await?;
            Ok(
                crate::send::verification::verification_notice_for_last_sent(
                    record
                        .as_ref()
                        .map(crate::send::verification::LastSentTransactionSnapshot::from),
                ),
            )
        })
        .await
    }
}

impl WalletService {
    /// Drop trackers for transactions nothing polls any more.
    ///
    /// Refuses when no database is bound rather than reading "core holds no
    /// transactions" as "none exist": dropping a live tracker stops a pending
    /// send from ever being polled again, where keeping a stale one costs a
    /// poll.
    ///
    /// Core works out which transactions are polled — by kind, chain, status
    /// and the chain's `pending_status_poll` shape — from its own table.
    pub(crate) async fn prune_status_trackers(&self) -> Result<(), SpectraBridgeError> {
        let live: std::collections::HashSet<String> = self
            .transactions()
            .await?
            .into_iter()
            .filter(|record| {
                {
                    super::pending_status::needs_status_poll(
                        record.kind,
                        record.status,
                        record.transaction_hash.as_deref(),
                        record.chain_id.pending_status_poll(),
                    )
                }
            })
            .map(|record| record.id)
            .collect();
        self.status_trackers
            .write()
            .await
            .retain(|id, _| live.contains(id));
        Ok(())
    }

    /// Which of `transaction_ids` are due for a confirmation poll now.
    ///
    /// An untracked transaction is always due — that is what makes a fresh
    /// launch re-poll everything pending.
    ///
    /// Not exported: the pending-status poll is core's own loop, and its only
    /// caller.
    pub async fn transactions_due_for_status_poll(
        &self,
        transaction_ids: Vec<String>,
    ) -> Vec<String> {
        let now_unix = crate::wallet_db::now_secs() as f64;
        let trackers = self.status_trackers.read().await;
        transaction_ids
            .into_iter()
            .filter(|id| {
                crate::store::should_poll_transaction_status(trackers.get(id).cloned(), now_unix)
            })
            .collect()
    }

    /// Record the outcome of one confirmation poll.
    ///
    /// Not exported: the pending-status poll is core's own loop, and its only
    /// caller.
    pub async fn record_status_poll(&self, transaction_id: String, outcome: StatusPollOutcome) {
        let now_unix = crate::wallet_db::now_secs() as f64;
        let mut trackers = self.status_trackers.write().await;
        let previous = trackers.get(&transaction_id).cloned();
        let next = match outcome {
            StatusPollOutcome::Failed => crate::store::transaction_status_after_failed_poll(
                previous,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
            StatusPollOutcome::Confirmed => crate::store::transaction_status_after_successful_poll(
                previous,
                true,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
            StatusPollOutcome::Pending => crate::store::transaction_status_after_successful_poll(
                previous,
                false,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
            StatusPollOutcome::Unresolved => {
                crate::store::transaction_status_after_successful_poll(
                    previous,
                    false,
                    now_unix,
                    TransactionStatusPollConfig::default(),
                )
            }
        };
        trackers.insert(transaction_id, next);
    }

    /// Apply one chain's resolved statuses, store the results, and report
    /// what changed.
    ///
    /// A transaction given up on stores `FAILURE_REASON_STUCK`, a code. The
    /// text a user reads is localized at render — a localized string written
    /// into the database keeps its language when the user changes theirs.
    ///
    /// The unexpected-record check is what production polls through
    /// `apply_polled_pending_statuses`; this is the same call without it,
    /// which only tests want.
    #[cfg(test)]
    pub(crate) async fn apply_resolved_pending_statuses(
        &self,
        chain_id: crate::registry::Chain,
        resolutions: Vec<crate::store::ResolvedPendingStatus>,
    ) -> Result<Vec<crate::store::TransactionStatusChange>, SpectraBridgeError> {
        self.apply_polled_pending_statuses(chain_id, resolutions, None)
            .await
    }

    pub(super) async fn apply_polled_pending_statuses(
        &self,
        chain_id: crate::registry::Chain,
        resolutions: Vec<crate::store::ResolvedPendingStatus>,
        expected: Option<Vec<crate::store::persistence_models::CorePersistedTransactionRecord>>,
    ) -> Result<Vec<crate::store::TransactionStatusChange>, SpectraBridgeError> {
        use super::history_derived::status_string;
        let stale: std::collections::HashSet<String> = self
            .stale_pending_failure_ids(chain_id)
            .await?
            .into_iter()
            .collect();
        let by_id: HashMap<String, crate::store::ResolvedPendingStatus> =
            resolutions.into_iter().map(|r| (r.id.clone(), r)).collect();
        if by_id.is_empty() && stale.is_empty() {
            return Ok(Vec::new());
        }

        let database = self.bound_database().await?;
        // Keep tracker changes and the database commit ordered, including when
        // the caller cancels while the blocking transaction is running.
        let mut tracker_guard = self.status_trackers.clone().write_owned().await;
        let changes = tokio::task::spawn_blocking(move || -> Result<_, SpectraBridgeError> {
            let mut next_trackers = tracker_guard.clone();
            let changes = crate::wallet_db::history_update_chain(&database, chain_id, |rows| {
                let stored: Vec<_> = rows
                    .into_iter()
                    .map(|row| row.payload)
                    .filter(|t| by_id.contains_key(&t.id) || stale.contains(&t.id))
                    .filter(|t| {
                        expected.as_ref().is_none_or(|snapshots| {
                            snapshots.iter().any(|old| {
                                old.id == t.id
                                    && old.wallet_id == t.wallet_id
                                    && old.chain_id == t.chain_id
                                    && old.transaction_hash == t.transaction_hash
                                    && old.kind == t.kind
                                    && old.status == t.status
                                    && old.receipt_block_number == t.receipt_block_number
                                    && old.confirmation_count == t.confirmation_count
                            })
                        })
                    })
                    .collect();
                let inputs: Vec<crate::store::ResolvedPendingTransactionInput> = stored
                    .iter()
                    .map(|t| crate::store::ResolvedPendingTransactionInput {
                        id: t.id.clone(),
                        old_status: status_string(t.status),
                        old_failure_reason: t.failure_reason.clone(),
                        resolution: by_id.get(&t.id).map(|r| {
                            crate::store::ResolvedPendingStatusInput {
                                status: r.status.clone(),
                            }
                        }),
                        is_stale_failure: stale.contains(&t.id)
                            && t.status
                                == crate::store::wallet_domain::CoreTransactionStatus::Pending,
                    })
                    .collect();

                let now_unix = crate::wallet_db::now_secs() as f64;
                let decisions = {
                    crate::store::apply_resolved_pending_transaction_statuses(
                        inputs,
                        &mut next_trackers,
                        now_unix,
                        TransactionStatusPollConfig::default(),
                    )
                };

                let stored_by_id: HashMap<
                    &str,
                    &crate::store::persistence_models::CorePersistedTransactionRecord,
                > = stored.iter().map(|t| (t.id.as_str(), t)).collect();
                let mut writes = Vec::new();
                let mut changes = Vec::new();
                for decision in decisions {
                    let Some(old) = stored_by_id.get(decision.id.as_str()).copied() else {
                        continue;
                    };
                    let Some(new_status) =
                        crate::store::wallet_domain::CoreTransactionStatus::from_raw(
                            &decision.new_status,
                        )
                    else {
                        continue;
                    };
                    let resolution = by_id.get(&decision.id);
                    let mut updated = old.clone();
                    updated.status = new_status;
                    updated.failure_reason = match decision.failure_reason_disposition {
                        crate::store::FailureReasonDisposition::None => None,
                        crate::store::FailureReasonDisposition::Preserve => {
                            old.failure_reason.clone()
                        }
                        crate::store::FailureReasonDisposition::LocalizedFallback => Some(
                            crate::store::persistence_models::TransactionFailure::StuckAfterRetries,
                        ),
                    };
                    if new_status == crate::store::wallet_domain::CoreTransactionStatus::Pending {
                        updated.receipt_block_number = None;
                        updated.receipt_gas_used = None;
                        updated.receipt_effective_gas_price_gwei = None;
                        updated.receipt_network_fee = None;
                        updated.confirmation_count = None;
                        updated.confirmed_network_fee = None;
                        let next = crate::store::transaction_status_after_successful_poll(
                            next_trackers.get(&updated.id).cloned(),
                            false,
                            now_unix,
                            TransactionStatusPollConfig::default(),
                        );
                        next_trackers.insert(updated.id.clone(), next);
                    }
                    if let Some(r) = resolution {
                        if let Some(block) = r.receipt_block_number {
                            updated.receipt_block_number = Some(block);
                        }
                        if let Some(c) = r.confirmations {
                            updated.confirmation_count = Some(i64::from(c));
                        }
                        if let Some(cost) = &r.evm_receipt_cost {
                            updated.receipt_gas_used = Some(cost.gas_used.clone());
                            updated.receipt_effective_gas_price_gwei =
                                Some(cost.effective_gas_price_gwei.clone());
                            updated.receipt_network_fee = Some(cost.network_fee.clone());
                        }
                    }
                    changes.push(crate::store::TransactionStatusChange {
                        id: decision.id.clone(),
                        chain_id: updated.chain_id,
                        transaction_hash: updated.transaction_hash.clone(),
                        old_status: old.status,
                        new_status,
                        status_changed: decision.status_changed,
                    });
                    writes.push(crate::wallet_db::history_record_from_payload(updated));
                }
                Ok::<_, SpectraBridgeError>((writes, changes))
            })?;
            *tracker_guard = next_trackers;
            Ok(changes)
        })
        .await
        .map_err(|e| SpectraBridgeError::failure(format!("spawn_blocking: {e}")))??;
        self.record_status_changes(&changes).await;
        Ok(changes)
    }

    /// Stored transactions for one wallet, newest first.
    pub async fn transactions_for_wallet(
        &self,
        wallet_id: String,
    ) -> Result<
        Vec<crate::store::persistence_models::CorePersistedTransactionRecord>,
        SpectraBridgeError,
    > {
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_fetch_for_wallet(&database, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::failure(format!("spawn_blocking: {e}")))?
        .map(|rows| rows.into_iter().map(|row| row.payload).collect())
        .map_err(Into::into)
    }
}

/// What one confirmation poll found: one of three outcomes, so no
/// meaningless combination can be expressed.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum StatusPollOutcome {
    /// The provider reported the transaction confirmed.
    Confirmed,
    /// The provider reported it still pending.
    Pending,
    /// The provider answered without resolving it either way.
    Unresolved,
    /// The poll itself failed — a network or provider error, not a verdict.
    Failed,
}

/// Cheap content signature, to tell an unchanged merge result from a real one.
/// Serialization is enough: these records are flat and compare by value.
fn fingerprint(record: &crate::fetch::transactions::CoreTransactionRecord) -> String {
    serde_json::to_string(record).unwrap_or_default()
}

#[cfg(test)]
mod audit_fix5_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_history_merges_keep_one_identity_per_wallet() {
        let service = WalletService::new(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "atomic-history-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(24));
        let mut tasks = Vec::new();
        for i in 0..24 {
            let service = service.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                let record: crate::store::persistence_models::CorePersistedTransactionRecord = serde_json::from_value(json!({
                    "id":crate::store::new_transaction_id(), "walletId":format!("wallet-{}",i%2), "walletName":"W", "kind":"send", "status":"confirmed", "chainId":"ethereum", "transactionHash":"0xshared", "amount":"1", "symbol":"ETH", "assetDisplayName":"Ether", "address":"0xrecipient", "createdAtUnix":1000.0
                })).unwrap();
                barrier.wait().await;
                service.apply_transaction_command(TransactionCommand::Merge { incoming:vec![record.into()], chain_id:crate::registry::Chain::Ethereum, preserve_created_at_sentinel_unix:None }).await.unwrap()
            }));
        }
        let mut added = 0;
        for task in tasks {
            added += task.await.unwrap().added.len();
        }
        assert_eq!(added, 2);
        let records = service.transactions().await.unwrap();
        assert_eq!(records.len(), 2);
        assert_ne!(records[0].wallet_id, records[1].wallet_id);
    }
}

#[cfg(test)]
mod status_commit_regressions {
    use super::*;
    use crate::store::persistence_models::CorePersistedTransactionRecord;

    fn record(id: &str, time: f64) -> CorePersistedTransactionRecord {
        serde_json::from_value(json!({
            "id":id, "walletId":"W", "walletName":"Before", "kind":"send", "status":"pending",
            "chainId":"bitcoin", "transactionHash":format!("hash-{id}"), "amount":"1",
            "symbol":"BTC", "assetDisplayName":"Bitcoin", "address":"recipient", "createdAtUnix":time
        }))
        .unwrap()
    }
    fn resolution(id: &str, status: &str) -> crate::store::ResolvedPendingStatus {
        crate::store::ResolvedPendingStatus {
            id: id.into(),
            status: status.into(),
            confirmations: Some(12),
            receipt_block_number: Some(900000),
            evm_receipt_cost: None,
        }
    }
    async fn setup() -> (Arc<WalletService>, String) {
        let service = WalletService::new(vec![]).unwrap();
        let db = std::env::temp_dir()
            .join(format!(
                "status-atomic-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .to_string();
        service.open_state(db.clone()).await.unwrap();
        (service, db)
    }

    #[tokio::test]
    async fn status_commit_keeps_unix_sorting_and_canonical_wallet_index() {
        let (service, _) = setup().await;
        service
            .apply_transaction_command(TransactionCommand::Upsert {
                records: vec![record("old", 700000000.0), record("NEW", 800000000.0)],
            })
            .await
            .unwrap();
        service
            .apply_resolved_pending_statuses(
                crate::registry::Chain::Bitcoin,
                vec![resolution("NEW", "confirmed")],
            )
            .await
            .unwrap();
        let rows = service.fetch_all_history_records().await.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["new", "old"]
        );
        assert_eq!(rows[0].created_at, 800000000.0);
        assert_eq!(
            service
                .transactions_for_wallet("W".into())
                .await
                .unwrap()
                .len(),
            2
        );
        let changes = service
            .apply_polled_pending_statuses(
                crate::registry::Chain::Bitcoin,
                vec![resolution("NEW", "pending")],
                Some(vec![record("NEW", 800000000.0)]),
            )
            .await
            .unwrap();
        assert!(
            changes.is_empty(),
            "a late pending read must not undo confirmation"
        );
    }

    #[tokio::test]
    async fn status_fresh_reorgs_and_failed_receipts_can_correct_confirmed_history() {
        let (service, _) = setup().await;
        let mut tx = record("tx", 0.0);
        tx.status = crate::store::wallet_domain::CoreTransactionStatus::Confirmed;
        tx.receipt_block_number = Some(123);
        tx.confirmation_count = Some(12);
        service
            .apply_transaction_command(TransactionCommand::Upsert {
                records: vec![tx.clone()],
            })
            .await
            .unwrap();
        let mut pending = resolution("tx", "pending");
        pending.confirmations = None;
        pending.receipt_block_number = None;
        let changes = service
            .apply_polled_pending_statuses(
                crate::registry::Chain::Bitcoin,
                vec![pending],
                Some(vec![tx]),
            )
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        let row = service.transactions().await.unwrap().remove(0);
        assert_eq!(
            row.status,
            crate::store::wallet_domain::CoreTransactionStatus::Pending
        );
        assert!(row.receipt_block_number.is_none());
        assert!(row.confirmation_count.is_none());
        service
            .apply_resolved_pending_statuses(
                crate::registry::Chain::Bitcoin,
                vec![resolution("tx", "confirmed")],
            )
            .await
            .unwrap();
        let current = service.transactions().await.unwrap();
        let changes = service
            .apply_polled_pending_statuses(
                crate::registry::Chain::Bitcoin,
                vec![resolution("tx", "failed")],
                Some(current),
            )
            .await
            .unwrap();
        assert_eq!(
            changes[0].new_status,
            crate::store::wallet_domain::CoreTransactionStatus::Failed
        );
    }

    #[tokio::test]
    async fn failed_status_commit_does_not_publish_tracker_changes() {
        let (service, db) = setup().await;
        service
            .apply_transaction_command(TransactionCommand::Upsert {
                records: vec![record("tx", 0.0)],
            })
            .await
            .unwrap();
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_status BEFORE UPDATE ON history_records BEGIN SELECT RAISE(FAIL, 'test refusal'); END;").unwrap();
        assert!(
            service
                .apply_resolved_pending_statuses(
                    crate::registry::Chain::Bitcoin,
                    vec![resolution("tx", "confirmed")]
                )
                .await
                .is_err()
        );
        assert!(service.status_trackers.read().await.is_empty());
        assert_eq!(
            service.transactions().await.unwrap()[0].status,
            crate::store::wallet_domain::CoreTransactionStatus::Pending
        );
        conn.execute_batch("DROP TRIGGER reject_status;").unwrap();
        service
            .apply_resolved_pending_statuses(
                crate::registry::Chain::Bitcoin,
                vec![resolution("tx", "confirmed")],
            )
            .await
            .unwrap();
        assert!(service.status_trackers.read().await["tx"].polling_complete);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_status_commits_preserve_metadata_and_never_resurrect_deleted_rows() {
        let (service, db) = setup().await;
        for i in 0..24 {
            let id = format!("tx{i}");
            service
                .apply_transaction_command(TransactionCommand::Upsert {
                    records: vec![record(&id, 0.0)],
                })
                .await
                .unwrap();
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let sender = service.clone();
            let gate = barrier.clone();
            let key = id.clone();
            let status = tokio::spawn(async move {
                gate.wait().await;
                sender
                    .apply_resolved_pending_statuses(
                        crate::registry::Chain::Bitcoin,
                        vec![resolution(&key, "confirmed")],
                    )
                    .await
                    .unwrap();
            });
            barrier.wait().await;
            let path = db.clone();
            let key = id.clone();
            tokio::task::spawn_blocking(move || {
                if i % 2 == 0 {
                    crate::wallet_db::history_delete(
                        &crate::wallet_db::WalletDatabase::new(&path),
                        &[key],
                    )
                    .unwrap();
                } else {
                    crate::wallet_db::history_update_chain(
                        &crate::wallet_db::WalletDatabase::new(&path),
                        crate::registry::Chain::Bitcoin,
                        |rows| {
                            let writes = rows
                                .into_iter()
                                .filter(|r| r.id == key)
                                .map(|r| {
                                    let mut payload = r.payload;
                                    payload.wallet_name = "After".into();
                                    crate::wallet_db::history_record_from_payload(payload)
                                })
                                .collect();
                            Ok::<_, crate::wallet_db::error::DbError>((writes, ()))
                        },
                    )
                    .unwrap();
                }
            })
            .await
            .unwrap();
            status.await.unwrap();
            let rows = service.transactions().await.unwrap();
            let stored = rows.iter().find(|r| r.id == id);
            if i % 2 == 0 {
                assert!(stored.is_none());
            } else {
                let row = stored.unwrap();
                assert_eq!(row.wallet_name, "After");
                assert_eq!(
                    row.status,
                    crate::store::wallet_domain::CoreTransactionStatus::Confirmed
                );
            }
        }
    }
}

fn merge_history_rows(
    existing: Vec<crate::wallet_db::HistoryRecord>,
    incoming: Vec<crate::fetch::transactions::CoreTransactionRecord>,
    chain: Chain,
    preserve_created_at_sentinel_unix: Option<f64>,
) -> Result<(Vec<crate::wallet_db::HistoryRecord>, TransactionChange), SpectraBridgeError> {
    let existing: Vec<crate::fetch::transactions::CoreTransactionRecord> =
        existing.into_iter().map(|row| row.payload.into()).collect();
    let before: std::collections::HashMap<String, String> = existing
        .iter()
        .map(|record| (record.id.to_lowercase(), fingerprint(record)))
        .collect();

    let merged = crate::fetch::transactions::merge_transactions(
        crate::fetch::transactions::TransactionMergeRequest {
            existing_transactions: existing,
            incoming_transactions: incoming,
            strategy: chain.transaction_merge_strategy(),
            chain_id: chain,
            preserve_created_at_sentinel_unix,
        },
    );

    // Only records the merge actually altered are written — a
    // history refresh mostly returns what is already stored.
    let mut added = Vec::new();
    let mut updated = Vec::new();
    let mut rows = Vec::new();
    for record in merged {
        let id = record.id.to_lowercase();
        match before.get(&id) {
            Some(previous) if *previous == fingerprint(&record) => continue,
            Some(_) => updated.push(id),
            None => added.push(id),
        }
        rows.push(crate::wallet_db::history_record_from_payload(record.into()));
    }
    Ok((
        rows,
        TransactionChange {
            added,
            updated,
            removed: Vec::new(),
        },
    ))
}

impl WalletService {
    /// A provider response may only write history for a wallet still present
    /// on that exact network. The ownership check shares the merge transaction.
    pub(super) async fn merge_fetched_history(
        &self,
        incoming: Vec<crate::fetch::transactions::CoreTransactionRecord>,
    ) -> Result<TransactionChange, SpectraBridgeError> {
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || -> Result<TransactionChange, SpectraBridgeError> {
            let mut groups = std::collections::BTreeMap::<Chain, Vec<_>>::new();
            for row in incoming {
                groups.entry(row.chain_id).or_default().push(row);
            }
            let mut combined = TransactionChange::default();
            for (chain, incoming) in groups {
                let change = crate::wallet_db::history_update_chain_checked(
                    &database,
                    chain,
                    |conn, existing| {
                        use rusqlite::OptionalExtension;
                        let mut accepted = Vec::new();
                        let mut query = conn
                            .prepare_cached(
                                "SELECT name, json_extract(payload, '$.chainId') FROM wallets WHERE id = ?1",
                            )
                            .map_err(crate::wallet_db::error::DbError::from)?;
                        for mut record in incoming {
                            let Some(id) = record.wallet_id.as_deref() else {
                                continue;
                            };
                            let owner: Option<(String, String)> = query
                                .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
                                .optional()
                                .map_err(crate::wallet_db::error::DbError::from)?;
                            let Some((wallet_name, chain_id)) = owner else { continue; };
                            if chain_id != chain.str_id() { continue; }
                            record.wallet_name = wallet_name;
                            accepted.push(record);
                        }
                        merge_history_rows(
                            existing,
                            accepted,
                            chain,
                            Some(super::history_refresh::SENTINEL_CREATED_AT_UNIX),
                        )
                    },
                )?;
                combined.added.extend(change.added);
                combined.updated.extend(change.updated);
            }
            Ok(combined)
        })
        .await
        .map_err(SpectraBridgeError::failure)?
    }
}

impl WalletService {
    #[cfg(test)]
    pub(crate) async fn upsert_history_records(
        &self,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_upsert_batch(&database, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::failure(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }
}

impl WalletService {
    pub(crate) async fn stale_pending_failure_ids(
        &self,
        chain_id: crate::registry::Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        use crate::store::wallet_domain::CoreTransactionKind::Send;
        // Whether receives count is `Chain::pending_status_poll`'s
        // `require_send_kind` — Litecoin's explorer confirms receives on its
        // own cadence, so its sweep tracks them too.
        let require_send_kind = match chain_id.pending_status_poll() {
            crate::registry::PendingStatusPoll::Utxo {
                require_send_kind, ..
            } => require_send_kind,
            _ => true,
        };
        let failures: HashMap<String, u32> = self
            .status_trackers
            .read()
            .await
            .iter()
            .map(|(id, tracker)| (id.clone(), tracker.consecutive_failures))
            .collect();
        let inputs: Vec<crate::store::StalePendingFailureTransactionInput> = self
            .transactions()
            .await?
            .into_iter()
            .filter(|t| t.chain_id == chain_id && (!require_send_kind || t.kind == Send))
            // A missing response/receipt cannot prove a journaled send failed.
            // Keep its nonce reserved until an actual network outcome is known.
            .filter(|t| {
                t.signed_transaction_payload_format.as_deref() != Some("core.submission_json")
            })
            .map(|t| crate::store::StalePendingFailureTransactionInput {
                id: t.id,
                created_at_unix: t.created_at_unix,
                status_is_pending: t.status
                    == crate::store::wallet_domain::CoreTransactionStatus::Pending,
            })
            .collect();
        Ok(crate::store::stale_pending_failure_ids(
            inputs,
            &failures,
            crate::wallet_db::now_secs() as f64,
            TransactionStatusPollConfig::default(),
        ))
    }
}

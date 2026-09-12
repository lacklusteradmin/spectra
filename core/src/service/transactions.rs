//! Transaction commands and confirmation tracking.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Upsert a batch of transaction history records. `records[*].payload`
    /// is the typed `CorePersistedTransactionRecord`; Rust serializes to JSON
    /// for the SQLite TEXT column internally — no JSON crosses the FFI.
    pub(crate) async fn upsert_history_records(
        &self,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_upsert_batch(&db_path, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    pub async fn fetch_all_history_records_typed(
        &self,
    ) -> Result<Vec<crate::wallet_db::HistoryRecord>, SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(Into::into)
    }

    /// Change a command made to the transaction store. Ids, not records —
    /// callers re-read only what they need.
    pub async fn apply_transaction_command(
        &self,
        command: TransactionCommand,
    ) -> Result<TransactionChange, SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;

        tokio::task::spawn_blocking(move || -> Result<TransactionChange, String> {
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
                    let existing = crate::wallet_db::history_existing_ids(&db_path, &ids)?;
                    crate::wallet_db::history_upsert_batch(&db_path, &rows)?;
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
                    chain_name,
                    preserve_created_at_sentinel_unix,
                } => {
                    let chain = Chain::from_display_name(&chain_name)
                        .ok_or_else(|| format!("merge: unknown chain {chain_name:?}"))?;
                    let strategy = chain.transaction_merge_strategy();
                    let include_symbol_in_identity = chain.merge_identity_includes_symbol();
                    crate::wallet_db::history_update_chain(&db_path, &chain_name, |existing| {
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
                                strategy,
                                chain_name: chain_name.clone(),
                                include_symbol_in_identity,
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
                    })
                }
                TransactionCommand::Remove { ids } => {
                    if ids.is_empty() {
                        return Ok(TransactionChange::default());
                    }
                    let ids: Vec<String> = ids.iter().map(|id| id.to_lowercase()).collect();
                    let removed = crate::wallet_db::history_existing_ids(&db_path, &ids)?;
                    crate::wallet_db::history_delete(&db_path, &ids)?;
                    Ok(TransactionChange {
                        removed,
                        ..TransactionChange::default()
                    })
                }
                TransactionCommand::RemoveForWallet { wallet_id } => {
                    let removed: Vec<String> =
                        crate::wallet_db::history_fetch_for_wallet(&db_path, &wallet_id)?
                            .into_iter()
                            .map(|record| record.id)
                            .collect();
                    crate::wallet_db::history_delete_for_wallet(&db_path, &wallet_id)?;
                    Ok(TransactionChange {
                        removed,
                        ..TransactionChange::default()
                    })
                }
                TransactionCommand::Clear => {
                    let removed: Vec<String> = crate::wallet_db::history_fetch_all(&db_path)?
                        .into_iter()
                        .map(|record| record.id)
                        .collect();
                    crate::wallet_db::history_clear(&db_path)?;
                    Ok(TransactionChange {
                        removed,
                        ..TransactionChange::default()
                    })
                }
            }
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    /// Every stored transaction, newest first.
    pub async fn transactions(
        &self,
    ) -> Result<
        Vec<crate::store::persistence_models::CorePersistedTransactionRecord>,
        SpectraBridgeError,
    > {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map(|rows| rows.into_iter().map(|row| row.payload).collect())
            .map_err(Into::into)
    }

    /// Force `transaction_id` to be polled on the next sweep.
    ///
    /// `clear_finality` re-opens a transaction that had already been treated as
    /// final — the UTXO chains do this when a reorg is suspected.
    pub async fn reset_status_tracker(&self, transaction_id: String, clear_finality: bool) {
        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let mut trackers = self.status_trackers.write().await;
        let entry = trackers
            .entry(transaction_id)
            .or_insert_with(|| TransactionStatusTrackerState::initial(now_unix));
        entry.next_check_at_unix = f64::NEG_INFINITY;
        if clear_finality {
            entry.reached_finality = false;
        }
    }

    /// Drop trackers for transactions that no longer exist.
    /// Keep only these trackers and forget the rest.
    ///
    /// `clear_status_trackers()` was a second name for this with an empty list,
    /// and one call site already spelled it that way.
    /// Drop trackers for transactions nothing polls any more.
    ///
    /// Refuses when no database is bound rather than reading "core holds no
    /// transactions" as "none exist": dropping a live tracker stops a pending
    /// send from ever being polled again, where keeping a stale one costs a
    /// poll.
    ///
    /// Took the ids to keep, which meant the front end filtered core's own
    /// transaction table — by kind, by chain, by status, and by the chain's
    /// `pending_status_poll` shape — and told core the answer. Every one of
    /// those is core's, so core works it out.
    pub async fn prune_status_trackers(&self) -> Result<(), SpectraBridgeError> {
        let live: std::collections::HashSet<String> = self
            .transactions()
            .await?
            .into_iter()
            .filter(|record| {
                Chain::from_display_name(&record.chain_name).is_some_and(|chain| {
                    super::pending_status::needs_status_poll(
                        record.kind,
                        record.status,
                        record.transaction_hash.as_deref(),
                        chain.pending_status_poll(),
                    )
                })
            })
            .map(|record| record.id)
            .collect();
        self.status_trackers
            .write()
            .await
            .retain(|id, _| live.contains(id));
        Ok(())
    }

    /// Pending transactions old enough, and failing often enough, to be treated
    /// as failed. Failure counts come from core's own trackers.
    /// Sends on `chain_name` that have been pending too long and failed to
    /// resolve often enough to call it.
    ///
    /// The chain is a parameter because the sweep is per chain: reading every
    /// transaction here would mark sends on chains the caller was not polling.
    pub(crate) async fn stale_pending_failure_ids(
        &self,
        chain_name: String,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        use crate::store::wallet_domain::CoreTransactionKind::Send;
        // Whether receives count is `Chain::pending_status_poll`'s
        // `require_send_kind` — Litecoin's explorer confirms receives on its
        // own cadence, so its sweep tracks them too.
        let require_send_kind = crate::registry::Chain::from_display_name(&chain_name)
            .map(|chain| match chain.pending_status_poll() {
                crate::registry::PendingStatusPoll::Utxo {
                    require_send_kind, ..
                } => require_send_kind,
                _ => true,
            })
            .unwrap_or(true);
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
            .filter(|t| t.chain_name == chain_name && (!require_send_kind || t.kind == Send))
            // A missing response/receipt cannot prove a journaled send failed.
            // Keep its nonce reserved until an actual network outcome is known.
            .filter(|t| {
                t.signed_transaction_payload_format.as_deref() != Some("core.submission_json")
            })
            .map(|t| crate::store::StalePendingFailureTransactionInput {
                id: t.id,
                created_at_unix: t.created_at
                    + crate::store::persistence_models::SWIFT_REFERENCE_EPOCH_OFFSET_SECS,
                status_is_pending: t.status
                    == Some(crate::store::wallet_domain::CoreTransactionStatus::Pending),
            })
            .collect();
        Ok(crate::store::plan_stale_pending_failure_ids(
            inputs,
            &failures,
            crate::store::wallet_db::now_secs() as f64,
            TransactionStatusPollConfig::default(),
        ))
    }
}

impl WalletService {
    // Not exported: the pending-status poll is core's own loop now, and it
    // is the only caller. It was an export because a front end drove the
    // loop and asked for each piece.
    /// Which of `transaction_ids` are due for a confirmation poll now.
    ///
    /// An untracked transaction is always due — that is what makes a fresh
    /// launch re-poll everything pending.
    pub async fn transactions_due_for_status_poll(
        &self,
        transaction_ids: Vec<String>,
    ) -> Vec<String> {
        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let trackers = self.status_trackers.read().await;
        transaction_ids
            .into_iter()
            .filter(|id| {
                crate::store::plan_transaction_status_should_poll(
                    trackers.get(id).cloned(),
                    now_unix,
                )
            })
            .collect()
    }

    // Not exported: the pending-status poll is core's own loop now, and it
    // is the only caller. It was an export because a front end drove the
    // loop and asked for each piece.
    /// Record the outcome of one confirmation poll.
    ///
    /// Two methods before, and the success arm took `resolved_status_confirmed`
    /// and `resolved_status_pending` as separate booleans — a three-state
    /// written as two, so "confirmed and pending" was representable and had no
    /// meaning. The outcome is the outcome.
    pub async fn record_status_poll(&self, transaction_id: String, outcome: StatusPollOutcome) {
        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let mut trackers = self.status_trackers.write().await;
        let previous = trackers.get(&transaction_id).cloned();
        let next = match outcome {
            StatusPollOutcome::Failed => crate::store::plan_transaction_status_poll_failure(
                previous,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
            StatusPollOutcome::Confirmed { confirmations } => {
                crate::store::plan_transaction_status_poll_success(
                    previous,
                    true,
                    false,
                    confirmations,
                    now_unix,
                    TransactionStatusPollConfig::default(),
                )
            }
            StatusPollOutcome::Pending => crate::store::plan_transaction_status_poll_success(
                previous,
                false,
                true,
                None,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
            StatusPollOutcome::Unresolved => crate::store::plan_transaction_status_poll_success(
                previous,
                false,
                false,
                None,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
        };
        trackers.insert(transaction_id, next);
    }

    // Not exported: the pending-status poll is core's own loop now, and it
    // is the only caller. It was an export because a front end drove the
    // loop and asked for each piece.
    /// Decide what each resolved pending transaction becomes, advancing the
    /// confirmation trackers as a side effect.
    /// Apply one chain's resolved statuses, store the results, and report
    /// what changed.
    ///
    /// The caller used to send core an input per transaction built from its own
    /// projection — old status, old failure reason, old confirmations — take
    /// back a decision per transaction, apply it to build new records, and
    /// upsert those into core. Every value in that round trip except the
    /// resolutions came from the store it ended up back in.
    ///
    /// A transaction given up on stores `FAILURE_REASON_STUCK`, a code. The
    /// text a user reads is localized at render — a localized string written
    /// into the database keeps its language when the user changes theirs.
    pub async fn apply_resolved_pending_statuses(
        &self,
        chain_name: String,
        resolutions: Vec<crate::store::ResolvedPendingStatus>,
    ) -> Result<Vec<crate::store::TransactionStatusChange>, SpectraBridgeError> {
        use super::history_derived::{parse_status, status_string};
        let stale: std::collections::HashSet<String> = self
            .stale_pending_failure_ids(chain_name.clone())
            .await?
            .into_iter()
            .collect();
        let by_id: HashMap<String, crate::store::ResolvedPendingStatus> =
            resolutions.into_iter().map(|r| (r.id.clone(), r)).collect();
        if by_id.is_empty() && stale.is_empty() {
            return Ok(Vec::new());
        }

        let stored: Vec<_> = self
            .transactions()
            .await?
            .into_iter()
            .filter(|t| {
                t.chain_name == chain_name && (by_id.contains_key(&t.id) || stale.contains(&t.id))
            })
            .collect();

        let inputs: Vec<crate::store::ResolvedPendingTransactionInput> = stored
            .iter()
            .map(|t| crate::store::ResolvedPendingTransactionInput {
                id: t.id.clone(),
                old_status: status_string(t.status),
                old_failure_reason: t.failure_reason.clone(),
                old_confirmations: t.confirmation_count.map(|c| c.max(0) as u32),
                resolution: by_id
                    .get(&t.id)
                    .map(|r| crate::store::ResolvedPendingStatusInput {
                        status: r.status.clone(),
                        confirmations: r.confirmations,
                    }),
                is_stale_failure: stale.contains(&t.id),
            })
            .collect();

        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let decisions = {
            let mut trackers = self.status_trackers.write().await;
            crate::store::plan_apply_resolved_pending_transaction_statuses(
                inputs,
                &mut trackers,
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
            let Some(new_status) = parse_status(&decision.new_status) else {
                continue;
            };
            let resolution = by_id.get(&decision.id);
            let mut updated = old.clone();
            updated.status = Some(new_status);
            updated.failure_reason = match decision.failure_reason_disposition {
                crate::store::FailureReasonDisposition::None => None,
                crate::store::FailureReasonDisposition::Preserve => old.failure_reason.clone(),
                crate::store::FailureReasonDisposition::LocalizedFallback => {
                    Some(crate::store::FAILURE_REASON_STUCK.to_string())
                }
            };
            if let Some(r) = resolution {
                if let Some(block) = r.receipt_block_number {
                    updated.receipt_block_number = Some(block);
                }
                if let Some(c) = r.confirmations {
                    updated.confirmation_count = Some(i64::from(c));
                }
                if let Some(fee) = r.dogecoin_network_fee_doge {
                    updated.dogecoin_confirmed_network_fee_doge = Some(fee);
                }
            }
            changes.push(crate::store::TransactionStatusChange {
                id: decision.id.clone(),
                chain_name: updated.chain_name.clone(),
                transaction_hash: updated.transaction_hash.clone(),
                old_status: status_string(old.status),
                new_status: decision.new_status.clone(),
                status_changed: decision.status_changed,
                send_status_notification: decision.send_status_notification,
                emit_event_code: decision.emit_event_code.clone(),
                reached_finality_confirmations: decision.reached_finality_confirmations,
            });
            writes.push(crate::wallet_db::HistoryRecord {
                id: updated.id.clone(),
                wallet_id: updated.wallet_id.clone(),
                chain_name: updated.chain_name.clone(),
                tx_hash: updated.transaction_hash.clone(),
                created_at: updated.created_at,
                payload: updated,
            });
        }
        if !writes.is_empty() {
            self.upsert_history_records(writes).await?;
        }
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
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_fetch_for_wallet(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map(|rows| rows.into_iter().map(|row| row.payload).collect())
        .map_err(Into::into)
    }
}

/// What one confirmation poll found.
///
/// Replaces a pair of methods and, inside the success arm, a pair of booleans:
/// `resolved_status_confirmed` and `resolved_status_pending` encoded three
/// states in two flags, so "confirmed and pending" type-checked and meant
/// nothing.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum StatusPollOutcome {
    /// The provider reported the transaction confirmed.
    Confirmed { confirmations: Option<u32> },
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
    async fn audit_fix5_concurrent_history_merges_keep_one_identity_per_wallet() {
        let service = WalletService::new_typed(vec![]).unwrap();
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
                    "id":crate::store::new_transaction_id(), "walletId":format!("wallet-{}",i%2), "walletName":"W", "kind":"send", "status":"confirmed", "chainName":"Ethereum", "transactionHash":"0xshared", "amount":1.0, "symbol":"ETH", "assetName":"Ether", "address":"0xrecipient", "createdAt":1000.0
                })).unwrap();
                barrier.wait().await;
                service.apply_transaction_command(TransactionCommand::Merge { incoming:vec![record.into()], chain_name:"Ethereum".into(), preserve_created_at_sentinel_unix:None }).await.unwrap()
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

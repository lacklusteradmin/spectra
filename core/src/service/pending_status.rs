//! Polling a chain's pending transactions for a final status.
//!
//! How a chain reaches finality is a registry fact, and the three shapes it
//! takes — a UTXO status endpoint, an address history that names confirmed
//! txids, an EVM receipt — were three loops on the front end's side of the
//! boundary. Each one selected the records to poll from its own projection of
//! core's store, asked core whether each was due, fetched, told core the
//! outcome, collected the resolutions and handed them back to be applied: five
//! crossings per transaction. Core owns the store, the schedule and the fetch,
//! so it owns the loop; what comes back is what changed, which is what a front
//! end needs to write an event and a notification.

use crate::registry::{Chain, PendingStatusPoll};
use crate::service::WalletService;
use crate::store::{ResolvedPendingStatus, TransactionStatusChange};
use crate::SpectraBridgeError;

/// The stored records this chain's poll shape tracks.
///
/// A receive confirms on its own where `require_send_kind` says so, and a
/// chain that counts confirmations keeps polling after the first one.
fn tracked(
    records: &[crate::store::persistence_models::CorePersistedTransactionRecord],
    chain: Chain,
    poll: PendingStatusPoll,
) -> Vec<crate::store::persistence_models::CorePersistedTransactionRecord> {
    use crate::store::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};
    let (tracks_finality, require_send_kind) = match poll {
        PendingStatusPoll::Utxo {
            tracks_finality,
            require_send_kind,
        } => (tracks_finality, require_send_kind),
        PendingStatusPoll::HistoryTxids | PendingStatusPoll::EvmReceipt => (false, true),
        PendingStatusPoll::None => return Vec::new(),
    };
    records
        .iter()
        .filter(|record| record.chain_name == chain.chain_display_name())
        .filter(|record| !require_send_kind || record.kind == CoreTransactionKind::Send)
        .filter(|record| {
            record
                .transaction_hash
                .as_deref()
                .is_some_and(|hash| !hash.trim().is_empty())
        })
        .filter(|record| match record.status {
            Some(CoreTransactionStatus::Pending) => true,
            Some(CoreTransactionStatus::Confirmed) => tracks_finality,
            _ => false,
        })
        .cloned()
        .collect()
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Poll one chain's pending transactions and apply what came back.
    ///
    /// Returns what changed: enough for a caller to write an operational event
    /// and send a notification, which is the part that is genuinely a
    /// platform's. A transaction that is not due yet is skipped, a provider
    /// failure is recorded against the poll schedule rather than raised, and a
    /// chain the registry does not poll does nothing.
    pub async fn poll_pending_transactions(
        &self,
        chain_id: String,
    ) -> Result<Vec<TransactionStatusChange>, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain {chain_id:?}")))?;
        let poll = chain.pending_status_poll();
        if matches!(poll, PendingStatusPoll::None) {
            return Ok(Vec::new());
        }
        // Trackers for records nothing polls any more go first: the set is read
        // from the store here rather than computed by a caller and sent over.
        let _ = self.prune_status_trackers().await;

        let records = {
            let stored = self
                .fetch_all_history_records_typed()
                .await
                .unwrap_or_default();
            let all: Vec<_> = stored.into_iter().map(|row| row.payload).collect();
            tracked(&all, chain, poll)
        };
        if records.is_empty() {
            return Ok(Vec::new());
        }

        let due: std::collections::HashSet<String> = self
            .transactions_due_for_status_poll(
                records.iter().map(|record| record.id.clone()).collect(),
            )
            .await
            .into_iter()
            .collect();

        let network = {
            let state = self.app_state().await;
            state.settings.network_chain(chain)
        };
        let network_id = network.str_id().to_string();

        let mut resolutions = Vec::new();
        match poll {
            PendingStatusPoll::Utxo {
                tracks_finality, ..
            } => {
                for record in records.iter().filter(|r| due.contains(&r.id)) {
                    let Some(hash) = record.transaction_hash.clone() else {
                        continue;
                    };
                    match self
                        .fetch_utxo_tx_status_typed(network_id.clone(), hash)
                        .await
                    {
                        Ok(status) => {
                            let confirmations: Option<u32> = tracks_finality
                                .then(|| {
                                    status.confirmations.map(|count| count as u32).or_else(|| {
                                        record.confirmation_count.map(|count| count.max(0) as u32)
                                    })
                                })
                                .flatten();
                            self.record_status_poll(
                                record.id.clone(),
                                if status.confirmed {
                                    crate::service::StatusPollOutcome::Confirmed { confirmations }
                                } else {
                                    crate::service::StatusPollOutcome::Pending
                                },
                            )
                            .await;
                            resolutions.push(ResolvedPendingStatus {
                                id: record.id.clone(),
                                status: if status.confirmed {
                                    "confirmed"
                                } else {
                                    "pending"
                                }
                                .to_string(),
                                confirmations,
                                receipt_block_number: status
                                    .block_height
                                    .map(|height| height as i64),
                                dogecoin_network_fee_doge: None,
                            });
                        }
                        Err(_) => {
                            self.record_status_poll(
                                record.id.clone(),
                                crate::service::StatusPollOutcome::Failed,
                            )
                            .await
                        }
                    }
                }
            }
            PendingStatusPoll::EvmReceipt => {
                for record in records.iter().filter(|r| due.contains(&r.id)) {
                    let Some(hash) = record.transaction_hash.clone() else {
                        continue;
                    };
                    match self.evm_transaction_status(network_id.clone(), hash).await {
                        // A node that answered "no receipt yet" is a pending
                        // poll, not a failed one.
                        Ok(None) => {
                            self.record_status_poll(
                                record.id.clone(),
                                crate::service::StatusPollOutcome::Pending,
                            )
                            .await
                        }
                        Ok(Some(classification)) if !classification.is_confirmed => {
                            self.record_status_poll(
                                record.id.clone(),
                                crate::service::StatusPollOutcome::Pending,
                            )
                            .await
                        }
                        Ok(Some(classification)) => {
                            // A reverted receipt is a failure, not a
                            // confirmation: the history summary the other arm
                            // reads cannot tell the two apart.
                            let status = if classification.is_failed {
                                "failed"
                            } else {
                                "confirmed"
                            };
                            self.record_status_poll(
                                record.id.clone(),
                                crate::service::StatusPollOutcome::Confirmed {
                                    confirmations: None,
                                },
                            )
                            .await;
                            resolutions.push(ResolvedPendingStatus {
                                id: record.id.clone(),
                                status: status.to_string(),
                                confirmations: None,
                                receipt_block_number: classification.block_number,
                                dogecoin_network_fee_doge: None,
                            });
                        }
                        Err(_) => {
                            self.record_status_poll(
                                record.id.clone(),
                                crate::service::StatusPollOutcome::Failed,
                            )
                            .await
                        }
                    }
                }
            }
            PendingStatusPoll::HistoryTxids => {
                // One history read per address, not per transaction: the
                // records are grouped by the wallet's address first.
                let state = self.app_state().await;
                let mut by_address: std::collections::HashMap<String, Vec<&_>> =
                    std::collections::HashMap::new();
                for record in &records {
                    let Some(address) = record
                        .wallet_id
                        .as_deref()
                        .and_then(|wallet_id| {
                            state
                                .wallets
                                .iter()
                                .find(|wallet| wallet.id.eq_ignore_ascii_case(wallet_id))
                        })
                        .and_then(|wallet| wallet.active_address(&state.settings))
                    else {
                        continue;
                    };
                    by_address
                        .entry(address.to_string())
                        .or_default()
                        .push(record);
                }
                for (address, group) in by_address {
                    let confirmed = match self
                        .fetch_history_summary(network_id.clone(), address)
                        .await
                    {
                        Ok(summary) => summary
                            .confirmed_txids
                            .into_iter()
                            .map(|txid| txid.to_lowercase())
                            .collect::<std::collections::HashSet<_>>(),
                        Err(_) => {
                            for record in group {
                                self.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Failed,
                                )
                                .await;
                            }
                            continue;
                        }
                    };
                    for record in group.into_iter().filter(|r| due.contains(&r.id)) {
                        let is_confirmed = record
                            .transaction_hash
                            .as_deref()
                            .is_some_and(|hash| confirmed.contains(&hash.to_lowercase()));
                        self.record_status_poll(
                            record.id.clone(),
                            if is_confirmed {
                                crate::service::StatusPollOutcome::Confirmed {
                                    confirmations: None,
                                }
                            } else {
                                crate::service::StatusPollOutcome::Pending
                            },
                        )
                        .await;
                        resolutions.push(ResolvedPendingStatus {
                            id: record.id.clone(),
                            status: if is_confirmed { "confirmed" } else { "pending" }.to_string(),
                            confirmations: None,
                            receipt_block_number: None,
                            dogecoin_network_fee_doge: None,
                        });
                    }
                }
            }
            PendingStatusPoll::None => {}
        }

        self.apply_resolved_pending_statuses(chain.chain_display_name().to_string(), resolutions)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::persistence_models::CorePersistedTransactionRecord;
    use serde_json::json;

    /// Built from the stored JSON shape so a test says only what it is about.
    fn record(
        id: &str,
        chain: Chain,
        overrides: serde_json::Value,
    ) -> CorePersistedTransactionRecord {
        let mut value = json!({
            "id": id,
            "walletId": "wallet-1",
            "kind": "send",
            "status": "pending",
            "walletName": "Main",
            "assetName": chain.chain_display_name(),
            "symbol": chain.coin_symbol(),
            "chainName": chain.chain_display_name(),
            "amount": 1.0,
            "address": "counterparty",
            "transactionHash": "0xabc",
            "createdAt": 745_200_000.0,
        });
        for (key, patch) in overrides.as_object().expect("overrides object") {
            if patch.is_null() {
                value.as_object_mut().unwrap().remove(key);
            } else {
                value[key] = patch.clone();
            }
        }
        serde_json::from_value(value).expect("stored transaction shape")
    }

    fn ids(records: &[CorePersistedTransactionRecord]) -> Vec<&str> {
        records.iter().map(|record| record.id.as_str()).collect()
    }

    /// What a chain tracks is its poll shape's answer, not a filter each
    /// caller wrote out.
    #[test]
    fn a_shape_decides_which_records_are_polled() {
        let all = vec![
            record("pending-send", Chain::Bitcoin, json!({})),
            record(
                "confirmed-send",
                Chain::Bitcoin,
                json!({"status": "confirmed"}),
            ),
            record(
                "pending-receive",
                Chain::Bitcoin,
                json!({"kind": "receive"}),
            ),
            record("failed-send", Chain::Bitcoin, json!({"status": "failed"})),
            record("no-hash", Chain::Bitcoin, json!({"transactionHash": null})),
            record(
                "blank-hash",
                Chain::Bitcoin,
                json!({"transactionHash": "  "}),
            ),
            record("other-chain", Chain::Litecoin, json!({})),
        ];

        // A send, still pending, with a hash — and only this chain's.
        let plain = tracked(
            &all,
            Chain::Bitcoin,
            PendingStatusPoll::Utxo {
                tracks_finality: false,
                require_send_kind: true,
            },
        );
        assert_eq!(ids(&plain), vec!["pending-send"]);

        // Counting confirmations means polling past the first one.
        let finality = tracked(
            &all,
            Chain::Bitcoin,
            PendingStatusPoll::Utxo {
                tracks_finality: true,
                require_send_kind: true,
            },
        );
        assert_eq!(ids(&finality), vec!["pending-send", "confirmed-send"]);

        // A chain that tracks receives too.
        let receives = tracked(
            &all,
            Chain::Bitcoin,
            PendingStatusPoll::Utxo {
                tracks_finality: false,
                require_send_kind: false,
            },
        );
        assert_eq!(ids(&receives), vec!["pending-send", "pending-receive"]);

        // The receipt and history shapes track pending sends.
        for poll in [
            PendingStatusPoll::EvmReceipt,
            PendingStatusPoll::HistoryTxids,
        ] {
            assert_eq!(
                ids(&tracked(&all, Chain::Bitcoin, poll)),
                vec!["pending-send"]
            );
        }
        assert!(tracked(&all, Chain::Bitcoin, PendingStatusPoll::None).is_empty());
    }

    /// A chain the registry does not poll does nothing, and an unknown one is
    /// refused. Offline: neither reaches a provider.
    #[tokio::test]
    async fn a_chain_that_is_not_polled_does_nothing() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let unpolled = Chain::all()
            .find(|chain| matches!(chain.pending_status_poll(), PendingStatusPoll::None))
            .expect("some chain is not polled");
        assert!(service
            .poll_pending_transactions(unpolled.str_id().to_string())
            .await
            .expect("poll")
            .is_empty());
        assert!(service
            .poll_pending_transactions("not-a-chain".to_string())
            .await
            .is_err());
    }
}

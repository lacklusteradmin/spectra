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
    records
        .iter()
        .filter(|r| r.chain_name == chain.chain_display_name())
        .filter(|r| needs_status_poll(r.kind, r.status, r.transaction_hash.as_deref(), poll))
        .cloned()
        .collect()
}

/// Shared by polling and pruning: a tracker lives as long as its transaction is tracked.
pub(super) fn needs_status_poll(
    kind: crate::store::wallet_domain::CoreTransactionKind,
    status: Option<crate::store::wallet_domain::CoreTransactionStatus>,
    hash: Option<&str>,
    poll: PendingStatusPoll,
) -> bool {
    use crate::store::wallet_domain::{CoreTransactionKind as K, CoreTransactionStatus as S};
    let (finality, sends_only) = match poll {
        PendingStatusPoll::Utxo {
            tracks_finality,
            require_send_kind,
        } => (tracks_finality, require_send_kind),
        PendingStatusPoll::EvmReceipt | PendingStatusPoll::HistoryTxids => (false, true),
        PendingStatusPoll::None => return false,
    };
    hash.is_some_and(|h| !h.trim().is_empty())
        && (!sends_only || kind == K::Send)
        && (status == Some(S::Pending) || (finality && status == Some(S::Confirmed)))
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct PendingMaintenanceFailure {
    pub chain_id: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct PendingMaintenanceResult {
    pub chains: Vec<String>,
    pub changes: Vec<TransactionStatusChange>,
    pub failures: Vec<PendingMaintenanceFailure>,
}

impl WalletService {
    /// The registry decides which stored records still need maintenance.
    pub async fn pending_maintenance_chains(&self) -> Result<Vec<String>, SpectraBridgeError> {
        let rows = self.fetch_all_history_records_typed().await?;
        let mut chains = std::collections::BTreeSet::new();
        for row in rows {
            let r = row.payload;
            if let Some(chain) = Chain::from_display_name(&r.chain_name) {
                if needs_status_poll(
                    r.kind,
                    r.status,
                    r.transaction_hash.as_deref(),
                    chain.pending_status_poll(),
                ) {
                    chains.insert(chain.str_id().to_owned());
                }
            }
        }
        Ok(chains.into_iter().collect())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Run maintenance for the exact networks of stored transactions, independent
    /// of a front end's visible chain catalog. Keep successful changes when a
    /// different chain fails; provider failures retain their per-record backoff.
    pub async fn refresh_pending_transactions(
        &self,
    ) -> Result<PendingMaintenanceResult, SpectraBridgeError> {
        self.prune_status_trackers().await?;
        let chains = self.pending_maintenance_chains().await?;
        let outcomes = futures::future::join_all(
            chains
                .iter()
                .map(|chain| self.poll_pending_transactions(chain.clone())),
        )
        .await;
        let mut changes = Vec::new();
        let mut failures = Vec::new();
        for (chain_id, outcome) in chains.iter().zip(outcomes) {
            match outcome {
                Ok(updated) => changes.extend(updated),
                Err(error) => failures.push(PendingMaintenanceFailure {
                    chain_id: chain_id.clone(),
                    message: error.to_string(),
                }),
            }
        }
        Ok(PendingMaintenanceResult {
            chains,
            changes,
            failures,
        })
    }

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
        self.prune_status_trackers().await?;

        let records = {
            let stored = self.fetch_all_history_records_typed().await?;
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

        // `tracked` selected records for this exact stored network. Settings
        // may have changed since submission and must not redirect old hashes.
        let network = chain;
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
                for record in records.iter().filter(|r| due.contains(&r.id)) {
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
    #[tokio::test]
    async fn maintenance_scope_uses_registry_finality_and_ignores_empty_hashes() {
        let (service, path) = stored_service().await;
        use crate::store::state::{StateCommand, WalletSummary};
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletSummary::single_address(
                    "wallet-1",
                    "W",
                    "Ethereum",
                    "0x1111111111111111111111111111111111111111",
                    None,
                    true,
                ),
            })
            .await
            .unwrap();
        service
            .apply_transaction_command(crate::service::TransactionCommand::Upsert {
                records: vec![
                    record(
                        "eth-confirmed",
                        Chain::Ethereum,
                        json!({"status":"confirmed"}),
                    ),
                    record(
                        "doge-confirmed",
                        Chain::Dogecoin,
                        json!({"status":"confirmed"}),
                    ),
                    record("btc-empty", Chain::Bitcoin, json!({"transactionHash":""})),
                ],
            })
            .await
            .unwrap();
        assert_eq!(
            service.pending_maintenance_chains().await.unwrap(),
            vec!["dogecoin"]
        );
        let reopened = WalletService::new_typed(vec![]).unwrap();
        reopened.open_state(path).await.unwrap();
        assert_eq!(
            reopened.pending_maintenance_chains().await.unwrap(),
            vec!["dogecoin"]
        );
    }

    async fn stored_service() -> (std::sync::Arc<WalletService>, String) {
        let service = WalletService::new_typed(vec![]).unwrap();
        let path = std::env::temp_dir()
            .join(format!(
                "spectra-poll-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        service.open_state(path.clone()).await.unwrap();
        (service, path)
    }

    #[tokio::test]
    async fn pruning_preserves_backoff_for_every_poll_shape() {
        let (service, _) = stored_service().await;
        let chains: Vec<_> = Chain::mainnets()
            .filter(|c| !matches!(c.pending_status_poll(), PendingStatusPoll::None))
            .collect();
        let rows = chains
            .iter()
            .map(|c| {
                crate::wallet_db::history_record_from_payload(record(c.str_id(), *c, json!({})))
            })
            .collect();
        service.upsert_history_records(rows).await.unwrap();
        let ids: Vec<_> = chains.iter().map(|c| c.str_id().to_string()).collect();
        for id in &ids {
            service
                .record_status_poll(id.clone(), crate::service::StatusPollOutcome::Failed)
                .await;
            service
                .record_status_poll(id.clone(), crate::service::StatusPollOutcome::Failed)
                .await;
        }
        service
            .record_status_poll("deleted".into(), crate::service::StatusPollOutcome::Failed)
            .await;
        service.prune_status_trackers().await.unwrap();
        assert!(service
            .transactions_due_for_status_poll(ids.clone())
            .await
            .is_empty());
        let trackers = service.status_trackers.read().await;
        assert!(!trackers.contains_key("deleted"));
        for id in ids {
            assert_eq!(trackers[&id].consecutive_failures, 2, "{id}");
        }
    }

    #[tokio::test]
    async fn consecutive_evm_polls_do_not_repeat_the_request() {
        use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(|request: &Request| {
            let body: serde_json::Value = request.body_json().unwrap();
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0", "id":body["id"],
                "result":if body["method"] == "eth_chainId" { json!("0x1") } else { serde_json::Value::Null }}))
        }).mount(&server).await;
        let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
            chain_id: "ethereum".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let db = std::env::temp_dir().join(format!(
            "spectra-poll-rounds-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(db.to_string_lossy().into_owned())
            .await
            .unwrap();
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(record(
                "pending",
                Chain::Ethereum,
                json!({}),
            ))])
            .await
            .unwrap();
        service
            .poll_pending_transactions("ethereum".into())
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap().len();
        assert!(requests > 0);
        service
            .poll_pending_transactions("ethereum".into())
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), requests);
    }

    #[tokio::test]
    async fn polling_propagates_unopened_and_corrupt_storage() {
        let unopened = WalletService::new_typed(vec![]).unwrap();
        assert!(unopened
            .poll_pending_transactions("ethereum".into())
            .await
            .is_err());
        let (service, path) = stored_service().await;
        assert!(service
            .poll_pending_transactions("ethereum".into())
            .await
            .unwrap()
            .is_empty());
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(record(
                "broken",
                Chain::Ethereum,
                json!({}),
            ))])
            .await
            .unwrap();
        service
            .record_status_poll("broken".into(), crate::service::StatusPollOutcome::Failed)
            .await;
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("UPDATE history_records SET payload = '{}'", [])
            .unwrap();
        assert!(service
            .poll_pending_transactions("ethereum".into())
            .await
            .is_err());
        assert!(service.status_trackers.read().await.contains_key("broken"));
        let count: i64 = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM history_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn owned_pending_maintenance_prunes_even_when_no_network_needs_polling() {
        let (service, _) = stored_service().await;
        service
            .record_status_poll("deleted".into(), crate::service::StatusPollOutcome::Failed)
            .await;
        let result = service.refresh_pending_transactions().await.unwrap();
        assert!(result.chains.is_empty());
        assert!(result.changes.is_empty());
        assert!(result.failures.is_empty());
        assert!(service.status_trackers.read().await.is_empty());
    }

    #[tokio::test]
    async fn owned_pending_maintenance_refuses_unopened_and_corrupt_storage() {
        let unopened = WalletService::new_typed(vec![]).unwrap();
        assert!(unopened.refresh_pending_transactions().await.is_err());
        let (service, path) = stored_service().await;
        service
            .record_status_poll("keep".into(), crate::service::StatusPollOutcome::Failed)
            .await;
        rusqlite::Connection::open(path)
            .unwrap()
            .execute("DROP TABLE history_records", [])
            .unwrap();
        assert!(service.refresh_pending_transactions().await.is_err());
        assert!(service.status_trackers.read().await.contains_key("keep"));
    }

    #[tokio::test]
    async fn audit_fix5_owned_pending_maintenance_uses_recorded_network_after_settings_change() {
        use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};
        let mainnet = MockServer::start().await;
        let sepolia = MockServer::start().await;
        let (service, _) = stored_service().await;
        service
            .update_endpoints_typed(vec![
                crate::service::ChainEndpoints {
                    chain_id: "ethereum".into(),
                    endpoints: vec![mainnet.uri()],
                    api_key: None,
                },
                crate::service::ChainEndpoints {
                    chain_id: "ethereum-sepolia".into(),
                    endpoints: vec![sepolia.uri()],
                    api_key: None,
                },
            ])
            .await
            .unwrap();
        Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":1,"result":{"status":"0x1","blockNumber":"0x7","gasUsed":"0x5208","effectiveGasPrice":"0x1"}}))).expect(1).mount(&sepolia).await;
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(record(
                "sepolia-pending",
                Chain::EthereumSepolia,
                json!({}),
            ))])
            .await
            .unwrap();
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(record(
                "mainnet-backoff",
                Chain::Ethereum,
                json!({}),
            ))])
            .await
            .unwrap();
        for _ in 0..2 {
            service
                .record_status_poll(
                    "mainnet-backoff".into(),
                    crate::service::StatusPollOutcome::Failed,
                )
                .await;
        }
        // Defaults select mainnet; historical Sepolia hashes must stay on Sepolia.
        let result = service.refresh_pending_transactions().await.unwrap();
        assert_eq!(result.chains, vec!["ethereum", "ethereum-sepolia"]);
        assert_eq!(result.changes.len(), 1);
        assert!(result.failures.is_empty());
        assert!(mainnet.received_requests().await.unwrap().is_empty());
        let row = service
            .transactions()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.id == "sepolia-pending")
            .unwrap();
        assert_eq!(
            row.status,
            Some(crate::store::wallet_domain::CoreTransactionStatus::Confirmed)
        );
        sepolia.verify().await;
    }
}

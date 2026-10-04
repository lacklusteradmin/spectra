//! Polling a chain's pending transactions for a final status.
//!
//! How a chain resolves pending transactions is a registry fact, and it takes
//! a UTXO status endpoint, an exact transaction execution result, an EVM
//! receipt or finalized Substrate events. Core owns the store, the schedule and the
//! fetch, so it owns the loop; what comes back is what changed, which is what
//! a front end needs to write an event and a notification.

use crate::SpectraBridgeError;
use crate::registry::{Chain, PendingStatusPoll};
use crate::service::WalletService;
use crate::store::{ResolvedPendingStatus, TransactionStatusChange};

/// The stored records this chain's poll shape tracks.
///
/// A receive confirms on its own where `require_send_kind` says so.
/// Confirmed records never need automatic status polling.
fn tracked(
    records: &[crate::store::persistence_models::CorePersistedTransactionRecord],
    chain: Chain,
    poll: PendingStatusPoll,
) -> Vec<crate::store::persistence_models::CorePersistedTransactionRecord> {
    records
        .iter()
        .filter(|r| r.chain_id == chain)
        .filter(|r| needs_status_poll(r.kind, r.status, r.transaction_hash.as_deref(), poll))
        .cloned()
        .collect()
}

/// Shared by polling and pruning: a tracker lives as long as its transaction is tracked.
pub(super) fn needs_status_poll(
    kind: crate::store::wallet_domain::CoreTransactionKind,
    status: crate::store::wallet_domain::CoreTransactionStatus,
    hash: Option<&str>,
    poll: PendingStatusPoll,
) -> bool {
    use crate::store::wallet_domain::CoreTransactionStatus as S;
    let sends_only = match poll {
        PendingStatusPoll::Utxo { require_send_kind } => require_send_kind,
        PendingStatusPoll::EvmReceipt
        | PendingStatusPoll::TransactionStatus(_)
        | PendingStatusPoll::SubstrateFinality => true,
        PendingStatusPoll::None => return false,
    };
    hash.is_some_and(|h| !h.trim().is_empty())
        && (!sends_only || kind.is_submitted())
        && status == S::Pending
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct PendingMaintenanceFailure {
    pub chain_id: crate::registry::Chain,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct PendingMaintenanceResult {
    pub chains: Vec<Chain>,
    pub changes: Vec<TransactionStatusChange>,
    pub failures: Vec<PendingMaintenanceFailure>,
}

impl WalletService {
    /// The registry decides which stored records still need maintenance.
    pub async fn pending_maintenance_chains(&self) -> Result<Vec<Chain>, SpectraBridgeError> {
        let rows = self.fetch_all_history_records().await?;
        let mut chains = std::collections::BTreeSet::new();
        for row in rows {
            let r = row.payload;
            let chain = r.chain_id;
            if needs_status_poll(
                r.kind,
                r.status,
                r.transaction_hash.as_deref(),
                chain.pending_status_poll(),
            ) {
                chains.insert(chain);
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
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            this.prune_status_trackers().await?;
            let chains = this.pending_maintenance_chains().await?;
            let outcomes = futures::future::join_all(
                chains
                    .iter()
                    .map(|&chain| this.poll_pending_transactions(chain)),
            )
            .await;
            let mut changes = Vec::new();
            let mut failures = Vec::new();
            for (chain_id, outcome) in chains.iter().zip(outcomes) {
                match outcome {
                    Ok(updated) => changes.extend(updated),
                    Err(error) => failures.push(PendingMaintenanceFailure {
                        chain_id: *chain_id,
                        message: error.to_string(),
                    }),
                }
            }
            Ok(PendingMaintenanceResult {
                chains,
                changes,
                failures,
            })
        })
        .await
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
        chain: crate::registry::Chain,
    ) -> Result<Vec<TransactionStatusChange>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let poll = chain.pending_status_poll();
            if matches!(poll, PendingStatusPoll::None) {
                return Ok(Vec::new());
            }
            // Trackers for records nothing polls any more go first: the set is read
            // from the store here rather than computed by a caller and sent over.
            this.prune_status_trackers().await?;

            let records = {
                let stored = this.fetch_all_history_records().await?;
                let all: Vec<_> = stored.into_iter().map(|row| row.payload).collect();
                tracked(&all, chain, poll)
            };
            if records.is_empty() {
                return Ok(Vec::new());
            }

            let due: std::collections::HashSet<String> = this
                .transactions_due_for_status_poll(
                    records.iter().map(|record| record.id.clone()).collect(),
                )
                .await
                .into_iter()
                .collect();

            // `tracked` selected records for this exact stored network. Settings
            // may have changed since submission and must not redirect old hashes.

            let mut resolutions = Vec::new();
            match poll {
                PendingStatusPoll::Utxo { .. } => {
                    for record in records.iter().filter(|r| due.contains(&r.id)) {
                        let Some(hash) = record.transaction_hash.clone() else {
                            continue;
                        };
                        match this.fetch_utxo_tx_status(chain, hash).await {
                            Ok(status) => {
                                // Confirmation depth is provider metadata, not a polling threshold.
                                let confirmations = if status.confirmed {
                                    match status.confirmations.map(u32::try_from).transpose() {
                                        Ok(count) => count,
                                        Err(_) => {
                                            this.record_status_poll(
                                                record.id.clone(),
                                                crate::service::StatusPollOutcome::Failed,
                                            )
                                            .await;
                                            continue;
                                        }
                                    }
                                } else {
                                    None
                                };
                                let block = match status.block_height.map(i64::try_from).transpose()
                                {
                                    Ok(block) => block,
                                    Err(_) => {
                                        this.record_status_poll(
                                            record.id.clone(),
                                            crate::service::StatusPollOutcome::Failed,
                                        )
                                        .await;
                                        continue;
                                    }
                                };
                                this.record_status_poll(
                                    record.id.clone(),
                                    if status.confirmed {
                                        crate::service::StatusPollOutcome::Confirmed
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
                                    receipt_block_number: block,
                                    evm_receipt_cost: None,
                                });
                            }
                            Err(_) => {
                                this.record_status_poll(
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
                        match this.evm_transaction_status(chain, hash).await {
                            // A node that answered "no receipt yet" is a pending
                            // poll, not a failed one.
                            Ok(None) => {
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Pending,
                                )
                                .await
                            }
                            Ok(Some(classification)) if !classification.is_confirmed => {
                                this.record_status_poll(
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
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Confirmed,
                                )
                                .await;
                                resolutions.push(ResolvedPendingStatus {
                                    id: record.id.clone(),
                                    status: status.to_string(),
                                    confirmations: None,
                                    receipt_block_number: classification.block_number,
                                    evm_receipt_cost: classification.cost,
                                });
                            }
                            Err(_) => {
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Failed,
                                )
                                .await
                            }
                        }
                    }
                }
                PendingStatusPoll::SubstrateFinality => {
                    for record in records.iter().filter(|r| due.contains(&r.id)) {
                        let Some(hash) = record.transaction_hash.as_deref() else {
                            continue;
                        };
                        match this.poll_substrate_artifact(chain, &record.id, hash).await {
                            Ok(Some(outcome)) => {
                                let Ok(block) = i64::try_from(outcome.block_number) else {
                                    this.record_status_poll(
                                        record.id.clone(),
                                        crate::service::StatusPollOutcome::Failed,
                                    )
                                    .await;
                                    continue;
                                };
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Confirmed,
                                )
                                .await;
                                resolutions.push(ResolvedPendingStatus {
                                    id: record.id.clone(),
                                    status: if outcome.succeeded {
                                        "confirmed"
                                    } else {
                                        "failed"
                                    }
                                    .into(),
                                    confirmations: None,
                                    receipt_block_number: Some(block),
                                    evm_receipt_cost: None,
                                });
                            }
                            Ok(None) => {
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Pending,
                                )
                                .await
                            }
                            Err(_) => {
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Failed,
                                )
                                .await
                            }
                        }
                    }
                }
                PendingStatusPoll::TransactionStatus(api) => {
                    for record in records.iter().filter(|record| due.contains(&record.id)) {
                        let outcome = this
                            .fetch_pending_transaction_status(chain, api, record)
                            .await;
                        match outcome {
                            Ok(crate::api::transaction_status::TransactionStatus::Pending) => {
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Pending,
                                )
                                .await;
                            }
                            Ok(crate::api::transaction_status::TransactionStatus::Confirmed {
                                succeeded,
                                block,
                            }) => {
                                let Ok(block) = block.map(i64::try_from).transpose() else {
                                    this.record_status_poll(
                                        record.id.clone(),
                                        crate::service::StatusPollOutcome::Failed,
                                    )
                                    .await;
                                    continue;
                                };
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Confirmed,
                                )
                                .await;
                                resolutions.push(ResolvedPendingStatus {
                                    id: record.id.clone(),
                                    status: if succeeded { "confirmed" } else { "failed" }.into(),
                                    confirmations: None,
                                    receipt_block_number: block,
                                    evm_receipt_cost: None,
                                });
                            }
                            Err(_) => {
                                this.record_status_poll(
                                    record.id.clone(),
                                    crate::service::StatusPollOutcome::Failed,
                                )
                                .await;
                            }
                        }
                    }
                }
                PendingStatusPoll::None => {}
            }

            this.apply_polled_pending_statuses(chain, resolutions, Some(records))
                .await
        })
        .await
    }
}

impl WalletService {
    pub(super) async fn fetch_pending_transaction_status(
        &self,
        chain: Chain,
        api: crate::EndpointApi,
        record: &crate::store::persistence_models::CorePersistedTransactionRecord,
    ) -> Result<crate::api::transaction_status::TransactionStatus, SpectraBridgeError> {
        use crate::EndpointApi as Api;
        use std::sync::Arc;
        if chain == Chain::Icp && record.kind.is_staking() {
            let stored = self.load_send_artifact(record.id.clone()).await?;
            stored.validate()?;
            return self.icp_staking_receipts(&stored, None).await;
        }
        let hash = record
            .transaction_hash
            .clone()
            .ok_or_else(|| SpectraBridgeError::failure("Missing transaction hash"))?;
        let endpoints = if chain.endpoint_apis().contains(&api) {
            self.endpoints_for(chain, &[crate::EndpointCapability::Verification])
                .await
        } else {
            Arc::new(
                self.api_endpoints(chain, api, &[crate::EndpointCapability::Verification])
                    .await?,
            )
        };
        if api == Api::ToncenterV3 {
            let stored = self.load_send_artifact(record.id.clone()).await?;
            stored.validate()?;
            if stored.view.chain_id != chain
                || stored.view.transaction_hash.as_deref() != Some(&hash)
            {
                return Err(SpectraBridgeError::failure(
                    "TON status: stored transaction identity mismatch",
                ));
            }
            let crate::send::stages::PreparedPayload::Ton {
                amount,
                seqno,
                jetton,
                ..
            } = &stored.prepared
            else {
                return Err(SpectraBridgeError::failure(
                    "TON status: missing transfer artifact",
                ));
            };
            let raw = |address: &str| -> Result<String, SpectraBridgeError> {
                let address = crate::derivation::ton::parse_ton_address(address)?
                    .for_network(chain.is_testnet())?;
                Ok(format!(
                    "{}:{}",
                    address.workchain,
                    hex::encode(address.account_id)
                ))
            };
            let expected = crate::api::toncenter_v3::TonTransferExpectation {
                owner: raw(&stored.view.sender)?,
                recipient: raw(&stored.request.to_address)?,
                amount: *amount,
                jetton: jetton
                    .as_ref()
                    .map(|transfer| {
                        Ok::<_, SpectraBridgeError>(
                            crate::api::toncenter_v3::TonJettonExpectation {
                                master: raw(&transfer.master)?,
                                source_wallet: raw(&transfer.source_wallet)?,
                                query_id: u64::from(*seqno),
                            },
                        )
                    })
                    .transpose()?,
            };
            return Ok(crate::api::toncenter_v3::ToncenterV3Client::new(endpoints)
                .fetch_transaction_status(chain, &hash, &expected)
                .await?);
        }
        let sender = record.source_address.clone().unwrap_or_default();
        Ok(crate::api::http::race(&endpoints, |endpoint| {
            let hash = hash.clone();
            let sender = sender.clone();
            async move {
                let eps = Arc::new(vec![endpoint.clone()]);
                match api {
                    Api::SolanaJsonRpc => {
                        let client = crate::api::solana_json_rpc::SolanaClient::new(eps);
                        client.verify_network(chain).await?;
                        client.fetch_transaction_status(&hash).await
                    }
                    Api::AptosRest => {
                        let client = crate::api::aptos_rest::AptosClient::new(eps);
                        let expected = chain.aptos_chain_id().ok_or_else(|| {
                            crate::api::error::ApiError::decode("Missing Aptos network identity")
                        })?;
                        if client.fetch_ledger_info().await?.0 != u64::from(expected) {
                            return Err(crate::api::error::ApiError::decode(
                                "Aptos status: wrong network",
                            ));
                        }
                        client.fetch_transaction_status(&hash).await
                    }
                    Api::SuiJsonRpc => {
                        let client = crate::api::sui_json_rpc::SuiClient::new(eps);
                        client.verify_network(chain).await?;
                        client.fetch_transaction_status(&hash).await
                    }
                    Api::NearJsonRpc => {
                        if sender.is_empty() {
                            return Err(crate::api::error::ApiError::decode(
                                "NEAR status: missing stored sender",
                            ));
                        }
                        let client = crate::api::near_json_rpc::NearClient::new(eps);
                        client.verify_network(chain).await?;
                        client.fetch_transaction_status(&hash, &sender).await
                    }
                    Api::XrplJsonRpc => {
                        let client = crate::api::xrpl_json_rpc::XrplClient::new(eps);
                        client.verify_network(chain).await?;
                        client.fetch_transaction_status(&hash).await
                    }
                    Api::Horizon => {
                        let client = crate::api::horizon::HorizonClient::new(eps);
                        client.verify_network(chain).await?;
                        client.fetch_transaction_status(&hash).await
                    }
                    Api::TronHttp => {
                        let client = crate::api::tron_http::TronHttpClient::new(eps);
                        client.verify_network(chain).await?;
                        client.fetch_transaction_status(&hash).await
                    }
                    Api::Koios => {
                        crate::api::koios::KoiosClient::new(eps)
                            .fetch_transaction_status(&hash)
                            .await
                    }
                    Api::IcpRosetta => {
                        let client = crate::api::icp_rosetta::IcpClient::new(eps);
                        client.verify_network().await?;
                        client.fetch_transaction_status(&hash).await
                    }
                    Api::MoneroDaemonRpc => {
                        crate::api::monero_daemon_rpc::fetch_transaction_status(
                            &endpoint, chain, &hash,
                        )
                        .await
                    }
                    _ => Err(crate::api::error::ApiError::decode(
                        "Registry transaction status API is unsupported",
                    )),
                }
            }
        })
        .await?)
    }

    pub(super) async fn poll_substrate_artifact(
        &self,
        chain: Chain,
        id: &str,
        hash: &str,
    ) -> Result<Option<crate::api::substrate_json_rpc::SubstrateFinalizedOutcome>, SpectraBridgeError>
    {
        let mut stored = self.load_send_artifact(id.to_string()).await?;
        let finalized_number = match &stored.prepared {
            crate::send::stages::PreparedPayload::Substrate(prepared) => prepared.finalized_number,
            _ => {
                return Err(SpectraBridgeError::failure(
                    "Missing Substrate transaction artifact",
                ));
            }
        };
        if stored.view.chain_id != chain || stored.view.transaction_hash.as_deref() != Some(hash) {
            return Err(SpectraBridgeError::failure(
                "Substrate transaction identity mismatch",
            ));
        }
        let after = stored
            .substrate_verified_through
            .unwrap_or(finalized_number);
        if after < finalized_number {
            return Err(SpectraBridgeError::failure(
                "Invalid Substrate finality cursor",
            ));
        }
        let endpoints = self
            .endpoints_for(chain, &[crate::EndpointCapability::Verification])
            .await;
        let (outcome, through) = crate::api::http::race(&endpoints, |endpoint| async move {
            crate::api::substrate_json_rpc::SubstrateClient::new(std::sync::Arc::new(vec![
                endpoint,
            ]))
            .finalized_outcome(chain, hash, after, 64)
            .await
        })
        .await?;
        // A found outcome and the history status commit in different writes.
        // Keep its block discoverable until that status is durable.
        if outcome.is_none() && through > after {
            stored.substrate_verified_through = Some(through);
            stored.view.revision = stored
                .view
                .revision
                .checked_add(1)
                .ok_or_else(|| SpectraBridgeError::failure("Transaction revision overflow"))?;
            // Status maintenance follows the artifact's stored network even
            // when the wallet's selected network has changed.
            let _writer = self.state_writer.lock().await;
            let db = self.bound_database().await?;
            tokio::task::spawn_blocking(move || crate::wallet_db::send_save(&db, &stored, &[]))
                .await??;
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::persistence_models::CorePersistedTransactionRecord;
    use serde_json::json;

    #[test]
    fn all_staking_submissions_remain_eligible_for_pending_verification() {
        use crate::store::wallet_domain::{CoreTransactionKind as K, CoreTransactionStatus as S};
        for kind in [K::Stake, K::Unstake, K::Withdraw, K::ClaimRewards] {
            assert!(needs_status_poll(
                kind,
                S::Pending,
                Some("hash"),
                PendingStatusPoll::SubstrateFinality
            ));
            assert!(!needs_status_poll(
                kind,
                S::Confirmed,
                Some("hash"),
                PendingStatusPoll::SubstrateFinality
            ));
            assert!(!needs_status_poll(
                kind,
                S::Pending,
                None,
                PendingStatusPoll::SubstrateFinality
            ));
        }
        assert!(!needs_status_poll(
            K::Receive,
            S::Pending,
            Some("hash"),
            PendingStatusPoll::SubstrateFinality
        ));
    }

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
            "assetDisplayName": chain.str_id(),
            "symbol": chain.coin_symbol(),
            "chainId": chain.str_id(),
            "amount": "1",
            "address": "counterparty",
            "transactionHash": "0xabc",
            "createdAtUnix": 745_200_000.0,
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
                require_send_kind: true,
            },
        );
        assert_eq!(ids(&plain), vec!["pending-send"]);

        // A chain that tracks receives too.
        let receives = tracked(
            &all,
            Chain::Bitcoin,
            PendingStatusPoll::Utxo {
                require_send_kind: false,
            },
        );
        assert_eq!(ids(&receives), vec!["pending-send", "pending-receive"]);

        // Execution result shapes track pending sends.
        for poll in [
            PendingStatusPoll::EvmReceipt,
            PendingStatusPoll::TransactionStatus(crate::EndpointApi::SolanaJsonRpc),
        ] {
            assert_eq!(
                ids(&tracked(&all, Chain::Bitcoin, poll)),
                vec!["pending-send"]
            );
        }
        assert!(tracked(&all, Chain::Bitcoin, PendingStatusPoll::None).is_empty());
    }

    #[test]
    fn every_send_protocol_has_confirmation_maintenance() {
        for chain in Chain::all() {
            assert!(
                !matches!(chain.pending_status_poll(), PendingStatusPoll::None),
                "{chain}"
            );
        }
    }
    #[tokio::test]
    async fn maintenance_skips_confirmed_records_after_reopening_and_ignores_empty_hashes() {
        let (service, path) = stored_service().await;
        use crate::store::state::{StateCommand, WalletState};
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    "wallet-1",
                    "W",
                    crate::registry::Chain::Ethereum,
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
        assert!(
            service
                .pending_maintenance_chains()
                .await
                .unwrap()
                .is_empty()
        );
        let reopened = WalletService::new(vec![]).unwrap();
        reopened.open_state(path).await.unwrap();
        assert!(
            reopened
                .pending_maintenance_chains()
                .await
                .unwrap()
                .is_empty()
        );
    }

    async fn stored_service() -> (std::sync::Arc<WalletService>, String) {
        let service = WalletService::new(vec![]).unwrap();
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
        assert!(
            service
                .transactions_due_for_status_poll(ids.clone())
                .await
                .is_empty()
        );
        let trackers = service.status_trackers.read().await;
        assert!(!trackers.contains_key("deleted"));
        for id in ids {
            assert_eq!(trackers[&id].consecutive_failures, 2, "{id}");
        }
    }

    #[tokio::test]
    async fn six_failed_status_reads_never_fail_an_old_transaction() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};
        let server = MockServer::start().await;
        Mock::given(body_partial_json(json!({"method":"getGenesisHash"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":1,"result":Chain::Solana.solana_genesis_hash().unwrap()})))
            .mount(&server).await;
        Mock::given(body_partial_json(json!({"method":"getSignatureStatuses"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"temporarily unavailable"}})))
            .expect(6).mount(&server).await;
        let service = WalletService::new(vec![crate::service::ChainEndpoints {
            chain_id: Chain::Solana,
            capabilities: crate::EndpointCapability::ALL.to_vec(),
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "spectra-status-failures-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into_owned())
            .await
            .unwrap();
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(record(
                "old",
                Chain::Solana,
                json!({"transactionHash":"1".repeat(64),"createdAtUnix":0.0}),
            ))])
            .await
            .unwrap();
        for _ in 0..6 {
            if let Some(tracker) = service.status_trackers.write().await.get_mut("old") {
                tracker.next_check_at_unix = 0.0;
            }
            assert!(
                service
                    .poll_pending_transactions(Chain::Solana)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        let stored = service.transaction("old".into()).await.unwrap().unwrap();
        assert_eq!(
            stored.status,
            crate::store::wallet_domain::CoreTransactionStatus::Pending
        );
        assert!(stored.failure_reason.is_none());
        let tracker = service.status_trackers.read().await["old"].clone();
        assert_eq!(tracker.consecutive_failures, 6);
        assert!(!tracker.polling_complete);
        assert!(
            service
                .transactions_due_for_status_poll(vec!["old".into()])
                .await
                .is_empty()
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn overflowing_block_keeps_polling_and_does_not_drop_other_resolutions() {
        use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|request: &Request| {
                let body: serde_json::Value = request.body_json().unwrap();
                let result = match body["method"].as_str().unwrap() {
                    "getGenesisHash" => json!(Chain::Solana.solana_genesis_hash().unwrap()),
                    "getSignatureStatuses" => json!({"value":[{
                        "confirmationStatus":"finalized", "err":null,
                        "slot":if body["params"][0][0] == "1".repeat(64) {
                            u64::MAX
                        } else {
                            17
                        }
                    }]}),
                    method => panic!("Unexpected status method: {method}"),
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0", "id":body["id"], "result":result}))
            })
            .expect(4)
            .mount(&server)
            .await;
        let service = WalletService::new(vec![crate::service::ChainEndpoints {
            chain_id: Chain::Solana,
            capabilities: crate::EndpointCapability::ALL.to_vec(),
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "spectra-overflow-status-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into_owned())
            .await
            .unwrap();
        service
            .upsert_history_records(
                [
                    ("overflow", "1".repeat(64)),
                    ("healthy", bs58::encode([1u8; 64]).into_string()),
                ]
                .into_iter()
                .map(|(id, hash)| {
                    crate::wallet_db::history_record_from_payload(record(
                        id,
                        Chain::Solana,
                        json!({"transactionHash":hash}),
                    ))
                })
                .collect(),
            )
            .await
            .unwrap();
        let changes = service
            .poll_pending_transactions(Chain::Solana)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].id, "healthy");
        let overflow = service
            .transaction("overflow".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            overflow.status,
            crate::store::wallet_domain::CoreTransactionStatus::Pending
        );
        assert!(overflow.receipt_block_number.is_none());
        assert!(overflow.failure_reason.is_none());
        let healthy = service
            .transaction("healthy".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            healthy.status,
            crate::store::wallet_domain::CoreTransactionStatus::Confirmed
        );
        assert_eq!(healthy.receipt_block_number, Some(17));
        let trackers = service.status_trackers.read().await;
        assert_eq!(trackers["overflow"].consecutive_failures, 1);
        assert!(!trackers["overflow"].polling_complete);
        assert!(trackers["healthy"].polling_complete);
        server.verify().await;
    }

    #[tokio::test]
    async fn consecutive_evm_polls_do_not_repeat_the_request() {
        use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(|request: &Request| {
            let body: serde_json::Value = request.body_json().unwrap();
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0", "id":body["id"],
                "result":if body["method"] == "eth_chainId" { json!("0x1") } else { serde_json::Value::Null }}))
        }).mount(&server).await;
        let service = WalletService::new(vec![crate::service::ChainEndpoints {
            capabilities: crate::EndpointCapability::ALL.to_vec(),
            chain_id: crate::registry::Chain::Ethereum,
            endpoints: vec![server.uri()],
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
            .poll_pending_transactions(crate::registry::Chain::Ethereum)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap().len();
        assert!(requests > 0);
        service
            .poll_pending_transactions(crate::registry::Chain::Ethereum)
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), requests);
    }

    #[tokio::test]
    async fn polling_propagates_unopened_and_corrupt_storage() {
        let unopened = WalletService::new(vec![]).unwrap();
        assert!(
            unopened
                .poll_pending_transactions(crate::registry::Chain::Ethereum)
                .await
                .is_err()
        );
        let (service, path) = stored_service().await;
        assert!(
            service
                .poll_pending_transactions(crate::registry::Chain::Ethereum)
                .await
                .unwrap()
                .is_empty()
        );
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
            .execute(
                "UPDATE history_records SET payload = json_set(payload, '$.amount', json('[]'))",
                [],
            )
            .unwrap();
        assert!(
            service
                .poll_pending_transactions(crate::registry::Chain::Ethereum)
                .await
                .is_err()
        );
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
        let unopened = WalletService::new(vec![]).unwrap();
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
    async fn owned_pending_maintenance_uses_recorded_network_after_settings_change() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
        let mainnet = MockServer::start().await;
        let sepolia = MockServer::start().await;
        let (service, _) = stored_service().await;
        service
            .update_endpoints(vec![
                crate::service::ChainEndpoints {
                    capabilities: crate::EndpointCapability::ALL.to_vec(),
                    chain_id: crate::registry::Chain::Ethereum,
                    endpoints: vec![mainnet.uri()],
                },
                crate::service::ChainEndpoints {
                    capabilities: crate::EndpointCapability::ALL.to_vec(),
                    chain_id: crate::registry::Chain::EthereumSepolia,
                    endpoints: vec![sepolia.uri()],
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
        assert_eq!(result.chains, vec![Chain::Ethereum, Chain::EthereumSepolia]);
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
            crate::store::wallet_domain::CoreTransactionStatus::Confirmed
        );
        // The receipt's cost reaches the record: 0x5208 gas at 1 wei is
        // 21000 wei, in Sepolia ETH's eighteen places.
        assert_eq!(row.receipt_gas_used.as_deref(), Some("21000"));
        assert_eq!(
            row.receipt_effective_gas_price_gwei,
            Some("0.000000001".into())
        );
        assert_eq!(
            row.receipt_network_fee.as_deref(),
            Some("0.000000000000021")
        );
        assert_eq!(row.receipt_block_number, Some(7));
        sepolia.verify().await;
    }
}

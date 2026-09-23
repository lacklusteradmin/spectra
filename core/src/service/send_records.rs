//! Outgoing records are written before submission and completed by core.
use super::*;
use crate::store::persistence_models::CorePersistedTransactionRecord;
use crate::store::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};

impl WalletService {
    pub(super) async fn begin_send_record(
        &self,
        chain: Chain,
        request: &crate::send::SendExecutionRequest,
        source: &str,
    ) -> Result<CorePersistedTransactionRecord, SpectraBridgeError> {
        self.bound_database().await?;
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == request.wallet_id)
            .ok_or("wallet removed before submission")?;
        let token = request.contract_address.as_ref().and_then(|contract| {
            state.token_preferences.iter().find(|p| {
                p.hosting_chain().is_some_and(|h| {
                    h.chain_name() == chain.mainnet_counterpart().chain_display_name()
                }) && crate::tokens::normalize_token_identifier(
                    Some(p.token.contract.clone()),
                    chain.chain_display_name().into(),
                ) == crate::tokens::normalize_token_identifier(
                    Some(contract.clone()),
                    chain.chain_display_name().into(),
                )
            })
        });
        let symbol = token.map(|p| p.token.symbol.as_str()).unwrap_or_else(|| {
            request
                .contract_address
                .as_deref()
                .unwrap_or(chain.coin_symbol())
        });
        let deployment_id =
            crate::tokens::history_deployment(chain, request.contract_address.as_deref())
                .ok_or("token identifier missing")?;
        let record: CorePersistedTransactionRecord = serde_json::from_value(json!({
            "deploymentId": deployment_id,
            "id": crate::store::new_transaction_id(), "walletId": wallet.id, "kind": "send", "status": "pending",
            "walletName": wallet.name, "assetDisplayName": token.map(|p| p.token.name.as_str()).unwrap_or(symbol), "symbol": symbol,
            "chainName": chain.chain_display_name(), "amount": request.amount_str.parse::<f64>().map_err(|_| "invalid amount")?,
            "address": request.to_address, "sourceAddress": source,
            "failureReason": "Submission outcome unknown; check network status before sending again.",
            "createdAtUnix": crate::store::now_unix()
        }))?;
        // This is only a draft. Signing failures must not leave pending rows.
        Ok(record)
    }
    pub(super) async fn save_send_record(
        &self,
        record: CorePersistedTransactionRecord,
    ) -> Result<(), SpectraBridgeError> {
        self.save_prepared_send_record(record, false).await
    }

    pub(super) async fn save_prepared_send_record(
        &self,
        record: CorePersistedTransactionRecord,
        reserve_nonce: bool,
    ) -> Result<(), SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let state = service.app_state().await;
            if !state
                .wallets
                .iter()
                .any(|w| Some(&w.id) == record.wallet_id.as_ref())
            {
                return Err("wallet removed during submission".into());
            }
            let database = service.bound_database().await?;
            tokio::task::spawn_blocking(move || {
                crate::wallet_db::history_save_send_progress(&database, &record, reserve_nonce)
            })
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))??;
            Ok(())
        })
        .await
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The stored chain and payload determine what is rebroadcast.
    pub async fn rebroadcast_transaction(
        &self,
        transaction_id: String,
    ) -> Result<String, SpectraBridgeError> {
        let service = self.clone();
        tokio::spawn(async move { service.rebroadcast_stored(transaction_id).await })
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
    }
}
impl WalletService {
    async fn rebroadcast_stored(
        &self,
        transaction_id: String,
    ) -> Result<String, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let id = transaction_id.clone();
        if tokio::task::spawn_blocking(move || crate::wallet_db::send_exists(&db, &id))
            .await
            .map_err(|e| e.to_string())??
        {
            let stored = self.load_send_artifact(transaction_id.clone()).await?;
            if stored.view.selected_endpoints.is_empty() {
                return Err(
                    "Select broadcast endpoints before submitting a signed transaction".into(),
                );
            }
            let previous = stored.view.attempts.len();
            let artifact = self
                .broadcast_send(transaction_id, stored.view.selected_endpoints)
                .await?;
            return artifact.attempts[previous..]
                .iter()
                .find(|a| a.outcome == crate::send::stages::SubmissionOutcome::Accepted)
                .and_then(|a| a.transaction_hash.clone())
                .ok_or_else(|| {
                    "Submission was not accepted; inspect per-endpoint results before retrying"
                        .into()
                });
        }
        let mut record = self
            .fetch_all_history_records()
            .await?
            .into_iter()
            .find(|r| r.id.eq_ignore_ascii_case(&transaction_id))
            .ok_or("transaction not found")?
            .payload;
        let (chain, payload, field) = rebroadcast_input(&record)?;
        // Store an uncertain outcome before network I/O; errors never pretend a send happened.
        record.failure_reason =
            Some("Rebroadcast outcome unknown; check network status before retrying.".into());
        self.save_send_record(record.clone()).await?;
        let hash = self
            .broadcast_raw_extract(chain.str_id().into(), payload, field)
            .await?;
        if hash.trim().is_empty() {
            return Err("node returned no transaction identifier".into());
        }
        record.transaction_hash = Some(hash.clone());
        record.status = CoreTransactionStatus::Pending;
        record.failure_reason = None;
        self.save_send_record(record).await?;
        Ok(hash)
    }
}

pub(super) fn rebroadcast_input(
    record: &CorePersistedTransactionRecord,
) -> Result<(Chain, String, String), SpectraBridgeError> {
    if record.kind != CoreTransactionKind::Send {
        return Err("only sends can be rebroadcast".into());
    }
    if record.status == CoreTransactionStatus::Confirmed {
        return Err("transaction already confirmed".into());
    }
    let chain = Chain::from_display_name(&record.chain_name).ok_or("unknown transaction chain")?;
    let payload = record
        .signed_transaction_payload
        .as_ref()
        .ok_or("signed payload was not saved")?;
    let format = record
        .signed_transaction_payload_format
        .as_deref()
        .ok_or("signed payload format missing")?;
    let (payload, field) = if format == "core.submission_json" {
        let prepared: crate::send::payload::PreparedSubmission = serde_json::from_str(payload)?;
        (prepared.payload, prepared.result_field)
    } else if chain.is_evm() {
        if !["evm.raw_hex", "evm.rust_json", "ethereum.rust_json"].contains(&format) {
            return Err("payload does not match transaction chain".into());
        }
        let raw = if format != "evm.raw_hex" {
            crate::send::preview_decode::extract_json_string_field(
                payload.clone(),
                "raw_tx_hex".into(),
            )
        } else {
            payload.clone()
        };
        if raw.is_empty() {
            return Err("empty signed payload".into());
        }
        (raw, "txid".to_string())
    } else {
        let prepared =
            crate::send::flow::rebroadcast_prepare_payload(format.into(), payload.clone())?;
        if Chain::from_str_id(&prepared.chain_id) != Some(chain.mainnet_counterpart()) {
            return Err("payload does not match transaction chain".into());
        }
        (prepared.broadcast_payload, prepared.result_field)
    };
    if payload.trim().is_empty() || field.trim().is_empty() {
        return Err("empty signed payload or result field".into());
    }
    Ok((chain, payload, field))
}

// Share a lock even across service instances using the same database. Weak entries
// avoid retaining an unbounded list of wallet addresses after operations finish.
static SEND_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(Default::default);

impl WalletService {
    pub(super) async fn lock_sender(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, SpectraBridgeError> {
        let database = self.bound_database().await?;
        let key = format!(
            "{}|{}|{}",
            database.path(),
            chain.str_id(),
            if chain.is_evm() {
                address.to_lowercase()
            } else {
                address.to_owned()
            }
        );
        let lock = {
            let mut locks = SEND_LOCKS.lock().map_err(|_| "send lock poisoned")?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        };
        Ok(lock.lock_owned().await)
    }

    pub(super) async fn next_send_nonce(
        &self,
        chain: Chain,
        source: &str,
    ) -> Result<u64, SpectraBridgeError> {
        let client = EvmClient::new(
            self.endpoints_for(chain.str_id()).await,
            chain.evm_chain_id()?,
        );
        let mut next = client.fetch_nonce(source).await?;
        for row in self.fetch_all_history_records().await? {
            let r = row.payload;
            if r.chain_name == chain.chain_display_name()
                && r.source_address
                    .as_deref()
                    .is_some_and(|a| a.eq_ignore_ascii_case(source))
                && r.kind == CoreTransactionKind::Send
                && r.status == CoreTransactionStatus::Pending
            {
                if let Some(nonce) = r.nonce {
                    let nonce = u64::try_from(nonce).map_err(|_| "invalid stored EVM nonce")?;
                    next = next.max(nonce.checked_add(1).ok_or("EVM nonce exhausted")?);
                }
            }
        }
        let db = self.bound_database().await?;
        let artifacts = tokio::task::spawn_blocking(move || crate::wallet_db::send_list(&db))
            .await
            .map_err(|e| e.to_string())??;
        for artifact in artifacts {
            if artifact.view.chain_id == chain.str_id()
                && artifact.view.sender.eq_ignore_ascii_case(source)
                && artifact.view.stage == crate::send::stages::SendStage::Signed
            {
                if let crate::send::stages::PreparedPayload::Evm(p) = artifact.prepared {
                    next = next.max(p.nonce.checked_add(1).ok_or("EVM nonce exhausted")?);
                }
            }
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::state::WalletState;
    use wiremock::{
        matchers::{body_partial_json, method},
        Mock, MockServer, ResponseTemplate,
    };
    #[tokio::test]
    async fn stored_rebroadcast_uses_recorded_network_and_requires_node_identifier() {
        let server = MockServer::start().await;
        let service = WalletService::new(vec![ChainEndpoints {
            chain_id: "ethereum-sepolia".into(),
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "rebroadcast-owned-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    "w",
                    "W",
                    "Ethereum",
                    "0x1111111111111111111111111111111111111111",
                    None,
                    true,
                ),
            })
            .await
            .unwrap();
        let mut record: CorePersistedTransactionRecord = serde_json::from_value(json!({
            "id": crate::store::new_transaction_id().to_uppercase(), "walletId": "w", "kind": "send", "status": "pending", "walletName": "W", "assetDisplayName": "Ether", "symbol": "ETH", "chainName": "Ethereum Sepolia", "amount": 1.0, "address": "0x2222222222222222222222222222222222222222", "createdAtUnix": 1000.0,
            "signedTransactionPayload": "0xdeadbeef", "signedTransactionPayloadFormat": "evm.raw_hex"
        })).unwrap();
        service.save_send_record(record.clone()).await.unwrap();
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({"method":"eth_sendRawTransaction","params":["0xdeadbeef"]}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":"0xaccepted"})),
            )
            .mount(&server)
            .await;
        assert_eq!(
            service
                .rebroadcast_transaction(record.id.clone())
                .await
                .unwrap(),
            "0xaccepted"
        );
        let stored = service
            .fetch_all_history_records()
            .await
            .unwrap()
            .remove(0)
            .payload;
        assert_eq!(stored.transaction_hash.as_deref(), Some("0xaccepted"));
        assert!(stored.failure_reason.is_none());
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":""})),
            )
            .mount(&server)
            .await;
        assert!(service
            .rebroadcast_transaction(record.id.clone())
            .await
            .is_err());
        assert!(service.fetch_all_history_records().await.unwrap()[0]
            .payload
            .failure_reason
            .is_some());
        record.status = CoreTransactionStatus::Confirmed;
        service.save_send_record(record.clone()).await.unwrap();
        server.reset().await;
        let mut late = record.clone();
        late.status = CoreTransactionStatus::Pending;
        late.failure_reason = Some("late result".into());
        service.save_send_record(late).await.unwrap();
        assert_eq!(
            service.fetch_all_history_records().await.unwrap()[0]
                .payload
                .status,
            CoreTransactionStatus::Confirmed
        );
        assert!(service.rebroadcast_transaction(record.id).await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

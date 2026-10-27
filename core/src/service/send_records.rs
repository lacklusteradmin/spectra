//! Outgoing records are written before submission and completed by core.
use super::*;
use crate::store::persistence_models::TransactionRecord;
use crate::store::wallet_domain::TransactionStatus;

/// The symbol and name a send's asset is shown by: the chain's coin, or the
/// known token at `contract`. A contract no known token claims is named by
/// the contract itself rather than by a guess.
pub(super) fn send_asset_names(
    state: &crate::store::state::ResidentState,
    chain: Chain,
    contract: Option<&str>,
) -> (String, String) {
    let Some(contract) = contract else {
        return (chain.coin_symbol().into(), chain.coin_symbol().into());
    };
    let wanted = crate::catalog::tokens::normalize_token_identifier(Some(contract.into()), chain);
    state
        .token_preferences
        .iter()
        .find(|p| {
            p.hosting_chain() == Some(chain)
                && crate::catalog::tokens::normalize_token_identifier(
                    Some(p.token.contract.clone()),
                    chain,
                ) == wanted
        })
        .map(|p| (p.token.symbol.clone(), p.token.name.clone()))
        .unwrap_or_else(|| (contract.into(), contract.into()))
}

impl WalletService {
    pub(super) async fn begin_send_record(
        &self,
        chain: Chain,
        request: &crate::send::SendExecutionRequest,
        source: &str,
    ) -> Result<TransactionRecord, SpectraBridgeError> {
        self.bound_database().await?;
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == request.wallet_id)
            .ok_or_else(|| SpectraBridgeError::failure("wallet removed before submission"))?;
        let (symbol, display_name) =
            send_asset_names(&state, chain, request.contract_address.as_deref());
        let deployment_id = match request.contract_address.as_deref() {
            None => crate::catalog::tokens::deployment_id_for(chain, None),
            Some(identifier) => crate::catalog::tokens::resolve_token_standard(
                chain,
                request.token_standard,
                identifier,
            )
            .ok()
            .and_then(|standard| {
                crate::catalog::tokens::protocol_deployment_id(chain, standard, identifier)
            }),
        }
        .ok_or_else(|| SpectraBridgeError::failure("token identifier missing"))?;
        let record: TransactionRecord = serde_json::from_value(json!({
            "deploymentId": deployment_id,
            "id": crate::store::new_transaction_id(), "walletId": wallet.id, "kind": "send", "status": "pending",
            "walletName": wallet.name, "assetDisplayName": display_name, "symbol": symbol,
            "chainId": chain.str_id(), "amount": crate::amount::decimal::canonical(&request.amount_str).ok_or_else(|| SpectraBridgeError::failure("invalid amount"))?,
            "address": request.to_address, "sourceAddress": source,
            "failureReason": {"kind": "submissionOutcomeUnknown"},
            "createdAtUnix": crate::store::now_unix()
        }))?;
        // This is only a draft. Signing failures must not leave pending rows.
        Ok(record)
    }
    pub(super) async fn save_send_record(
        &self,
        record: TransactionRecord,
    ) -> Result<(), SpectraBridgeError> {
        self.save_prepared_send_record(record, false).await
    }

    pub(super) async fn save_prepared_send_record(
        &self,
        record: TransactionRecord,
        reserve_nonce: bool,
    ) -> Result<(), SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let state = service.app_state().await;
            if !state
                .wallets
                .iter()
                .any(|w| Some(&w.id) == record.wallet_id.as_ref())
            {
                return Err(SpectraBridgeError::failure(
                    "wallet removed during submission",
                ));
            }
            let database = service.bound_database().await?;
            tokio::task::spawn_blocking(move || {
                crate::wallet_db::history_save_send_progress(&database, &record, reserve_nonce)
            })
            .await
            .map_err(SpectraBridgeError::failure)??;
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
        let this = self.clone();
        crate::service::worker::run(async move {
            let this = &this;
            let service = this.clone();
            tokio::spawn(async move { service.rebroadcast_stored(transaction_id).await })
                .await
                .map_err(SpectraBridgeError::failure)?
        })
        .await
    }
}
impl WalletService {
    /// A send made here goes out again as it was signed, through its stored
    /// transaction, to the endpoints it was submitted to.
    async fn rebroadcast_stored(
        &self,
        transaction_id: String,
    ) -> Result<String, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let id = transaction_id.clone();
        let (record, signed_here) = tokio::task::spawn_blocking(move || {
            Ok::<_, crate::wallet_db::error::DbError>((
                crate::wallet_db::history_find(&db, &id)?,
                crate::wallet_db::send_exists(&db, &id)?,
            ))
        })
        .await??;
        let record = record.ok_or_else(|| SpectraBridgeError::failure("transaction not found"))?;
        if let Some(reason) = rebroadcast_unavailable(&record, signed_here) {
            return Err(SpectraBridgeError::failure(reason));
        }
        let stored = self.load_send_artifact(transaction_id.clone()).await?;
        if stored.view.selected_endpoints.is_empty() {
            return Err(SpectraBridgeError::failure(
                "Select broadcast endpoints before submitting a signed transaction",
            ));
        }
        let previous = stored.view.attempts.len();
        let artifact = self
            .broadcast_send(transaction_id, stored.view.selected_endpoints)
            .await?;
        artifact.attempts[previous..]
            .iter()
            .find(|a| a.outcome == crate::send::stages::SubmissionOutcome::Accepted)
            .and_then(|a| a.transaction_hash.clone())
            .ok_or_else(|| {
                SpectraBridgeError::failure(
                    "Submission was not accepted; inspect per-endpoint results before retrying",
                )
            })
    }
}

/// Why `record` cannot go out again, or `None` when it can: an unconfirmed
/// send whose signed transaction this device stored (`signed_here`).
pub(super) fn rebroadcast_unavailable(
    record: &TransactionRecord,
    signed_here: bool,
) -> Option<&'static str> {
    if !record.kind.is_submitted() {
        Some("only sends can be rebroadcast")
    } else if record.status == TransactionStatus::Confirmed {
        Some("transaction already confirmed")
    } else if !signed_here {
        Some("signed payload was not saved")
    } else {
        None
    }
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
            let mut locks = SEND_LOCKS
                .lock()
                .map_err(|_| SpectraBridgeError::failure("send lock poisoned"))?;
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
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
            chain.evm_chain_id()?,
        );
        let mut next = client.fetch_nonce(source).await?;
        let db = self.bound_database().await?;
        let sender = source.to_owned();
        let rows = tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_pending_for_sender(&db, chain, &sender)
        })
        .await??;
        for r in rows {
            if r.chain_id == chain
                && r.source_address
                    .as_deref()
                    .is_some_and(|a| a.eq_ignore_ascii_case(source))
                && r.kind.is_submitted()
                && r.status == TransactionStatus::Pending
                && let Some(nonce) = r.nonce
            {
                let nonce = u64::try_from(nonce)
                    .map_err(|_| SpectraBridgeError::failure("invalid stored EVM nonce"))?;
                next = next.max(
                    nonce
                        .checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::failure("EVM nonce exhausted"))?,
                );
            }
        }
        let db = self.bound_database().await?;
        let sender = source.to_owned();
        let artifacts = tokio::task::spawn_blocking(move || {
            crate::wallet_db::signed_sends_for_sender(&db, chain, &sender)
        })
        .await??;
        for artifact in artifacts {
            if artifact.view.chain_id == chain
                && artifact.view.sender.eq_ignore_ascii_case(source)
                && artifact.view.stage == crate::send::stages::SendStage::Signed
                && let crate::send::stages::PreparedPayload::Evm(p) = artifact.prepared
            {
                next = next.max(
                    p.nonce
                        .checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::failure("EVM nonce exhausted"))?,
                );
            }
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::state::WalletState;
    use wiremock::MockServer;
    /// A send this device did not sign has nothing to submit again: it is
    /// refused before any endpoint is asked, as is a confirmed one.
    #[tokio::test]
    async fn only_a_send_signed_here_is_rebroadcast() {
        let server = MockServer::start().await;
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: crate::registry::Chain::EthereumSepolia,
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
                    crate::registry::Chain::Ethereum,
                    "0x1111111111111111111111111111111111111111",
                    None,
                    true,
                ),
            })
            .await
            .unwrap();
        let mut record: TransactionRecord = serde_json::from_value(json!({
            "id": crate::store::new_transaction_id(), "walletId": "w", "kind": "send", "status": "pending", "walletName": "W", "assetDisplayName": "Ether", "symbol": "ETH", "chainId": "ethereum-sepolia", "amount": "1", "address": "0x2222222222222222222222222222222222222222", "createdAtUnix": 1000.0
        })).unwrap();
        service.save_send_record(record.clone()).await.unwrap();
        assert_eq!(
            service
                .rebroadcast_transaction(record.id.clone())
                .await
                .unwrap_err()
                .to_string(),
            "signed payload was not saved"
        );
        assert_eq!(
            service
                .transaction(record.id.clone())
                .await
                .unwrap()
                .unwrap()
                .actions
                .rebroadcast_unavailable_reason
                .as_deref(),
            Some("signed payload was not saved")
        );
        record.status = TransactionStatus::Confirmed;
        service.save_send_record(record.clone()).await.unwrap();
        assert!(service.rebroadcast_transaction(record.id).await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// Only an unconfirmed send whose signed transaction is stored here goes
    /// out again.
    #[test]
    fn only_an_unconfirmed_send_signed_here_is_rebroadcast() {
        let record = |kind: &str, status: &str| -> TransactionRecord {
            serde_json::from_value(json!({
                "id": "send", "walletId": "w", "kind": kind, "status": status,
                "walletName": "W", "assetDisplayName": "Ether", "symbol": "ETH",
                "chainId": "ethereum", "amount": "1", "address": "recipient",
                "transactionHash": format!("0x{}", "ab".repeat(32)), "createdAtUnix": 1000.0
            }))
            .unwrap()
        };
        assert_eq!(
            rebroadcast_unavailable(&record("send", "pending"), true),
            None
        );
        assert_eq!(
            rebroadcast_unavailable(&record("send", "failed"), true),
            None
        );
        for (kind, status, signed_here, refusal) in [
            ("send", "pending", false, "signed payload was not saved"),
            ("send", "confirmed", true, "transaction already confirmed"),
            ("receive", "pending", true, "only sends can be rebroadcast"),
        ] {
            assert_eq!(
                rebroadcast_unavailable(&record(kind, status), signed_here),
                Some(refusal)
            );
        }
    }
}

#[cfg(test)]
mod send_asset_name_tests {
    use super::*;

    #[test]
    fn a_send_asset_is_named_by_its_coin_its_token_or_its_contract() {
        let state = crate::store::state::ResidentState {
            token_preferences: crate::store::state::built_in_token_preferences(),
            ..Default::default()
        };
        let token = state
            .token_preferences
            .iter()
            .find(|p| p.hosting_chain() == Some(Chain::Ethereum) && !p.token.contract.is_empty())
            .expect("the catalog ships an Ethereum token");
        assert_eq!(
            send_asset_names(&state, Chain::Ethereum, None),
            ("ETH".to_string(), "ETH".to_string())
        );
        assert_eq!(
            send_asset_names(
                &state,
                Chain::Ethereum,
                Some(&token.token.contract.to_uppercase().replace("0X", "0x"))
            ),
            (token.token.symbol.clone(), token.token.name.clone())
        );
        let unknown = format!("0x{}", "ab".repeat(20));
        assert_eq!(
            send_asset_names(&state, Chain::Ethereum, Some(&unknown)),
            (unknown.clone(), unknown)
        );
    }
}

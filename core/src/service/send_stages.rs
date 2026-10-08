//! Core-owned Build → Sign → Broadcast. No stage rebuilds a reviewed transaction.
use super::*;
use crate::send::stages::*;
use crate::store::wallet_domain::TransactionStatus;
use zeroize::Zeroizing;

#[cfg(test)]
#[path = "tests/send_uncertain_recovery.rs"]
mod uncertain_recovery_tests;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn build_send(
        &self,
        request: crate::send::SendExecutionRequest,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            this.build_send_with_review(request, None).await
        })
        .await
    }

    /// Resolve owned edits and persist their review with the prepared transaction.
    pub async fn build_owned_send(
        &self,
        input: super::send_review::SendReviewInput,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let review = this.review_owned_send(input).await?;
            // This operation completes the review itself; no unused confirmation remains.
            this.send_reviews.lock().await.remove(&review.id);
            let advisories = SendArtifactReview {
                warnings: review.warnings,
                recipient_warnings: review.recipient_warnings,
                requires_self_send_confirmation: review.requires_self_send_confirmation,
                staking: None,
                transfer_terms: None,
            };
            this.build_send_with_review(review.request, Some(advisories))
                .await
        })
        .await
    }

    pub async fn list_sends(&self) -> Result<Vec<SendArtifact>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let db = this.bound_database().await?;
            let stored =
                tokio::task::spawn_blocking(move || crate::wallet_db::send_list(&db)).await??;
            Ok(stored.into_iter().map(|s| s.view).collect())
        })
        .await
    }

    pub async fn inspect_send(&self, id: String) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            Ok(this.load_send_artifact(id).await?.view)
        })
        .await
    }

    /// The caller confirms a fingerprint, never supplies replacement transaction fields.
    pub async fn sign_send(
        &self,
        id: String,
        review_digest: String,
        password: Option<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let password = password.map(Zeroizing::new);
            let mut stored = this.load_send_artifact(id).await?;
            if stored.view.stage != SendStage::Prepared
                || stored.view.review_digest != review_digest
            {
                return Err(crate::SpectraBridgeError::failure(
                    "Transaction already signed or review does not match; inspect it again",
                ));
            }
            let chain = stored.view.chain_id;
            super::send_execution::send_chain_for(
                &this.app_state().await,
                &stored.view.wallet_id,
                chain,
            )?;
            let signer = this
                .resolve_send_identity(
                    chain,
                    &stored.view.wallet_id,
                    password.as_ref().map(|p| p.as_str()),
                )
                .await?;
            if crate::send::flow::normalize_address(chain, &stored.view.sender)
                != signer.from_address
            {
                return Err(SpectraBridgeError::failure(
                    "Signer changed; build and review again",
                ));
            }
            let _guard = this.lock_sender(chain, &signer.from_address).await?;
            if stored.view.staking.is_some() {
                this.validate_staking_protocol_state(&stored).await?;
                if chain == crate::registry::Chain::Icp {
                    this.validate_icp_staking_signer_state(&stored, &signer)
                        .await?;
                }
            } else {
                this.validate_prepared_protocol_state(chain, &stored)
                    .await?;
            }
            let (submission, resources) = match &stored.prepared {
                PreparedPayload::ZcashShielded(p) => {
                    this.sign_zcash_shielded(&stored, p, password.as_ref().map(|p| p.as_str()))
                        .await?
                }
                PreparedPayload::LitecoinMweb(p) => {
                    this.sign_litecoin_mweb(&stored, p, password.as_ref().map(|p| p.as_str()))
                        .await?
                }
                PreparedPayload::Evm(p) => {
                    let key = Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                    let raw = p.sign(&key)?;
                    use sha3::Digest;
                    (
                        crate::send::payload::PreparedSubmission {
                            payload: format!("0x{}", hex::encode(&raw)),
                            result_field: "txid".into(),
                            transaction_hash: Some(format!(
                                "0x{}",
                                hex::encode(sha3::Keccak256::digest(&raw))
                            )),
                            nonce: Some(p.nonce),
                        },
                        vec![format!(
                            "{}:{}:nonce:{}",
                            chain.str_id(),
                            signer.from_address,
                            p.nonce
                        )],
                    )
                }
                _ => this.sign_staged_protocol(chain, &stored, &signer).await?,
            };
            stored.view.stage = SendStage::Signed;
            stored.view.signed_payload = Some(submission.payload.clone());
            stored.view.transaction_hash = submission.transaction_hash.clone();
            stored.submission = Some(submission);
            stored.signed_digest = stored.submission_digest()?;
            stored.view.revision += 1;
            this.save_send_artifact(&stored, resources).await?;
            Ok(stored.view)
        })
        .await
    }

    /// Actual configured destinations, in the order the service will use them.
    pub async fn send_endpoints(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            Ok(this
                .endpoints_for(chain, &[EndpointCapability::Broadcast])
                .await
                .as_ref()
                .clone())
        })
        .await
    }

    pub async fn broadcast_send(
        &self,
        id: String,
        endpoints: Vec<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let service = this.clone();
            tokio::spawn(async move { service.broadcast_send_owned(id, endpoints).await }).await?
        })
        .await
    }
}

impl WalletService {
    pub(super) async fn load_send_artifact(
        &self,
        id: String,
    ) -> Result<StoredSend, SpectraBridgeError> {
        let db = self.bound_database().await?;
        Ok(tokio::task::spawn_blocking(move || crate::wallet_db::send_load(&db, &id)).await??)
    }
    pub(super) async fn save_send_artifact(
        &self,
        stored: &StoredSend,
        resources: Vec<String>,
    ) -> Result<(), SpectraBridgeError> {
        let _writer = self.state_writer.lock().await;
        let state = self.app_state().await;
        super::send_execution::send_chain_for(
            &state,
            &stored.view.wallet_id,
            stored.view.chain_id,
        )?;
        let db = self.bound_database().await?;
        let stored = stored.clone();
        tokio::task::spawn_blocking(move || crate::wallet_db::send_save(&db, &stored, &resources))
            .await??;
        Ok(())
    }
    async fn broadcast_send_owned(
        &self,
        id: String,
        endpoints: Vec<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let initial = self.load_send_artifact(id.clone()).await?;
        let chain = initial.view.chain_id;
        let _guard = self.lock_sender(chain, &initial.view.sender).await?;
        let mut stored = self.load_send_artifact(id).await?;
        let submission = stored.submission.clone().ok_or_else(|| {
            SpectraBridgeError::failure("Transaction must be signed before broadcasting")
        })?;
        if endpoints.is_empty() {
            return Err(SpectraBridgeError::failure(
                "Select at least one broadcast endpoint",
            ));
        }
        let chain = stored.view.chain_id;
        let icp_staking = matches!(&stored.prepared, PreparedPayload::IcpStaking(_));
        // A shielded transaction goes to the lightwalletd servers the wallet
        // scans with.
        let shielded = matches!(&stored.prepared, PreparedPayload::ZcashShielded(_));
        let configured = if stored.view.staking.is_some() {
            self.staking_broadcast_endpoints(chain).await?
        } else if shielded {
            self.zcash_shielded_broadcast_endpoints(chain).await?
        } else {
            self.send_endpoints(chain).await?
        };
        let mut unique = std::collections::HashSet::new();
        // Validate every destination before submitting to any of them.
        for endpoint in &endpoints {
            if !configured.contains(endpoint) || !unique.insert(endpoint) {
                return Err(SpectraBridgeError::failure(
                    "Select distinct configured broadcast endpoints",
                ));
            }
            if shielded {
                crate::api::lightwalletd::LightwalletdClient::new(Arc::new(vec![endpoint.clone()]))
                    .session(chain)
                    .await?;
            } else if !icp_staking {
                self.validate_broadcast_endpoint(chain, endpoint).await?;
            }
        }
        if !stored.view.attempts.is_empty()
            && matches!(&stored.prepared, PreparedPayload::Substrate(_))
        {
            let record = self
                .fetch_all_history_records()
                .await?
                .into_iter()
                .find(|r| r.id == stored.view.id)
                .map(|r| r.payload)
                .ok_or_else(|| {
                    SpectraBridgeError::failure("Missing journaled submission record")
                })?;
            if record.status != TransactionStatus::Pending {
                return Err(SpectraBridgeError::failure(
                    "Substrate operation is already final; do not rebroadcast",
                ));
            }
            let hash = submission
                .transaction_hash
                .as_deref()
                .ok_or_else(|| SpectraBridgeError::failure("Missing committed extrinsic hash"))?;
            if let Some(outcome) = self
                .poll_substrate_artifact(chain, &stored.view.id, hash)
                .await?
            {
                self.apply_polled_pending_statuses(
                    chain,
                    vec![crate::store::ResolvedPendingStatus {
                        id: record.id.clone(),
                        status: if outcome.succeeded {
                            "confirmed"
                        } else {
                            "failed"
                        }
                        .into(),
                        confirmations: None,
                        receipt_block_number: Some(
                            i64::try_from(outcome.block_number)
                                .map_err(SpectraBridgeError::failure)?,
                        ),
                        evm_receipt_cost: None,
                    }],
                    Some(vec![record]),
                )
                .await?;
                return Err(SpectraBridgeError::failure(if outcome.succeeded {
                    "Transaction is already confirmed"
                } else {
                    "Transaction execution failed; do not rebroadcast"
                }));
            }
            stored = self.load_send_artifact(stored.view.id.clone()).await?;
        }
        // An expired local signature may already have executed. Read the exact
        // committed hash before rejecting expiry or resubmitting saved bytes.
        // An absent result is still pending; failed reads never claim a refund.
        if !stored.view.attempts.is_empty()
            && !icp_staking
            && (stored.view.staking.is_some() && chain != Chain::Polkadot
                || matches!(
                    &stored.prepared,
                    PreparedPayload::Ton { .. }
                        | PreparedPayload::Aptos(_)
                        | PreparedPayload::Near { .. }
                        | PreparedPayload::NearFunctionCall(_)
                        | PreparedPayload::NearDeleteKey(_)
                ))
        {
            let crate::registry::PendingStatusPoll::TransactionStatus(api) =
                chain.pending_status_poll()
            else {
                return Err(SpectraBridgeError::failure(
                    "Missing exact transaction status reader",
                ));
            };
            let record = self
                .fetch_all_history_records()
                .await?
                .into_iter()
                .find(|row| row.id == stored.view.id)
                .map(|row| row.payload)
                .ok_or_else(|| {
                    SpectraBridgeError::failure("Missing journaled submission record")
                })?;
            match self
                .fetch_pending_transaction_status(chain, api, &record)
                .await?
            {
                crate::api::transaction_status::TransactionStatus::Pending => {}
                crate::api::transaction_status::TransactionStatus::Confirmed {
                    succeeded,
                    block,
                } => {
                    self.apply_polled_pending_statuses(
                        chain,
                        vec![crate::store::ResolvedPendingStatus {
                            id: record.id.clone(),
                            status: if succeeded { "confirmed" } else { "failed" }.into(),
                            confirmations: None,
                            receipt_block_number: block
                                .map(i64::try_from)
                                .transpose()
                                .map_err(SpectraBridgeError::failure)?,
                            evm_receipt_cost: None,
                        }],
                        Some(vec![record]),
                    )
                    .await?;
                    return Err(SpectraBridgeError::failure(if succeeded {
                        "Transaction is already confirmed"
                    } else {
                        "Transaction execution failed; do not rebroadcast"
                    }));
                }
            }
        }
        if let PreparedPayload::Substrate(transaction) = &stored.prepared {
            for endpoint in &endpoints {
                transaction
                    .validate_for_submission(
                        &SubstrateClient::new(Arc::new(vec![endpoint.clone()])),
                        chain,
                    )
                    .await?;
            }
        } else {
            self.validate_signed_expiry(chain, &stored).await?;
        }
        // NEAR bytes do not cap the gas price. Pending retries must still fit
        // the reviewed budget and current spendable balance after checking the
        // original transaction's execution result and reference block above.
        if stored.view.attempts.is_empty()
            || matches!(
                &stored.prepared,
                PreparedPayload::Near { .. }
                    | PreparedPayload::NearFunctionCall(_)
                    | PreparedPayload::NearDeleteKey(_)
            )
        {
            if stored.view.staking.is_some() {
                self.validate_staking_protocol_state(&stored).await?;
            } else {
                self.validate_prepared_protocol_state(chain, &stored)
                    .await?;
            }
        }
        stored.view.selected_endpoints = endpoints.clone();
        stored.view.revision += 1;
        self.save_send_artifact(&stored, Vec::new()).await?;
        let existing = self
            .fetch_all_history_records()
            .await?
            .into_iter()
            .find(|row| row.id == stored.view.id)
            .map(|row| row.payload);
        let mut history = match existing {
            Some(record) => {
                if record.status == TransactionStatus::Confirmed {
                    return Err(SpectraBridgeError::failure(
                        "Transaction is already confirmed",
                    ));
                }
                record
            }
            None => {
                self.begin_send_record(chain, &stored.request, &stored.view.sender)
                    .await?
            }
        };
        history.id = stored.view.id.clone();
        if let Some(intent) = &stored.view.staking {
            history.kind = intent.action.transaction_kind();
        }
        match &stored.view.operation {
            Some(crate::send::stages::WalletOperation::RevokeApproval { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::RevokeApproval;
            }
            Some(crate::send::stages::WalletOperation::DeleteAccessKey { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::DeleteAccessKey;
            }
            Some(crate::send::stages::WalletOperation::MergeCoins { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::MergeCoins;
            }
            Some(crate::send::stages::WalletOperation::CloseTokenAccounts { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::CloseTokenAccounts;
            }
            Some(crate::send::stages::WalletOperation::RefundTokenStorage { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::RefundTokenStorage;
            }
            Some(crate::send::stages::WalletOperation::TrustAsset { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::TrustAsset;
            }
            Some(crate::send::stages::WalletOperation::RemoveTrustLine { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::RemoveTrustLine;
            }
            // A transfer like any other, of an asset that is one token: the
            // indexer's row for it has this identity, and replaces this one.
            Some(crate::send::stages::WalletOperation::TransferNft {
                contract,
                standard,
                token_id,
                collection,
                ..
            }) => {
                history.deployment_id = Some(crate::tokens::nft_deployment_id(
                    chain, *standard, contract, token_id,
                ));
                history.asset_display_name = crate::tokens::nft_display_name(collection, token_id);
                history.symbol = stored.view.symbol.clone();
            }
            Some(crate::send::stages::WalletOperation::ShieldTransparent { .. }) => {
                history.kind = crate::store::wallet_domain::TransactionKind::Shield;
            }
            Some(
                crate::send::stages::WalletOperation::CloseAccount { .. }
                | crate::send::stages::WalletOperation::ShieldedPayment { .. },
            )
            | None => {}
        }
        history.created_at_unix = stored.view.created_at;
        if icp_staking {
            // A repaired neuron reuses its original history identity, while the
            // reviewed management revision has new ingress request IDs.
            history.status = TransactionStatus::Pending;
            history.failure_reason = None;
            history.transaction_hash = submission.transaction_hash.clone();
        } else if history.transaction_hash.is_none() {
            history.transaction_hash = submission.transaction_hash.clone();
        }
        history.nonce = submission
            .nonce
            .map(i64::try_from)
            .transpose()
            .map_err(|_| SpectraBridgeError::failure("Nonce exceeds history range"))?;
        history.signed_transaction_payload = Some(serde_json::to_string(&submission)?);
        history.signed_transaction_payload_format = Some("core.submission_json".into());
        self.save_send_record(history.clone()).await?;
        // Every selected endpoint gets the payload at once. Each attempt is
        // saved as uncertain before anything is sent, so a crash mid-way
        // never loses the record of a submission.
        let first = stored.view.attempts.len();
        let mut submissions = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            let api = if icp_staking {
                Some(crate::EndpointApi::IcpReplica)
            } else if shielded {
                Some(crate::EndpointApi::Lightwalletd)
            } else {
                self.endpoint_api(chain, &endpoint).await
            }
            .ok_or_else(|| {
                SpectraBridgeError::failure(
                    "Endpoint does not support broadcasts on the selected network",
                )
            })?;
            stored.view.attempts.push(BroadcastAttempt {
                endpoint: endpoint.clone(),
                attempted_at: crate::store::now_unix(),
                outcome: SubmissionOutcome::Uncertain,
                transaction_hash: None,
                detail: "Submission outcome unknown; retry only this signed payload".into(),
            });
            submissions.push((api, endpoint));
        }
        stored.view.revision += 1;
        self.save_send_artifact(&stored, Vec::new()).await?;
        let submission = &submission;
        let artifact_id = &stored.view.id;
        let results = futures::future::join_all(submissions.into_iter().map(|(api, endpoint)| {
            let payload = submission.payload.clone();
            async move {
                if icp_staking {
                    return self
                        .broadcast_icp_staking_at(&endpoint, artifact_id, &payload)
                        .await;
                }
                self.broadcast_at(chain, api, Arc::new(vec![endpoint]), payload)
                    .await
                    .and_then(|result| {
                        let value: serde_json::Value = serde_json::from_str(&result)?;
                        value[&submission.result_field]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                SpectraBridgeError::failure(
                                    "Node returned no transaction identifier",
                                )
                            })
                    })
            }
        }))
        .await;
        for (offset, result) in results.into_iter().enumerate() {
            let attempt = &mut stored.view.attempts[first + offset];
            match result {
                Ok(hash)
                    if submission.transaction_hash.as_ref().is_none_or(|expected| {
                        // Hex ids are one id in either case.
                        if chain.is_evm() || chain.mainnet_counterpart() == Chain::Cardano {
                            expected.eq_ignore_ascii_case(&hash)
                        } else {
                            expected == &hash
                        }
                    }) =>
                {
                    attempt.outcome = SubmissionOutcome::Accepted;
                    attempt.transaction_hash = Some(hash);
                    attempt.detail =
                        "Node accepted the transaction; on-chain confirmation is pending".into();
                    if icp_staking {
                        attempt.detail = "Every ICP funding and governance step has a verified execution receipt".into();
                    }
                }
                Ok(_) => {
                    attempt.detail =
                        "Node returned a different transaction identifier; submission is uncertain"
                            .into();
                }
                Err(error) => {
                    attempt.detail = error.to_string();
                }
            }
            let (level, message) = if attempt.outcome == SubmissionOutcome::Accepted {
                (
                    crate::service::DiagnosticLogLevel::Info,
                    format!("Broadcast accepted by {}.", attempt.endpoint),
                )
            } else {
                (
                    crate::service::DiagnosticLogLevel::Warning,
                    format!(
                        "Broadcast to {} uncertain: {}",
                        attempt.endpoint, attempt.detail
                    ),
                )
            };
            self.record_event(
                level,
                "Broadcast",
                message,
                Some(chain),
                submission.transaction_hash.clone(),
            )
            .await;
        }
        stored.view.revision += 1;
        self.save_send_artifact(&stored, Vec::new()).await?;
        if let Some(accepted) = stored
            .view
            .attempts
            .get(first..)
            .unwrap_or_default()
            .iter()
            .find(|a| a.outcome == SubmissionOutcome::Accepted)
        {
            history.transaction_hash = accepted.transaction_hash.clone();
            history.failure_reason = None;
            if icp_staking {
                history.status = TransactionStatus::Confirmed;
            }
            self.save_send_record(history).await?;
        }
        Ok(stored.view)
    }
}

impl WalletService {
    pub(super) async fn build_send_with_review(
        &self,
        mut request: crate::send::SendExecutionRequest,
        review: Option<SendArtifactReview>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        request.zeroize_sensitive_fields();
        request.password = None;
        request.sign_only = false;
        let chain = request.chain_id;
        if let Some(reason) = chain.transparent_send_unavailable_reason() {
            return Err(SpectraBridgeError::invalid(reason));
        }
        request.memo = request
            .memo
            .as_ref()
            .map(|memo| memo.validated(chain))
            .transpose()?;
        super::send_execution::validate_execution_amount(chain, &request)?;
        let state = self.app_state().await;
        if let Some(contract) = &request.contract_address {
            let normalized =
                crate::tokens::normalize_token_identifier(Some(contract.clone()), chain);
            let tracked = state.token_preferences.iter().find(|p| {
                p.token.chain_id == chain
                    && crate::tokens::normalize_token_identifier(
                        Some(p.token.contract.clone()),
                        chain,
                    ) == normalized
            });
            let standard = tracked
                .map(|p| p.token.token_standard.as_str())
                .or(request.token_standard.as_deref())
                .unwrap_or_else(|| chain.token_standard_for_identifier(contract));
            if request
                .token_standard
                .as_deref()
                .is_some_and(|requested| requested != standard)
            {
                return Err(SpectraBridgeError::invalid(
                    "requested token protocol differs from tracked deployment",
                ));
            }
            request.contract_address = Some(crate::tokens::validate_protocol_identifier(
                chain, standard, contract,
            )?);
            if !chain.sends_token_standard(standard) {
                return Err(SpectraBridgeError::invalid(format!(
                    "{standard} transfers are not supported"
                )));
            }
            request.token_standard = Some(standard.to_string());
        }
        super::send_execution::send_chain_for(&state, &request.wallet_id, chain)?;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == request.wallet_id)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))?;
        let sender = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
            .to_string();
        if !crate::send::flow::is_valid_send_address(chain, request.to_address.clone()) {
            return Err(SpectraBridgeError::failure(
                "Invalid destination for selected network",
            ));
        }
        let prepared = if chain.is_evm() {
            let endpoints = self.endpoints_for(chain, &[EndpointCapability::Fee]).await;
            let mut overrides = request
                .evm_overrides
                .clone()
                .unwrap_or_default()
                .resolve(chain)?;
            if overrides.nonce.is_none() {
                overrides.nonce = Some(self.next_send_nonce(chain, &sender).await?);
            }
            let (to, value, data) = if let Some(contract) = &request.contract_address {
                let metadata = EvmClient::new(
                    self.endpoints_for(chain, &[EndpointCapability::TokenBalance])
                        .await,
                    chain.evm_chain_id()?,
                )
                .fetch_erc20_metadata(contract)
                .await?;
                if request
                    .token_decimals
                    .is_some_and(|d| d != u32::from(metadata.decimals))
                {
                    return Err(SpectraBridgeError::failure(
                        "Token decimals do not match the selected network",
                    ));
                }
                request.token_decimals = Some(u32::from(metadata.decimals));
                let amount = crate::send::amount_input::parse_raw_amount(
                    &request.amount_str,
                    u32::from(metadata.decimals),
                )?;
                (
                    contract.clone(),
                    0,
                    crate::send::evm::encode_erc20_transfer(&request.to_address, amount)?,
                )
            } else {
                (
                    request.to_address.clone(),
                    crate::send::amount_input::parse_raw_amount(
                        &request.amount_str,
                        u32::from(chain.native_decimals()),
                    )?,
                    Vec::new(),
                )
            };
            PreparedPayload::Evm(
                crate::api::http::race(&endpoints, |endpoint| {
                    let sender = &sender;
                    let to = &to;
                    let data = &data;
                    let overrides = &overrides;
                    async move {
                        self.validate_endpoint_network(chain, &endpoint).await?;
                        crate::send::evm::prepare_transfer(
                            &EvmClient::new(Arc::new(vec![endpoint]), chain.evm_chain_id()?),
                            sender,
                            to,
                            value,
                            data,
                            overrides,
                        )
                        .await
                    }
                })
                .await?,
            )
        } else {
            self.prepare_staged_protocol(chain, &mut request, &sender)
                .await?
        };
        if let PreparedPayload::Evm(transaction) = &prepared {
            let budget = transaction.maximum_fee_wei()?;
            if let Some(reviewed) = &request.fee_amount {
                let reviewed =
                    crate::send::payload::fee_units(reviewed, u32::from(chain.native_decimals()))?;
                if budget > u128::from(reviewed) {
                    return Err(crate::send::error::SendError::invalid(
                        "Network fee changed; build and review again",
                    )
                    .into());
                }
            }
            self.validate_evm_funds(chain, &sender, transaction).await?;
        }
        let signing_payload_hex = hex::encode(match &prepared {
            PreparedPayload::Evm(p) => p.signing_payload()?,
            PreparedPayload::Bitcoin(p) => hex::decode(&p.unsigned_hex)?,
            PreparedPayload::Icp(p) => hex::decode(&p.argument_hex)?,
            PreparedPayload::Substrate(p) => p.signing_payload()?,
            PreparedPayload::Solana(p) => p.message.clone(),
            PreparedPayload::Tron(p) => p.raw.clone(),
            PreparedPayload::Aptos(p) => p.message.clone(),
            PreparedPayload::Sui(p) => {
                let mut bytes = vec![0, 0, 0];
                bytes.extend(&p.bytes);
                bytes
            }
            _ => Vec::new(),
        });
        let mut review = match review {
            Some(review) => review,
            None => self.staged_send_review(&request).await?,
        };
        review.transfer_terms = prepared.transfer_terms(request.token_decimals);
        let mut stored = StoredSend {
            view: SendArtifact {
                id: crate::store::new_transaction_id(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: request.wallet_id.clone(),
                chain_id: request.chain_id,
                sender,
                recipient: request.to_address.clone(),
                amount: request.amount_str.clone(),
                asset: request
                    .contract_address
                    .clone()
                    .unwrap_or_else(|| chain.coin_symbol().into()),
                symbol: super::send_records::send_asset_names(
                    &state,
                    chain,
                    request.contract_address.as_deref(),
                )
                .0,
                staking: None,
                operation: None,
                created_at: crate::store::now_unix().floor(),
                review_digest: String::new(),
                review,
                prepared_details: serde_json::to_string_pretty(&prepared)?,
                signing_payload_hex,
                signed_payload: None,
                transaction_hash: None,
                attempts: Vec::new(),
                selected_endpoints: Vec::new(),
                memo: request.memo.clone(),
            },
            request,
            prepared,
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        stored.view.review_digest = stored.digest()?;
        self.save_send_artifact(&stored, Vec::new()).await?;
        Ok(stored.view)
    }
}

impl WalletService {
    pub(super) async fn validate_evm_fee_budget(
        &self,
        chain: Chain,
        transaction: &crate::send::evm::PreparedEvmTransaction,
    ) -> Result<(), SpectraBridgeError> {
        let fees = EvmClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Fee]).await,
            chain.evm_chain_id()?,
        );
        let fresh_extra = fees
            .fetch_rollup_fee(&transaction.signing_payload()?, transaction.gas_limit)
            .await?;
        if fresh_extra > transaction.additional_fee_wei {
            return Err(crate::send::error::SendError::invalid(
                "Network fee changed; build and review again",
            )
            .into());
        }
        Ok(())
    }

    pub(super) async fn validate_evm_funds(
        &self,
        chain: Chain,
        sender: &str,
        transaction: &crate::send::evm::PreparedEvmTransaction,
    ) -> Result<(), SpectraBridgeError> {
        use crate::send::error::SendError;
        let client = EvmClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Balance])
                .await,
            chain.evm_chain_id()?,
        );
        let balance: u128 = client
            .fetch_balance(sender)
            .await?
            .balance_wei
            .parse()
            .map_err(|_| SendError::invalid("invalid EVM balance"))?;
        let needed = transaction
            .value_wei
            .checked_add(transaction.maximum_fee_wei()?)
            .ok_or_else(|| SendError::invalid("EVM amount plus fee overflow"))?;
        if balance < needed {
            return Err(SendError::insufficient_funds().into());
        }
        // Inspect the transaction's actual transfer, including a calldata override.
        // Other contract calls are simulated by eth_estimateGas and still need native gas.
        if transaction
            .data
            .starts_with(&crate::api::evm_json_rpc::SEL_TRANSFER)
        {
            if transaction.data.len() != 68
                || transaction.data[36..52].iter().any(|byte| *byte != 0)
            {
                return Err(SendError::invalid(
                    "ERC-20 transfer amount exceeds u128 or calldata is malformed",
                )
                .into());
            }
            let amount =
                u128::from_be_bytes(transaction.data[52..68].try_into().expect("ERC-20 amount"));
            let client = EvmClient::new(
                self.endpoints_for(chain, &[EndpointCapability::TokenBalance])
                    .await,
                chain.evm_chain_id()?,
            );
            if client
                .fetch_erc20_balance_of(&transaction.to, sender)
                .await?
                < amount
            {
                return Err(SendError::insufficient_funds().into());
            }
        }
        Ok(())
    }
}

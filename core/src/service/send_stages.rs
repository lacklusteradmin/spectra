//! Core-owned Build → Sign → Broadcast. No stage rebuilds a reviewed transaction.
use super::*;
use crate::send::stages::*;
use crate::store::wallet_domain::CoreTransactionStatus;
use zeroize::Zeroizing;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn build_send(
        &self,
        request: crate::send::SendExecutionRequest,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        self.build_send_with_review(request, None).await
    }

    /// Resolve owned edits and persist their review with the prepared transaction.
    pub async fn build_owned_send(
        &self,
        input: super::send_review::SendReviewInput,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let review = self.review_owned_send(input).await?;
        // This operation completes the review itself; no unused confirmation remains.
        self.send_reviews.lock().await.remove(&review.id);
        let advisories = SendArtifactReview {
            warnings: review.warnings,
            recipient_warnings: review.recipient_warnings,
            requires_self_send_confirmation: review.requires_self_send_confirmation,
        };
        self.build_send_with_review(review.request, Some(advisories))
            .await
    }

    pub async fn list_sends(&self) -> Result<Vec<SendArtifact>, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let stored = tokio::task::spawn_blocking(move || crate::wallet_db::send_list(&db))
            .await
            .map_err(|e| e.to_string())??;
        Ok(stored.into_iter().map(|s| s.view).collect())
    }

    pub async fn inspect_send(&self, id: String) -> Result<SendArtifact, SpectraBridgeError> {
        Ok(self.load_send_artifact(id).await?.view)
    }

    /// The caller confirms a fingerprint, never supplies replacement transaction fields.
    pub async fn sign_send(
        &self,
        id: String,
        review_digest: String,
        password: Option<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let password = password.map(Zeroizing::new);
        let mut stored = self.load_send_artifact(id).await?;
        if stored.view.stage != SendStage::Prepared || stored.view.review_digest != review_digest {
            return Err(
                "Transaction already signed or review does not match; inspect it again".into(),
            );
        }
        let chain = chain_for_id(&stored.view.chain_id)?;
        super::send_execution::send_chain_for(
            &self.app_state().await,
            &stored.view.wallet_id,
            chain,
        )?;
        let signer = self
            .resolve_send_identity(
                chain,
                &stored.view.wallet_id,
                password.as_ref().map(|p| p.as_str()),
            )
            .await?;
        if crate::send::flow::normalize_address(chain.str_id(), &stored.view.sender)
            != signer.from_address
        {
            return Err("Signer changed; build and review again".into());
        }
        let _guard = self.lock_sender(chain, &signer.from_address).await?;
        let (submission, resources) = match &stored.prepared {
            PreparedPayload::Evm(p) => {
                let client = EvmClient::new(
                    self.endpoints_for(chain.str_id(), &["verification"]).await,
                    chain.evm_chain_id()?,
                );
                let nonce = if stored
                    .request
                    .evm_overrides
                    .as_ref()
                    .and_then(|o| o.nonce)
                    .is_some()
                {
                    let response = client
                        .call(
                            "eth_getTransactionCount",
                            json!([signer.from_address, "latest"]),
                        )
                        .await?;
                    crate::fetch::evm::parse_hex_u64(
                        response.as_str().ok_or("Missing confirmed nonce")?,
                    )?
                } else {
                    client.fetch_nonce(&signer.from_address).await?
                };
                if p.chain_id != chain.evm_chain_id()? || nonce > p.nonce {
                    return Err("Prepared nonce or network is stale; build and review again".into());
                }
                let key = Zeroizing::new(
                    hex::decode(signer.private_key_hex.as_str()).map_err(|e| e.to_string())?,
                );
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
            _ => self.sign_staged_protocol(chain, &stored, &signer).await?,
        };
        stored.view.stage = SendStage::Signed;
        stored.view.signed_payload = Some(submission.payload.clone());
        stored.view.transaction_hash = submission.transaction_hash.clone();
        stored.submission = Some(submission);
        stored.signed_digest = stored.submission_digest()?;
        stored.view.revision += 1;
        self.save_send_artifact(&stored, resources).await?;
        Ok(stored.view)
    }

    /// Actual configured destinations, in the order the service will use them.
    pub async fn send_endpoints(
        &self,
        chain_id: String,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        Ok(self
            .endpoints_for(chain.str_id(), &["broadcast"])
            .await
            .as_ref()
            .clone())
    }

    pub async fn broadcast_send(
        &self,
        id: String,
        endpoints: Vec<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let service = self.clone();
        tokio::spawn(async move { service.broadcast_send_owned(id, endpoints).await })
            .await
            .map_err(|e| e.to_string())?
    }
}

impl WalletService {
    pub(super) async fn load_send_artifact(
        &self,
        id: String,
    ) -> Result<StoredSend, SpectraBridgeError> {
        let db = self.bound_database().await?;
        Ok(
            tokio::task::spawn_blocking(move || crate::wallet_db::send_load(&db, &id))
                .await
                .map_err(|e| e.to_string())??,
        )
    }
    async fn save_send_artifact(
        &self,
        stored: &StoredSend,
        resources: Vec<String>,
    ) -> Result<(), SpectraBridgeError> {
        let _writer = self.state_writer.lock().await;
        let state = self.app_state().await;
        super::send_execution::send_chain_for(
            &state,
            &stored.view.wallet_id,
            chain_for_id(&stored.view.chain_id)?,
        )?;
        let db = self.bound_database().await?;
        let stored = stored.clone();
        tokio::task::spawn_blocking(move || crate::wallet_db::send_save(&db, &stored, &resources))
            .await
            .map_err(|e| e.to_string())??;
        Ok(())
    }
    async fn broadcast_send_owned(
        &self,
        id: String,
        endpoints: Vec<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let initial = self.load_send_artifact(id.clone()).await?;
        let chain = chain_for_id(&initial.view.chain_id)?;
        let _guard = self.lock_sender(chain, &initial.view.sender).await?;
        let mut stored = self.load_send_artifact(id).await?;
        let submission = stored
            .submission
            .clone()
            .ok_or("Transaction must be signed before broadcasting")?;
        if endpoints.is_empty() {
            return Err("Select at least one broadcast endpoint".into());
        }
        let chain = chain_for_id(&stored.view.chain_id)?;
        let configured = self.send_endpoints(chain.str_id().into()).await?;
        let mut unique = std::collections::HashSet::new();
        // Validate every destination before submitting to any of them.
        for endpoint in &endpoints {
            if !configured.contains(endpoint) || !unique.insert(endpoint) {
                return Err("Select distinct configured broadcast endpoints".into());
            }
            self.validate_broadcast_endpoint(chain, endpoint).await?;
        }
        self.validate_signed_expiry(chain, &stored).await?;
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
                if record.status == CoreTransactionStatus::Confirmed {
                    return Err("Transaction is already confirmed".into());
                }
                record
            }
            None => {
                self.begin_send_record(chain, &stored.request, &stored.view.sender)
                    .await?
            }
        };
        history.id = stored.view.id.clone();
        history.created_at_unix = stored.view.created_at;
        if history.transaction_hash.is_none() {
            history.transaction_hash = submission.transaction_hash.clone();
        }
        history.nonce = submission
            .nonce
            .map(i64::try_from)
            .transpose()
            .map_err(|_| "Nonce exceeds history range")?;
        history.signed_transaction_payload = Some(serde_json::to_string(&submission)?);
        history.signed_transaction_payload_format = Some("core.submission_json".into());
        self.save_send_record(history.clone()).await?;
        for endpoint in endpoints {
            let index = stored.view.attempts.len();
            stored.view.attempts.push(BroadcastAttempt {
                endpoint: endpoint.clone(),
                attempted_at: crate::store::now_unix(),
                outcome: SubmissionOutcome::Uncertain,
                transaction_hash: None,
                detail: "Submission outcome unknown; retry only this signed payload".into(),
            });
            stored.view.revision += 1;
            self.save_send_artifact(&stored, Vec::new()).await?;
            let api = chain
                .endpoint_api(EndpointSlot::Primary)
                .ok_or("No broadcast API")?;
            let result = self
                .broadcast_at(
                    chain,
                    api,
                    Arc::new(vec![endpoint]),
                    submission.payload.clone(),
                )
                .await
                .and_then(|result| {
                    let value: serde_json::Value = serde_json::from_str(&result)?;
                    value[&submission.result_field]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| "Node returned no transaction identifier".into())
                });
            let attempt = &mut stored.view.attempts[index];
            match result {
                Ok(hash)
                    if submission.transaction_hash.as_ref().is_none_or(|expected| {
                        if chain.is_evm() {
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
            stored.view.revision += 1;
            self.save_send_artifact(&stored, Vec::new()).await?;
            self.record_event(
                level,
                "Broadcast",
                message,
                Some(chain.str_id().into()),
                submission.transaction_hash.clone(),
            )
            .await;
        }
        if let Some(accepted) = stored
            .view
            .attempts
            .iter()
            .find(|a| a.outcome == SubmissionOutcome::Accepted)
        {
            history.transaction_hash = accepted.transaction_hash.clone();
            history.failure_reason = None;
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
        let chain = chain_for_id(&request.chain_id)?;
        if let Some(reason) = chain.transparent_send_unavailable_reason() {
            return Err(reason.into());
        }
        super::send_execution::validate_execution_amount(chain, &request)?;
        let state = self.app_state().await;
        super::send_execution::send_chain_for(&state, &request.wallet_id, chain)?;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == request.wallet_id)
            .ok_or("Wallet removed")?;
        let sender = wallet
            .address_on(chain)
            .ok_or("Wallet has no address on this network")?
            .to_string();
        if !crate::send::flow::is_valid_send_address(
            chain.str_id().into(),
            request.to_address.clone(),
        ) {
            return Err("Invalid destination for selected network".into());
        }
        let prepared = if chain.is_evm() {
            let endpoints = self.endpoints_for(chain.str_id(), &["fee"]).await;
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
                    self.endpoints_for(chain.str_id(), &["token-balance"]).await,
                    chain.evm_chain_id()?,
                )
                .fetch_erc20_metadata(contract)
                .await?;
                if request
                    .token_decimals
                    .is_some_and(|d| d != u32::from(metadata.decimals))
                {
                    return Err("Token decimals do not match the selected network".into());
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
                    crate::send::amount_input::parse_raw_amount(&request.amount_str, 18)?,
                    Vec::new(),
                )
            };
            PreparedPayload::Evm(
                crate::fetch::http::with_fallback(&endpoints, |endpoint| {
                    let sender = &sender;
                    let to = &to;
                    let data = &data;
                    let overrides = &overrides;
                    async move {
                        self.validate_endpoint_network(chain, &endpoint)
                            .await
                            .map_err(|e| e.to_string())?;
                        EvmClient::new(Arc::new(vec![endpoint]), chain.evm_chain_id()?)
                            .prepare_transfer(sender, to, value, data, overrides)
                            .await
                    }
                })
                .await?,
            )
        } else {
            self.prepare_staged_protocol(chain, &mut request, &sender)
                .await?
        };
        let signing_payload_hex = hex::encode(match &prepared {
            PreparedPayload::Evm(p) => p.signing_payload()?,
            PreparedPayload::Bitcoin(p) => {
                hex::decode(&p.unsigned_hex).map_err(|e| e.to_string())?
            }
            PreparedPayload::Icp(p) => hex::decode(&p.argument_hex).map_err(|e| e.to_string())?,
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
        let review = match review {
            Some(review) => review,
            None => self.staged_send_review(&request).await?,
        };
        let mut stored = StoredSend {
            view: SendArtifact {
                id: crate::store::new_transaction_id(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: request.wallet_id.clone(),
                chain_id: request.chain_id.clone(),
                sender,
                recipient: request.to_address.clone(),
                amount: request.amount_str.clone(),
                asset: request
                    .contract_address
                    .clone()
                    .unwrap_or_else(|| chain.coin_symbol().into()),
                created_at: crate::store::now_unix().floor(),
                review_digest: String::new(),
                review,
                prepared_details: serde_json::to_string_pretty(&prepared)?,
                signing_payload_hex,
                signed_payload: None,
                transaction_hash: None,
                attempts: Vec::new(),
                selected_endpoints: Vec::new(),
            },
            request,
            prepared,
            submission: None,
            signed_digest: None,
        };
        stored.view.review_digest = stored.digest()?;
        self.save_send_artifact(&stored, Vec::new()).await?;
        Ok(stored.view)
    }
}

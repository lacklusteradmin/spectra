//! Staking queries use committed transport settings, refreshed for every call.
use super::*;
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend,
};
use crate::staking::{StakingPosition, StakingRequest, StakingValidator, service::StakingService};
use crate::store::wallet_domain::TransactionStatus;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Read positions owned by this wallet. Extra pool identifiers extend discovery;
    /// every returned position still has to prove the wallet's authority.
    pub async fn fetch_staking_positions(
        &self,
        wallet_id: String,
        chain_id: Chain,
        targets: Vec<String>,
        password: Option<String>,
    ) -> Result<Vec<StakingPosition>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let owner = this.staking_owner(&wallet_id, chain_id).await?;
            let mut known = this.stored_staking_targets(&wallet_id, chain_id).await?;
            known.extend(targets);
            known.sort();
            known.dedup();
            if known.len() > 1000 {
                return Err(SpectraBridgeError::invalid("Too many staking targets"));
            }
            match chain_id {
                Chain::Polkadot => this.fetch_substrate_staking_positions(&owner).await,
                Chain::Icp => {
                    let password = password.map(zeroize::Zeroizing::new);
                    let identity = this
                        .resolve_send_identity(
                            chain_id,
                            &wallet_id,
                            password.as_deref().map(String::as_str),
                        )
                        .await?;
                    this.fetch_icp_staking_positions(&identity).await
                }
                _ => {
                    this.fetch_account_staking_positions(chain_id, &owner, None, &known)
                        .await
                }
            }
        })
        .await
    }

    /// Persist a reviewed staking transaction. Sign and broadcast use the same
    /// durable operations as transfers; no password enters the artifact.
    pub async fn build_staking(
        &self,
        mut request: StakingRequest,
        password: Option<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let chain = request.chain_id;
            let owner = this.staking_owner(&request.wallet_id, chain).await?;
            if request.lockup_seconds.is_some() && chain != Chain::Icp {
                return Err(SpectraBridgeError::invalid(
                    "Dissolve delay applies only to ICP neurons",
                ));
            }
            for value in [&request.validator_id, &request.position_id, &request.amount] {
                if value
                    .as_ref()
                    .is_some_and(|s| s.is_empty() || s.trim() != s)
                {
                    return Err(SpectraBridgeError::invalid(
                        "Staking identifiers and amounts must be nonempty and canonical",
                    ));
                }
            }
            let password = password.map(zeroize::Zeroizing::new);
            let identity = this
                .resolve_send_identity(
                    chain,
                    &request.wallet_id,
                    password.as_deref().map(String::as_str),
                )
                .await?;
            if identity.from_address != owner {
                return Err(SpectraBridgeError::invalid(
                    "Staking owner differs from signing identity",
                ));
            }
            let mut resolved_amount = request.amount.clone();
            if resolved_amount.is_none() {
                let positions = match chain {
                    Chain::Polkadot => this.fetch_substrate_staking_positions(&owner).await?,
                    Chain::Icp => this.fetch_icp_staking_positions(&identity).await?,
                    _ => {
                        this.fetch_account_staking_positions(
                            chain,
                            &owner,
                            identity.public_key_hex.as_deref(),
                            &this
                                .stored_staking_targets(&request.wallet_id, chain)
                                .await?,
                        )
                        .await?
                    }
                };
                let position = positions
                    .iter()
                    .find(|p| Some(&p.id) == request.position_id.as_ref())
                    .ok_or_else(|| {
                        SpectraBridgeError::invalid("Select an owned staking position")
                    })?;
                let units = match request.action {
                    crate::staking::StakingAction::Unstake => &position.staked_amount_smallest_unit,
                    crate::staking::StakingAction::Withdraw => {
                        &position.withdrawable_amount_smallest_unit
                    }
                    crate::staking::StakingAction::ClaimRewards => position
                        .claimable_rewards_smallest_unit
                        .as_ref()
                        .ok_or_else(|| {
                            SpectraBridgeError::invalid("Claimable rewards are unavailable")
                        })?,
                    _ => return Err(SpectraBridgeError::invalid("Staking amount is required")),
                };
                resolved_amount = Some(
                    crate::decimal::from_unit_digits(units, u32::from(chain.native_decimals()))
                        .ok_or_else(|| {
                            SpectraBridgeError::invalid("Invalid staking position amount")
                        })?,
                );
                if matches!(
                    chain,
                    Chain::Solana | Chain::Sui | Chain::Aptos | Chain::Near
                ) {
                    request.amount = resolved_amount.clone();
                }
            }
            let prepared = match chain {
                Chain::Polkadot => this.prepare_substrate_staking(&request, &owner).await?,
                Chain::Icp => this.prepare_icp_staking(&request, &identity).await?,
                _ => {
                    this.prepare_account_staking(
                        &request,
                        &owner,
                        identity.public_key_hex.as_deref(),
                    )
                    .await?
                }
            };
            let recipient = match (&prepared, request.action) {
                (PreparedPayload::IcpStaking(p), crate::staking::StakingAction::Stake) => {
                    p.funding.as_ref().map(|funding| funding.recipient.clone())
                }
                (_, crate::staking::StakingAction::Stake) => request.validator_id.clone(),
                _ => request.position_id.clone(),
            }
            .unwrap_or_else(|| owner.clone());
            let amount = resolved_amount
                .ok_or_else(|| SpectraBridgeError::invalid("Staking amount is required"))?;
            let review = staking_review(&prepared, &request)?;
            let send_request = crate::send::SendExecutionRequest {
                wallet_id: request.wallet_id.clone(),
                chain_id: chain,
                to_address: recipient.clone(),
                amount_str: amount.clone(),
                password: None,
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_amount: None,
                evm_overrides: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                sign_only: false,
                memo: None,
            };
            let signing_payload = match &prepared {
                PreparedPayload::Solana(p) => p.message.clone(),
                PreparedPayload::Aptos(p) => p.message.clone(),
                PreparedPayload::Sui(p) => {
                    let mut bytes = vec![0, 0, 0];
                    bytes.extend(&p.bytes);
                    bytes
                }
                PreparedPayload::Substrate(p) => p.signing_payload()?,
                PreparedPayload::NearFunctionCall(p) => p.message.clone(),
                PreparedPayload::IcpStaking(p) => serde_json::to_vec(&p.calls)?,
                _ => Vec::new(),
            };
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id: request.wallet_id.clone(),
                    chain_id: chain,
                    sender: owner,
                    recipient,
                    amount,
                    asset: chain.coin_symbol().into(),
                    symbol: chain.coin_symbol().into(),
                    staking: Some(request),
                    operation: None,
                    created_at: crate::store::now_unix().floor(),
                    review_digest: String::new(),
                    review: SendArtifactReview {
                        staking: Some(review),
                        ..SendArtifactReview::default()
                    },
                    prepared_details: serde_json::to_string_pretty(&prepared)?,
                    signing_payload_hex: hex::encode(signing_payload),
                    signed_payload: None,
                    transaction_hash: None,
                    attempts: vec![],
                    selected_endpoints: vec![],
                    memo: None,
                },
                request: send_request,
                prepared,
                submission: None,
                signed_digest: None,
                substrate_verified_through: None,
                icp_staking_receipts: vec![],
            };
            stored.view.review_digest = stored.digest()?;
            this.save_send_artifact(&stored, vec![]).await?;
            Ok(stored.view)
        })
        .await
    }

    pub async fn fetch_staking_validators(
        &self,
        chain_id: crate::registry::Chain,
    ) -> Result<Vec<StakingValidator>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            if chain_id == Chain::Icp {
                return this.fetch_icp_staking_validators().await;
            }
            let endpoints = this.staking_endpoints(chain_id).await?;
            StakingService::new(vec![endpoints])
                .fetch_validators(chain_id)
                .await
                .map_err(|error| match error {
                    crate::staking::StakingError::Api(error) => error.into(),
                    crate::staking::StakingError::NotYetImplemented => {
                        SpectraBridgeError::invalid(error)
                    }
                })
        })
        .await
    }

    pub async fn staking_broadcast_endpoints(
        &self,
        chain_id: Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        if chain_id == Chain::Icp {
            self.api_endpoints(
                chain_id,
                crate::EndpointApi::IcpReplica,
                &[EndpointCapability::Staking, EndpointCapability::Broadcast],
            )
            .await
        } else {
            self.send_endpoints(chain_id).await
        }
    }

    /// Repair an interrupted, funded ICP neuron and persist a new review of
    /// management steps. This operation never constructs another funding call.
    pub async fn repair_staking(
        &self,
        id: String,
        password: Option<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move { this.repair_icp_staking_owned(id, password).await }).await
    }

    /// Check execution, authorizing only a fresh IC read_state request when
    /// required. This operation never signs or broadcasts a funding command.
    pub async fn recheck_staking(
        &self,
        id: String,
        password: Option<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let stored = this.load_send_artifact(id.clone()).await?;
            if stored.view.staking.is_none() || stored.view.attempts.is_empty() {
                return Err(SpectraBridgeError::invalid(
                    "Staking operation has not been submitted",
                ));
            }
            let chain = stored.view.chain_id;
            let record = this
                .fetch_all_history_records()
                .await?
                .into_iter()
                .find(|r| r.id == id)
                .map(|r| r.payload)
                .ok_or_else(|| {
                    SpectraBridgeError::invalid("Missing journaled staking operation")
                })?;
            if record.status != TransactionStatus::Pending {
                return Ok(stored.view);
            }
            let status = if chain == Chain::Icp {
                let password = password.map(zeroize::Zeroizing::new);
                let identity = this
                    .resolve_send_identity(
                        chain,
                        &stored.view.wallet_id,
                        password.as_deref().map(String::as_str),
                    )
                    .await?;
                let key = crate::send::keys::Ed25519Seed::from_hex(&identity.private_key_hex)?;
                this.icp_staking_receipts(&stored, Some(&key)).await?
            } else if chain == Chain::Polkadot {
                let hash = record.transaction_hash.as_deref().ok_or_else(|| {
                    SpectraBridgeError::invalid("Missing staking transaction hash")
                })?;
                this.poll_substrate_artifact(chain, &id, hash)
                    .await?
                    .map(
                        |outcome| crate::api::transaction_status::TransactionStatus::Confirmed {
                            succeeded: outcome.succeeded,
                            block: Some(outcome.block_number),
                        },
                    )
                    .unwrap_or(crate::api::transaction_status::TransactionStatus::Pending)
            } else {
                let api = chain.default_api().ok_or_else(|| {
                    SpectraBridgeError::invalid("Missing staking verification API")
                })?;
                this.fetch_pending_transaction_status(chain, api, &record)
                    .await?
            };
            if let crate::api::transaction_status::TransactionStatus::Confirmed {
                succeeded,
                block,
            } = status
            {
                this.apply_polled_pending_statuses(
                    chain,
                    vec![crate::store::ResolvedPendingStatus {
                        id: id.clone(),
                        status: if succeeded { "confirmed" } else { "failed" }.into(),
                        confirmations: None,
                        receipt_block_number: block
                            .map(i64::try_from)
                            .transpose()
                            .map_err(SpectraBridgeError::invalid)?,
                        evm_receipt_cost: None,
                    }],
                    Some(vec![record]),
                )
                .await?;
            }
            Ok(this.load_send_artifact(id).await?.view)
        })
        .await
    }
}
impl WalletService {
    pub(super) async fn staking_owner(
        &self,
        wallet_id: &str,
        chain: Chain,
    ) -> Result<String, SpectraBridgeError> {
        if chain.is_testnet() || !chain.supports_staking() {
            return Err(SpectraBridgeError::invalid(
                "Staking is unavailable for this network",
            ));
        }
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or_else(|| SpectraBridgeError::invalid("Staking wallet does not exist"))?;
        wallet.address_on(chain).map(str::to_string).ok_or_else(|| {
            SpectraBridgeError::invalid("Wallet has no address on this staking network")
        })
    }

    async fn stored_staking_targets(
        &self,
        wallet_id: &str,
        chain: Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let db = self.bound_database().await?;
        Ok(
            tokio::task::spawn_blocking(move || crate::wallet_db::send_list(&db))
                .await??
                .into_iter()
                .filter(|s| s.view.wallet_id == wallet_id && s.view.chain_id == chain)
                .filter_map(|s| {
                    s.view.staking.and_then(|r| {
                        if matches!(chain, Chain::Aptos | Chain::Near) {
                            r.validator_id.or(r.position_id)
                        } else {
                            r.validator_id
                        }
                    })
                })
                .collect(),
        )
    }

    pub(super) async fn validate_staking_protocol_state(
        &self,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        match stored.view.chain_id {
            Chain::Polkadot => self.validate_substrate_staking_state(stored).await,
            Chain::Icp => self.validate_icp_staking_state(stored).await,
            _ => self.validate_account_staking_state(stored).await,
        }
    }

    /// Inspect the same effective configuration used for the next query.
    pub async fn staking_endpoints(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<ChainEndpoints, SpectraBridgeError> {
        if chain.is_testnet() || !chain.supports_staking() {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!(
                    "Staking queries are unavailable for {}",
                    chain.chain_display_name()
                )
                .into(),
            });
        }
        let endpoints = if chain == Chain::Icp {
            self.api_endpoints(
                chain,
                crate::EndpointApi::IcpReplica,
                &[EndpointCapability::Staking],
            )
            .await?
        } else {
            let api = chain
                .default_api()
                .ok_or_else(|| SpectraBridgeError::invalid("Missing staking API"))?;
            self.chain_endpoints(chain, &[EndpointCapability::Staking])
                .await
                .into_iter()
                .filter(|endpoint| endpoint.api == api)
                .map(|endpoint| endpoint.url)
                .collect()
        };
        if endpoints.is_empty() {
            return Err(SpectraBridgeError::failure(
                "No staking endpoints configured",
            ));
        }
        Ok(ChainEndpoints {
            capabilities: vec![EndpointCapability::Staking],
            chain_id: chain,
            endpoints,
        })
    }
}

fn staking_review(
    prepared: &PreparedPayload,
    request: &StakingRequest,
) -> Result<crate::staking::StakingReview, SpectraBridgeError> {
    let invalid = || SpectraBridgeError::invalid("Missing reviewed staking network fee");
    let (fee, upper, rent) = match prepared {
        PreparedPayload::Solana(p) => (
            u128::from(p.network_fee.ok_or_else(invalid)?),
            false,
            p.stake_rent.map(u128::from),
        ),
        PreparedPayload::Sui(p) => (u128::from(p.gas_budget), true, None),
        PreparedPayload::Aptos(p) => {
            let number = |field: &str| {
                p.body[field]
                    .as_str()
                    .and_then(|s| s.parse::<u128>().ok())
                    .ok_or_else(invalid)
            };
            (
                number("gas_unit_price")?
                    .checked_mul(number("max_gas_amount")?)
                    .ok_or_else(invalid)?,
                true,
                None,
            )
        }
        PreparedPayload::NearFunctionCall(p) => (
            p.fee_budget
                .parse::<u128>()
                .map_err(SpectraBridgeError::invalid)?,
            true,
            None,
        ),
        PreparedPayload::Substrate(p) => (p.fee, true, None),
        PreparedPayload::IcpStaking(p) => (u128::from(p.fee), false, None),
        _ => return Err(invalid()),
    };
    let decimals = u32::from(request.chain_id.native_decimals());
    Ok(crate::staking::StakingReview {
        network_fee: crate::decimal::from_units(fee, decimals),
        fee_is_upper_bound: upper,
        fee_is_deducted_from_amount: request.chain_id == Chain::Icp
            && request.action == crate::staking::StakingAction::Withdraw,
        funding_already_completed: false,
        refundable_deposit: rent
            .filter(|r| *r > 0)
            .map(|r| crate::decimal::from_units(r, decimals)),
        lockup_seconds: request.lockup_seconds,
        reward_payout_is_delayed: request.chain_id == Chain::Icp
            && request.action == crate::staking::StakingAction::ClaimRewards,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn staking_reads_latest_owned_settings_and_preserves_explicit_overrides() {
        let path =
            std::env::temp_dir().join(format!("staking-{}.db", crate::store::new_event_id()));
        let service = WalletService::new_catalog().unwrap();
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        for url in ["http://127.0.0.1:13001", "http://127.0.0.1:13002"] {
            service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                        capabilities: crate::endpoint_capability_options(
                            crate::registry::Chain::Solana,
                            crate::EndpointApi::SolanaJsonRpc,
                        ),
                        chain_id: crate::registry::Chain::Solana,
                        api: "solana-json-rpc".into(),
                        endpoint: url.into(),
                    },
                })
                .await
                .unwrap();
            assert_eq!(
                service
                    .staking_endpoints(crate::registry::Chain::Solana)
                    .await
                    .unwrap()
                    .endpoints[0],
                url
            );
        }
        for chain in [Chain::Bitcoin, Chain::SolanaDevnet] {
            assert!(service.fetch_staking_validators(chain).await.is_err());
        }
        let explicit = WalletService::new(vec![ChainEndpoints {
            capabilities: vec![EndpointCapability::Staking],
            chain_id: crate::registry::Chain::Solana,
            endpoints: vec!["http://127.0.0.1:13003".into()],
        }])
        .unwrap();
        explicit
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        assert_eq!(
            explicit
                .staking_endpoints(crate::registry::Chain::Solana)
                .await
                .unwrap()
                .endpoints,
            vec!["http://127.0.0.1:13003"]
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn validators_come_from_the_configured_node_with_its_minimum_delegation() {
        use wiremock::matchers::{body_partial_json, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        for (rpc, result) in [
            (
                "getVoteAccounts",
                serde_json::json!({"current":[{
                    "votePubkey":"Validator11111111111111111111111111111111",
                    "activatedStake":1000,"commission":5
                }],"delinquent":[]}),
            ),
            (
                "getStakeMinimumDelegation",
                serde_json::json!({"context":{"slot":100},"value":1_500_000_000u64}),
            ),
        ] {
            Mock::given(method("POST"))
                .and(body_partial_json(serde_json::json!({"method": rpc})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"jsonrpc":"2.0","id":1,"result":result})),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        let path =
            std::env::temp_dir().join(format!("staking-{}.db", crate::store::new_event_id()));
        let service = WalletService::new_catalog().unwrap();
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        service
            .apply_state_command(StateCommand::SetAppSetting {
                update: crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                    capabilities: crate::endpoint_capability_options(
                        Chain::Solana,
                        crate::EndpointApi::SolanaJsonRpc,
                    ),
                    chain_id: Chain::Solana,
                    api: "solana-json-rpc".into(),
                    endpoint: server.uri(),
                },
            })
            .await
            .unwrap();
        let validators = service
            .fetch_staking_validators(Chain::Solana)
            .await
            .unwrap();
        assert_eq!(validators.len(), 1);
        assert_eq!(
            validators[0].min_delegation_smallest_unit.as_deref(),
            Some("1500000000")
        );
        let _ = std::fs::remove_file(path);
    }
}

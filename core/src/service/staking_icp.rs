//! NNS staking resolves the wallet controller, derives the neuron account, and
//! journals locally signed funding/management ingress IDs through send stages.
use super::send_identity::ResolvedSendIdentity;
use super::*;
use crate::api::icp_replica::*;
use crate::send::icp_staking::{
    self, IcpStakingCall, IcpStakingCallKind, PreparedIcpStaking, SignedIcpStakingCall,
};
use crate::send::keys::Ed25519Seed;
use crate::send::stages::{PreparedPayload, StoredSend};
use crate::staking::{
    StakingAction, StakingPosition, StakingPositionStatus, StakingRequest, StakingValidator,
};

fn key(identity: &ResolvedSendIdentity) -> Result<Ed25519Seed, SpectraBridgeError> {
    Ok(Ed25519Seed::from_hex(&identity.private_key_hex)?)
}
fn controller(key: &Ed25519Seed) -> candid::Principal {
    candid::Principal::from_slice(&crate::derivation::icp::principal(&key.public_key()))
}
fn fail(message: impl std::fmt::Display) -> SpectraBridgeError {
    SpectraBridgeError::invalid(message)
}
fn amount(request: &StakingRequest) -> Result<u64, SpectraBridgeError> {
    let value = request
        .amount
        .as_deref()
        .ok_or_else(|| fail("ICP staking amount is required"))?;
    let value = crate::send::amount_input::parse_raw_amount(value, 8)?;
    u64::try_from(value)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| fail("ICP staking amount must be a positive u64"))
}
fn require_repairable_ingress(
    status: &IngressStatus,
    expiry_ns: u64,
    now_ns: u64,
) -> Result<(), SpectraBridgeError> {
    match status {
        IngressStatus::Processing => Err(fail(
            "Original ICP ingress is still processing; recheck it before repair",
        )),
        IngressStatus::Pending if expiry_ns > now_ns => Err(fail(
            "Original ICP ingress can still be accepted; resume it or wait for expiry before repair",
        )),
        _ => Ok(()),
    }
}
fn net_stake(neuron: &Neuron) -> Result<u64, SpectraBridgeError> {
    neuron
        .cached_neuron_stake_e8s
        .checked_sub(neuron.neuron_fees_e8s)
        .ok_or_else(|| fail("Neuron fees exceed cached stake"))
}
fn maturity_can_be_disbursed(neuron: &Neuron) -> Result<bool, SpectraBridgeError> {
    let (minimum, maximum) = Chain::Icp.icp_maturity_disbursement_limits()?;
    Ok(neuron.maturity_e8s_equivalent >= minimum
        && neuron.spawn_at_timestamp_seconds.is_none()
        && neuron
            .maturity_disbursements_in_progress
            .as_ref()
            .map_or(0, Vec::len)
            < maximum)
}

fn withdraw_command(
    neuron: &Neuron,
    owner: &str,
    net_principal: Option<u64>,
) -> Result<ManageCommand, SpectraBridgeError> {
    // Governance subtracts neuron fees from an explicit Amount before
    // deducting its ledger fee. None withdraws all net stake directly.
    let amount = net_principal
        .map(|amount| {
            amount
                .checked_add(neuron.neuron_fees_e8s)
                .map(|e8s| Amount { e8s })
                .ok_or_else(|| fail("ICP partial withdrawal amount overflow"))
        })
        .transpose()?;
    Ok(ManageCommand::Disburse(Disburse {
        to_account: Some(AccountIdentifier {
            hash: crate::derivation::icp::validate_account(owner)?.to_vec(),
        }),
        amount,
    }))
}

fn claim_maturity_command(owner: &str) -> Result<ManageCommand, SpectraBridgeError> {
    Ok(ManageCommand::DisburseMaturity(DisburseMaturity {
        percentage_to_disburse: 100,
        to_account: None,
        to_account_identifier: Some(AccountIdentifier {
            hash: crate::derivation::icp::validate_account(owner)?.to_vec(),
        }),
    }))
}
fn existing_neuron_delay(neuron: &Neuron, now: u64) -> Result<Option<u64>, SpectraBridgeError> {
    match neuron.dissolve_state {
        Some(DissolveState::DissolveDelaySeconds(0)) => Ok(None),
        Some(DissolveState::DissolveDelaySeconds(delay)) => Ok(Some(delay)),
        Some(DissolveState::WhenDissolvedTimestampSeconds(timestamp)) => {
            Ok(Some(timestamp.saturating_sub(now)))
        }
        None => Err(fail(
            "Existing neuron has no dissolve state; repair cannot change its lock",
        )),
    }
}
fn validate_existing_neuron_intent(
    request: &StakingRequest,
    prepared: &PreparedIcpStaking,
    neuron: &Neuron,
    owner: candid::Principal,
    now: u64,
    fee: u64,
) -> Result<(), SpectraBridgeError> {
    if neuron.controller != Some(owner)
        || request.position_id.as_deref()
            != neuron.id.as_ref().map(|id| id.id.to_string()).as_deref()
        || prepared.subaccount_hex != hex::encode(&neuron.account)
        || prepared.amount == 0
        || !position(neuron, &prepared.sender, now)?
            .available_actions
            .contains(&request.action)
    {
        return Err(fail(
            "Neuron authority, position or staking readiness changed; review again",
        ));
    }
    let stake = net_stake(neuron)?;
    let valid = match request.action {
        StakingAction::Unstake => prepared.amount == stake && prepared.fee == 0,
        StakingAction::Withdraw => {
            prepared.amount <= stake
                && (request.amount.is_some() || prepared.amount == stake)
                && prepared.amount > fee
                && prepared.fee == fee
        }
        StakingAction::ClaimRewards => {
            prepared.amount == neuron.maturity_e8s_equivalent
                && maturity_can_be_disbursed(neuron)?
                && prepared.fee == 0
        }
        StakingAction::Stake => false,
    };
    if !valid {
        return Err(fail(
            "Neuron amount or network fee changed; build and review again",
        ));
    }
    let (command, kind) = match request.action {
        StakingAction::Unstake => (
            ManageCommand::Configure(Configure {
                operation: Some(ConfigureOperation::StartDissolving(Empty {})),
            }),
            IcpStakingCallKind::Configure,
        ),
        StakingAction::Withdraw => (
            withdraw_command(
                neuron,
                &prepared.sender,
                request.amount.as_ref().map(|_| prepared.amount),
            )?,
            IcpStakingCallKind::Disburse,
        ),
        StakingAction::ClaimRewards => (
            claim_maturity_command(&prepared.sender)?,
            IcpStakingCallKind::DisburseMaturity,
        ),
        StakingAction::Stake => return Err(fail("Existing neuron action required")),
    };
    let mut expected = vec![];
    let governance = candid::Principal::from_text(Chain::Icp.icp_governance_id()?).map_err(fail)?;
    add_manage(
        &mut expected,
        &governance,
        NeuronSelector::NeuronId(
            neuron
                .id
                .clone()
                .ok_or_else(|| fail("Neuron ID is absent"))?,
        ),
        command,
        kind,
        None,
    )?;
    if prepared.calls.len() != 1
        || prepared.calls[0].canister != expected[0].canister
        || prepared.calls[0].method != expected[0].method
        || prepared.calls[0].kind != expected[0].kind
        || prepared.calls[0].argument_hex != expected[0].argument_hex
        || prepared.calls[0].nonce_hex != expected[0].nonce_hex
    {
        return Err(fail(
            "ICP staking command differs from current reviewed amount or destination",
        ));
    }
    Ok(())
}

fn increase_delay(lockup: u64) -> Result<ManageCommand, SpectraBridgeError> {
    let delay = u32::try_from(lockup)
        .ok()
        .filter(|delay| *delay > 0 && u64::from(*delay) <= 63_115_200)
        .ok_or_else(|| fail("Review an ICP dissolve delay between 1 second and 2 years"))?;
    Ok(ManageCommand::Configure(Configure {
        operation: Some(ConfigureOperation::IncreaseDissolveDelay(
            IncreaseDissolveDelay {
                additional_dissolve_delay_seconds: delay,
            },
        )),
    }))
}
fn position(neuron: &Neuron, owner: &str, now: u64) -> Result<StakingPosition, SpectraBridgeError> {
    let id = neuron
        .id
        .as_ref()
        .map(|id| id.id)
        .filter(|id| *id > 0)
        .ok_or_else(|| fail("Owned neuron has no ID"))?;
    let amount = net_stake(neuron)?;
    let (status, unlock, stake, unbonding, withdrawable) = if neuron
        .spawn_at_timestamp_seconds
        .is_some()
    {
        (StakingPositionStatus::Inactive, None, amount, 0, 0)
    } else {
        match &neuron.dissolve_state {
            Some(DissolveState::DissolveDelaySeconds(delay)) if *delay > 0 => {
                (StakingPositionStatus::Active, None, amount, 0, 0)
            }
            Some(DissolveState::WhenDissolvedTimestampSeconds(timestamp)) if *timestamp > now => (
                StakingPositionStatus::Unbonding,
                Some(*timestamp),
                0,
                amount,
                0,
            ),
            _ => (StakingPositionStatus::Withdrawable, None, 0, 0, amount),
        }
    };
    let mut actions = Vec::new();
    if status == StakingPositionStatus::Active && amount > 0 {
        actions.push(StakingAction::Unstake);
    }
    if withdrawable > 0 {
        actions.push(StakingAction::Withdraw);
    }
    if maturity_can_be_disbursed(neuron)? {
        actions.push(StakingAction::ClaimRewards);
    }
    let mut pending_rewards = 0u64;
    let mut payout_time: Option<u64> = None;
    for pending in neuron.maturity_disbursements_in_progress.iter().flatten() {
        let amount = pending
            .amount_e8s
            .ok_or_else(|| fail("Pending maturity has no amount"))?;
        pending_rewards = pending_rewards
            .checked_add(amount)
            .ok_or_else(|| fail("Pending maturity overflow"))?;
        if let Some(time) = pending.finalize_disbursement_timestamp_seconds {
            payout_time = Some(payout_time.map_or(time, |previous| previous.min(time)));
        }
    }
    Ok(StakingPosition {
        id: id.to_string(),
        owner: owner.into(),
        validator_identifier: String::new(),
        status,
        staked_amount_smallest_unit: stake.to_string(),
        unbonding_amount_smallest_unit: unbonding.to_string(),
        withdrawable_amount_smallest_unit: withdrawable.to_string(),
        claimable_rewards_smallest_unit: Some(neuron.maturity_e8s_equivalent.to_string()),
        pending_rewards_smallest_unit: Some(pending_rewards.to_string()),
        rewards_unlock_time_unix: payout_time,
        unlock_epoch: None,
        unlock_time_unix: unlock,
        available_actions: actions,
    })
}

impl WalletService {
    async fn save_icp_reply(
        &self,
        artifact_id: &str,
        call: &SignedIcpStakingCall,
        certified: &CertifiedIngressStatus,
    ) -> Result<(), SpectraBridgeError> {
        let IngressStatus::Replied(reply) = &certified.status else {
            return Err(fail("Missing certified ICP reply"));
        };
        icp_staking::validate_reply(&call.kind, reply)?;
        let receipt = icp_staking::IcpStakingReceipt {
            canister: call.canister.clone(),
            request_id: call.request_id.clone(),
            kind: call.kind.clone(),
            certificate_hex: hex::encode(&certified.certificate),
            reply_hex: hex::encode(reply),
        };
        let _writer = self.state_writer.lock().await;
        let db = self.bound_database().await?;
        let id = artifact_id.to_string();
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::send_record_icp_receipt(&db, &id, receipt)
        })
        .await??;
        Ok(())
    }
    fn saved_icp_reply(
        stored: &StoredSend,
        call: &SignedIcpStakingCall,
    ) -> Result<bool, SpectraBridgeError> {
        if let Some(receipt) = stored
            .icp_staking_receipts
            .iter()
            .find(|receipt| receipt.matches(call))
        {
            receipt.validate()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub(super) async fn icp_staking_receipts(
        &self,
        stored: &StoredSend,
        key: Option<&Ed25519Seed>,
    ) -> Result<crate::api::transaction_status::TransactionStatus, SpectraBridgeError> {
        use crate::api::transaction_status::TransactionStatus;
        let PreparedPayload::IcpStaking(prepared) = &stored.prepared else {
            return Err(fail("Missing prepared ICP staking steps"));
        };
        let submission = stored
            .submission
            .as_ref()
            .ok_or_else(|| fail("Staking has not been signed"))?;
        let calls: Vec<SignedIcpStakingCall> = serde_json::from_str(&submission.payload)?;
        if calls.len() != prepared.calls.len() {
            return Err(fail("ICP ingress steps differ from reviewed operation"));
        }
        let client = self.icp_replica().await?;
        let mut pending = false;
        for call in &prepared.completed_calls {
            if !Self::saved_icp_reply(stored, call)?
                && !(call.kind == IcpStakingCallKind::Fund && prepared.funding_confirmed_by_ledger)
            {
                return Err(fail("Completed ICP step has no execution proof"));
            }
        }
        for call in calls {
            let current = self.load_send_artifact(stored.view.id.clone()).await?;
            if Self::saved_icp_reply(&current, &call)? {
                continue;
            }
            let id: [u8; 32] = hex::decode(&call.request_id)?
                .try_into()
                .map_err(|_| fail("Invalid ICP ingress ID"))?;
            let read = match key {
                Some(key) => icp_staking::read_state(&id, icp_staking::expiry()?, key)?,
                None => hex::decode(&call.read_state_hex)?,
            };
            let certified = client.status(&call.canister, &id, &read).await?;
            match &certified.status {
                IngressStatus::Replied(reply) => {
                    if icp_staking::validate_reply(&call.kind, reply).is_err() {
                        return Ok(TransactionStatus::Confirmed {
                            succeeded: false,
                            block: None,
                        });
                    }
                    self.save_icp_reply(&stored.view.id, &call, &certified)
                        .await?;
                }
                IngressStatus::Rejected(_) => {
                    return Ok(TransactionStatus::Confirmed {
                        succeeded: false,
                        block: None,
                    });
                }
                IngressStatus::Pending
                | IngressStatus::Processing
                | IngressStatus::Unknown
                | IngressStatus::Done => pending = true,
            }
        }
        Ok(if pending {
            TransactionStatus::Pending
        } else {
            TransactionStatus::Confirmed {
                succeeded: true,
                block: None,
            }
        })
    }
    async fn icp_replica(&self) -> Result<IcpReplicaClient, SpectraBridgeError> {
        let endpoints = self
            .api_endpoints(
                Chain::Icp,
                crate::EndpointApi::IcpReplica,
                &[EndpointCapability::Staking],
            )
            .await?;
        if endpoints.is_empty() {
            return Err(fail("No ICP replica staking endpoints configured"));
        }
        Ok(IcpReplicaClient::new(Arc::new(endpoints)))
    }
    async fn icp_query<T: candid::CandidType + serde::de::DeserializeOwned>(
        &self,
        method: &str,
        argument: Vec<u8>,
        key: &Ed25519Seed,
    ) -> Result<T, SpectraBridgeError> {
        let body = icp_staking::signed_query(method, &argument, key)?;
        let reply = self.icp_replica().await?.query(&body).await?;
        candid::decode_one(&reply).map_err(fail)
    }
    async fn icp_economics(
        &self,
        key: &Ed25519Seed,
    ) -> Result<NetworkEconomics, SpectraBridgeError> {
        self.icp_query(
            "get_network_economics_parameters",
            candid::encode_args(()).map_err(fail)?,
            key,
        )
        .await
    }
    async fn owned_icp_neurons(
        &self,
        key: &Ed25519Seed,
    ) -> Result<Vec<Neuron>, SpectraBridgeError> {
        let controller = controller(key);
        let mut neurons = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for page in 0..1000 {
            let args = ListNeurons {
                neuron_ids: vec![],
                include_neurons_readable_by_caller: true,
                include_empty_neurons_readable_by_caller: Some(true),
                include_public_neurons_in_full_neurons: Some(false),
                page_number: Some(page),
                page_size: Some(100),
                neuron_subaccounts: None,
            };
            let response: ListNeuronsResponse = self
                .icp_query("list_neurons", candid::encode_one(args).map_err(fail)?, key)
                .await?;
            let count = response.full_neurons.len();
            for neuron in response.full_neurons {
                // Readable-by-caller includes hot-key/public neurons. Only the
                // controller can withdraw; never present them as this wallet's.
                if neuron.controller != Some(controller) {
                    continue;
                }
                let id = neuron
                    .id
                    .as_ref()
                    .map(|id| id.id)
                    .ok_or_else(|| fail("Owned neuron has no ID"))?;
                if !seen.insert(id) {
                    return Err(fail("ICP neuron pagination repeated a position"));
                }
                if neuron.account.len() != 32 {
                    return Err(fail("Invalid owned neuron subaccount"));
                }
                neurons.push(neuron);
            }
            match response.total_pages_available {
                Some(total) if page + 1 >= total => return Ok(neurons),
                None if count < 100 => return Ok(neurons),
                _ => {}
            }
        }
        Err(fail("ICP neuron pagination exceeded its bound"))
    }
    pub(super) async fn fetch_icp_staking_positions(
        &self,
        identity: &ResolvedSendIdentity,
    ) -> Result<Vec<StakingPosition>, SpectraBridgeError> {
        let key = key(identity)?;
        let now = crate::store::now_unix() as u64;
        self.owned_icp_neurons(&key)
            .await?
            .iter()
            .map(|n| position(n, &identity.from_address, now))
            .collect()
    }
    pub(super) async fn fetch_icp_staking_validators(
        &self,
    ) -> Result<Vec<StakingValidator>, SpectraBridgeError> {
        crate::staking::icp::IcpStakingClient::new(
            self.api_endpoints(
                Chain::Icp,
                crate::EndpointApi::IcpReplica,
                &[EndpointCapability::Staking],
            )
            .await?,
        )
        .fetch_validators()
        .await
        .map_err(|error| match error {
            crate::staking::StakingError::Api(error) => error.into(),
            crate::staking::StakingError::NotYetImplemented => SpectraBridgeError::invalid(error),
        })
    }

    pub(super) async fn prepare_icp_staking(
        &self,
        request: &StakingRequest,
        identity: &ResolvedSendIdentity,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let key = key(identity)?;
        let controller = controller(&key);
        let owner = &identity.from_address;
        let governance =
            candid::Principal::from_text(Chain::Icp.icp_governance_id()?).map_err(fail)?;
        let economics = self.icp_economics(&key).await?;
        let mut calls = Vec::new();
        let mut nonce = None;
        let mut funding_ledger_hash = None;
        let mut funding = None;
        let (subaccount, prepared_amount, fee) = if request.action == StakingAction::Stake {
            if request.position_id.is_some() {
                return Err(fail(
                    "A new ICP neuron does not select an existing position",
                ));
            }
            let amount = amount(request)?;
            if amount < economics.neuron_minimum_stake_e8s {
                return Err(fail("Amount is below the live minimum neuron stake"));
            }
            let lockup = request
                .lockup_seconds
                .filter(|delay| *delay > 0 && *delay <= 63_115_200)
                .ok_or_else(|| fail("Review an ICP dissolve delay between 1 second and 2 years"))?;
            let follow = request
                .validator_id
                .as_deref()
                .ok_or_else(|| fail("Select a known neuron for voting follow"))?;
            let follow_id = follow
                .parse::<u64>()
                .ok()
                .filter(|id| *id > 0 && id.to_string() == follow)
                .ok_or_else(|| fail("Invalid neuron follow ID"))?;
            if !self
                .fetch_icp_staking_validators()
                .await?
                .iter()
                .any(|n| n.identifier == follow)
            {
                return Err(fail("Follow target is not a current known NNS neuron"));
            }
            let index = rand::random::<u64>();
            nonce = Some(index);
            let subaccount = icp_staking::neuron_subaccount(controller.as_slice(), index);
            let recipient = hex::encode(crate::derivation::icp::account_with_subaccount(
                governance.as_slice(),
                &subaccount,
            ));
            let client = IcpClient::new(
                self.endpoints_for(
                    Chain::Icp,
                    &[EndpointCapability::Balance, EndpointCapability::Fee],
                )
                .await,
            );
            let transfer =
                crate::send::icp_stages::prepare_transfer(&client, owner, &recipient, amount)
                    .await?;
            if transfer.fee != economics.transaction_fee_e8s {
                return Err(fail("ICP ledger and governance fee quotes disagree"));
            }
            let required = amount
                .checked_add(transfer.fee)
                .ok_or_else(|| fail("ICP staking amount overflow"))?;
            if client.fetch_balance(owner).await?.e8s < required {
                return Err(fail("Insufficient ICP balance for stake and network fee"));
            }
            let mut transfer = transfer;
            transfer.memo = index;
            transfer.argument_hex = hex::encode(transfer.argument()?);
            funding_ledger_hash = Some(transfer.transaction_hash()?);
            funding = Some(transfer.clone());
            calls.push(icp_staking::funding_call(transfer, index)?);
            let gov_nonce = Some(hex::encode(candid::encode_one(index).map_err(fail)?));
            calls.push(IcpStakingCall {
                canister: governance.to_text(),
                method: "claim_or_refresh_neuron_from_account".into(),
                argument_hex: hex::encode(
                    candid::encode_one(ClaimNeuron {
                        memo: index,
                        controller: None,
                    })
                    .map_err(fail)?,
                ),
                nonce_hex: gov_nonce.clone(),
                kind: IcpStakingCallKind::Claim,
            });
            add_manage(
                &mut calls,
                &governance,
                NeuronSelector::Subaccount(subaccount.to_vec()),
                increase_delay(lockup)?,
                IcpStakingCallKind::Configure,
                gov_nonce.clone(),
            )?;
            // NNS CatchAll (0) excludes Governance (4) and SNS (14).
            // Follow all three explicitly for the wallet's selected target.
            for topic in [0, 4, 14] {
                add_manage(
                    &mut calls,
                    &governance,
                    NeuronSelector::Subaccount(subaccount.to_vec()),
                    ManageCommand::Follow(Follow {
                        topic,
                        followees: vec![NeuronId { id: follow_id }],
                    }),
                    IcpStakingCallKind::Follow,
                    gov_nonce.clone(),
                )?;
            }
            (subaccount.to_vec(), amount, economics.transaction_fee_e8s)
        } else {
            if request.validator_id.is_some() || request.lockup_seconds.is_some() {
                return Err(fail(
                    "Existing neuron actions select only an owned position",
                ));
            }
            let id = request
                .position_id
                .as_deref()
                .ok_or_else(|| fail("Select an owned neuron"))?;
            let neuron = self
                .owned_icp_neurons(&key)
                .await?
                .into_iter()
                .find(|n| n.id.as_ref().is_some_and(|n| n.id.to_string() == id))
                .ok_or_else(|| fail("Neuron is not controlled by this wallet"))?;
            let current = position(&neuron, owner, crate::store::now_unix() as u64)?;
            if !current.available_actions.contains(&request.action) {
                return Err(fail("Neuron is not ready for this staking action"));
            }
            let (command, kind, amount, fee) = match request.action {
                StakingAction::Unstake => {
                    if request.amount.is_some() {
                        return Err(fail("ICP dissolve applies to the complete neuron"));
                    }
                    (
                        ManageCommand::Configure(Configure {
                            operation: Some(ConfigureOperation::StartDissolving(Empty {})),
                        }),
                        IcpStakingCallKind::Configure,
                        net_stake(&neuron)?,
                        0,
                    )
                }
                StakingAction::Withdraw => {
                    let stake = net_stake(&neuron)?;
                    let amount = if request.amount.is_some() {
                        amount(request)?
                    } else {
                        stake
                    };
                    if amount > stake || amount <= economics.transaction_fee_e8s {
                        return Err(fail(
                            "ICP disbursement exceeds stake or cannot pay its ledger fee",
                        ));
                    }
                    (
                        withdraw_command(&neuron, owner, request.amount.as_ref().map(|_| amount))?,
                        IcpStakingCallKind::Disburse,
                        amount,
                        economics.transaction_fee_e8s,
                    )
                }
                StakingAction::ClaimRewards => {
                    if request.amount.is_some() {
                        return Err(fail(
                            "ICP maturity disbursement claims 100% of current maturity",
                        ));
                    }
                    if !maturity_can_be_disbursed(&neuron)? {
                        return Err(fail(
                            "ICP maturity disbursement requires at least 1 ICP, fewer than 10 pending payouts and a neuron that is not spawning",
                        ));
                    }
                    (
                        claim_maturity_command(owner)?,
                        IcpStakingCallKind::DisburseMaturity,
                        neuron.maturity_e8s_equivalent,
                        0,
                    )
                }
                _ => return Err(fail("Invalid existing neuron action")),
            };
            add_manage(
                &mut calls,
                &governance,
                NeuronSelector::NeuronId(neuron.id.clone().unwrap()),
                command,
                kind,
                None,
            )?;
            (neuron.account, amount, fee)
        };
        Ok(PreparedPayload::IcpStaking(PreparedIcpStaking {
            sender: owner.clone(),
            controller_hex: hex::encode(controller.as_slice()),
            ingress_expiry_ns: icp_staking::expiry()?,
            amount: prepared_amount,
            fee,
            neuron_nonce: nonce,
            subaccount_hex: hex::encode(subaccount),
            calls,
            funding_ledger_hash,
            funding,
            completed_calls: vec![],
            prior_calls: vec![],
            prior_attempts: vec![],
            funding_confirmed_by_ledger: false,
            recovered_configured_neuron: None,
        }))
    }

    pub(super) async fn validate_icp_staking_state(
        &self,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let request = stored
            .view
            .staking
            .as_ref()
            .ok_or_else(|| fail("Missing ICP staking intent"))?;
        let PreparedPayload::IcpStaking(prepared) = &stored.prepared else {
            return Err(fail("ICP staking payload differs from its network"));
        };
        if prepared.sender != stored.view.sender {
            return Err(fail("ICP staking sender changed"));
        }
        if request.action == StakingAction::Stake
            && !prepared
                .completed_calls
                .iter()
                .any(|call| call.kind == IcpStakingCallKind::Fund)
        {
            let client = IcpClient::new(
                self.endpoints_for(Chain::Icp, &[EndpointCapability::Balance])
                    .await,
            );
            if client.fetch_balance(&prepared.sender).await?.e8s
                < prepared
                    .amount
                    .checked_add(prepared.fee)
                    .ok_or_else(|| fail("ICP balance overflow"))?
            {
                return Err(fail("Insufficient ICP staking balance before submission"));
            }
        }
        // Broadcast submits only the previously signed commands. The certified
        // canister result checks authority, readiness and amount at execution.
        Ok(())
    }

    /// Signing already resolved the wallet's controller. Use that identity for
    /// a fresh owned-neuron and fee check without adding a broadcast key prompt.
    pub(super) async fn validate_icp_staking_signer_state(
        &self,
        stored: &StoredSend,
        identity: &ResolvedSendIdentity,
    ) -> Result<(), SpectraBridgeError> {
        let request = stored
            .view
            .staking
            .as_ref()
            .ok_or_else(|| fail("Missing ICP staking intent"))?;
        let PreparedPayload::IcpStaking(prepared) = &stored.prepared else {
            return Err(fail("Missing ICP staking payload"));
        };
        let key = key(identity)?;
        let controller = controller(&key);
        if prepared.sender != identity.from_address
            || prepared.controller_hex != hex::encode(controller.as_slice())
        {
            return Err(fail("ICP staking controller differs from signing identity"));
        }
        if request.action == StakingAction::Stake {
            prepared.validate_original_funding(controller.as_slice())?;
            if prepared
                .calls
                .iter()
                .any(|call| call.kind == IcpStakingCallKind::Fund)
            {
                let economics = self.icp_economics(&key).await?;
                if prepared.amount < economics.neuron_minimum_stake_e8s
                    || prepared.fee != economics.transaction_fee_e8s
                {
                    return Err(fail(
                        "ICP minimum stake or funding fee changed; review again",
                    ));
                }
            }
            return Ok(());
        }
        let neuron = self
            .owned_icp_neurons(&key)
            .await?
            .into_iter()
            .find(|neuron| {
                request.position_id.as_deref()
                    == neuron.id.as_ref().map(|id| id.id.to_string()).as_deref()
            })
            .ok_or_else(|| fail("Neuron is no longer controlled by this wallet"))?;
        let fee = if matches!(
            request.action,
            StakingAction::Unstake | StakingAction::ClaimRewards
        ) {
            0
        } else {
            self.icp_economics(&key).await?.transaction_fee_e8s
        };
        validate_existing_neuron_intent(
            request,
            prepared,
            &neuron,
            controller,
            crate::store::now_unix() as u64,
            fee,
        )
    }

    /// Repair only a funded new neuron. A fresh review signs governance calls
    /// against the original derived subaccount; it cannot sign another transfer.
    pub(super) async fn repair_icp_staking_owned(
        &self,
        id: String,
        password: Option<String>,
    ) -> Result<crate::send::stages::SendArtifact, SpectraBridgeError> {
        let initial = self.load_send_artifact(id.clone()).await?;
        let _sender = self.lock_sender(Chain::Icp, &initial.view.sender).await?;
        let mut stored = self.load_send_artifact(id.clone()).await?;
        let request = stored
            .view
            .staking
            .clone()
            .filter(|r| r.chain_id == Chain::Icp && r.action == StakingAction::Stake)
            .ok_or_else(|| fail("Repair applies only to an interrupted new ICP neuron"))?;
        if stored.view.stage != crate::send::stages::SendStage::Signed
            || stored.view.attempts.is_empty()
        {
            return Err(fail(
                "Only a submitted ICP staking artifact can be repaired",
            ));
        }
        let password = password.map(zeroize::Zeroizing::new);
        let identity = self
            .resolve_send_identity(
                Chain::Icp,
                &stored.view.wallet_id,
                password.as_deref().map(String::as_str),
            )
            .await?;
        let key = key(&identity)?;
        let owner = controller(&key);
        let PreparedPayload::IcpStaking(original) = &stored.prepared else {
            return Err(fail("Missing ICP staking steps"));
        };
        let mut prepared = original.clone();
        let nonce = prepared
            .neuron_nonce
            .ok_or_else(|| fail("Original neuron nonce is absent"))?;
        let subaccount = icp_staking::neuron_subaccount(owner.as_slice(), nonce);
        let governance =
            candid::Principal::from_text(Chain::Icp.icp_governance_id()?).map_err(fail)?;
        let account = hex::encode(crate::derivation::icp::account_with_subaccount(
            governance.as_slice(),
            &subaccount,
        ));
        if prepared.sender != identity.from_address
            || prepared.controller_hex != hex::encode(owner.as_slice())
            || prepared.subaccount_hex != hex::encode(subaccount)
            || stored.view.recipient != account
        {
            return Err(fail(
                "Original neuron funding identity or derived subaccount changed",
            ));
        }
        prepared.validate_original_funding(owner.as_slice())?;
        let signed: Vec<SignedIcpStakingCall> = serde_json::from_str(
            &stored
                .submission
                .as_ref()
                .ok_or_else(|| fail("Missing original signed steps"))?
                .payload,
        )?;
        if signed.len() != prepared.calls.len() {
            return Err(fail("Original ICP step journal is inconsistent"));
        }
        let client = self.icp_replica().await?;
        // Never renew a management call while an earlier ingress is processing.
        // An absent old request is safe to replace only after its acceptance
        // window has closed. Unknown/done proofs remain uncertain, never refunds.
        for call in &signed {
            if Self::saved_icp_reply(&stored, call)? {
                continue;
            }
            let request_id: [u8; 32] = hex::decode(&call.request_id)?
                .try_into()
                .map_err(|_| fail("Invalid original ICP request ID"))?;
            let read = icp_staking::read_state(&request_id, icp_staking::expiry()?, &key)?;
            let certified = client.status(&call.canister, &request_id, &read).await?;
            match &certified.status {
                IngressStatus::Replied(reply)
                    if icp_staking::validate_reply(&call.kind, reply).is_ok() =>
                {
                    self.save_icp_reply(&id, call, &certified).await?;
                }
                status => {
                    require_repairable_ingress(status, prepared.ingress_expiry_ns, unix_ns()?)?
                }
            }
        }
        stored = self.load_send_artifact(id.clone()).await?;
        let neurons = self.owned_icp_neurons(&key).await?;
        let existing = neurons.iter().find(|n| n.account.as_slice() == subaccount);
        let fund = signed
            .iter()
            .chain(&prepared.completed_calls)
            .chain(&prepared.prior_calls)
            .find(|call| call.kind == IcpStakingCallKind::Fund)
            .cloned()
            .ok_or_else(|| fail("Original funding ingress is absent"))?;
        if fund.validated_argument(owner.as_slice())?
            != prepared
                .funding
                .as_ref()
                .ok_or_else(|| fail("Original funding transfer is absent"))?
                .argument()?
        {
            return Err(fail(
                "Certified funding ingress differs from the original funding ledger hash",
            ));
        }
        let mut funded = Self::saved_icp_reply(&stored, &fund)?;
        if !funded {
            let ledger = IcpClient::new(
                self.endpoints_for(Chain::Icp, &[EndpointCapability::Verification])
                    .await,
            );
            let hash = prepared
                .funding_ledger_hash
                .as_deref()
                .ok_or_else(|| fail("Original funding ledger hash is absent"))?;
            let exact = matches!(
                ledger.fetch_transaction_status(hash).await,
                Ok(
                    crate::api::transaction_status::TransactionStatus::Confirmed {
                        succeeded: true,
                        ..
                    }
                )
            );
            funded = exact;
            if funded {
                prepared.funding_confirmed_by_ledger = true;
            }
        }
        if !funded {
            return Err(fail(
                "Neuron funding is not proved; repair never sends another funding transfer",
            ));
        }
        if !prepared
            .completed_calls
            .iter()
            .any(|call| call.request_id == fund.request_id)
        {
            prepared.completed_calls.push(fund);
        }
        let recovered_delay = existing
            .map(|neuron| existing_neuron_delay(neuron, crate::store::now_unix() as u64))
            .transpose()?
            .flatten();
        let configured = recovered_delay.is_some();
        if let Some(neuron) = existing.filter(|_| configured) {
            prepared.recovered_configured_neuron = neuron.id.as_ref().map(|id| id.id);
        }
        let original_calls = prepared.calls.clone();
        let mut remaining = Vec::new();
        for (call, signed_call) in original_calls.into_iter().zip(&signed) {
            if !prepared
                .prior_calls
                .iter()
                .any(|old| old.request_id == signed_call.request_id)
            {
                prepared.prior_calls.push(signed_call.clone());
            }
            if call.kind == IcpStakingCallKind::Fund {
                continue;
            }
            if Self::saved_icp_reply(&stored, signed_call)? {
                if !prepared
                    .completed_calls
                    .iter()
                    .any(|old| old.request_id == signed_call.request_id)
                {
                    prepared.completed_calls.push(signed_call.clone());
                }
                continue;
            }
            if call.kind == IcpStakingCallKind::Configure && configured {
                continue;
            }
            if call.kind == IcpStakingCallKind::Configure {
                let lockup = request
                    .lockup_seconds
                    .filter(|delay| *delay > 0 && *delay <= 63_115_200)
                    .ok_or_else(|| fail("A reviewed dissolve delay is required"))?;
                add_manage(
                    &mut remaining,
                    &governance,
                    NeuronSelector::Subaccount(subaccount.to_vec()),
                    increase_delay(lockup)?,
                    IcpStakingCallKind::Configure,
                    call.nonce_hex,
                )?;
            } else {
                remaining.push(call);
            }
        }
        if remaining.is_empty() {
            return Err(fail(
                "All neuron management steps are complete; recheck the existing operation",
            ));
        }
        if prepared.prior_calls.len() > 32 {
            return Err(fail(
                "Too many ICP repair attempts; inspect the retained ingress journal",
            ));
        }
        prepared.calls = remaining;
        prepared.ingress_expiry_ns = icp_staking::expiry()?;
        prepared.fee = 0;
        prepared.retain_attempts_for_repair(
            &mut stored.view.attempts,
            &mut stored.view.selected_endpoints,
        );
        stored.prepared = PreparedPayload::IcpStaking(prepared);
        stored.submission = None;
        stored.signed_digest = None;
        stored.view.stage = crate::send::stages::SendStage::Prepared;
        stored.view.signed_payload = None;
        stored.view.transaction_hash = None;
        stored.view.prepared_details = serde_json::to_string_pretty(&stored.prepared)?;
        if let PreparedPayload::IcpStaking(p) = &stored.prepared {
            stored.view.signing_payload_hex = hex::encode(serde_json::to_vec(&p.calls)?);
        }
        if let Some(review) = &mut stored.view.review.staking {
            review.network_fee = "0".into();
            review.funding_already_completed = true;
            if configured {
                review.lockup_seconds = recovered_delay;
            }
        }
        stored.view.revision += 1;
        stored.view.review_digest = stored.digest()?;
        self.save_send_artifact(&stored, vec![]).await?;
        Ok(stored.view)
    }
    pub(super) async fn broadcast_icp_staking_at(
        &self,
        endpoint: &str,
        artifact_id: &str,
        payload: &str,
    ) -> Result<String, SpectraBridgeError> {
        let calls: Vec<SignedIcpStakingCall> = serde_json::from_str(payload)?;
        let client = IcpReplicaClient::new(Arc::new(vec![endpoint.into()]));
        for call in &calls {
            let stored = self.load_send_artifact(artifact_id.to_string()).await?;
            if Self::saved_icp_reply(&stored, call)? {
                continue;
            }
            let id: [u8; 32] = hex::decode(&call.request_id)?
                .try_into()
                .map_err(|_| fail("Invalid ICP ingress ID"))?;
            let read = hex::decode(&call.read_state_hex)?;
            let mut certified = client.status(&call.canister, &id, &read).await?;
            if matches!(
                certified.status,
                IngressStatus::Pending | IngressStatus::Processing
            ) {
                if certified.status == IngressStatus::Pending {
                    client
                        .submit(&call.canister, &hex::decode(&call.update_hex)?)
                        .await?;
                }
                for _ in 0..12 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    certified = client.status(&call.canister, &id, &read).await?;
                    if !matches!(
                        certified.status,
                        IngressStatus::Pending | IngressStatus::Processing
                    ) {
                        break;
                    }
                }
            }
            match &certified.status {
                IngressStatus::Replied(_) => {
                    self.save_icp_reply(artifact_id, call, &certified).await?
                }
                IngressStatus::Rejected(message) => {
                    return Err(SpectraBridgeError::failure(format!(
                        "Certified ICP staking rejection: {message}; earlier steps may have completed"
                    )));
                }
                IngressStatus::Pending
                | IngressStatus::Processing
                | IngressStatus::Unknown
                | IngressStatus::Done => {
                    return Err(SpectraBridgeError::failure(
                        "ICP staking execution is uncertain; inspect saved ingress proofs before repairing the operation",
                    ));
                }
            }
        }
        calls
            .last()
            .map(|call| call.request_id.clone())
            .ok_or_else(|| fail("Empty ICP staking submission"))
    }
}

fn unix_ns() -> Result<u64, SpectraBridgeError> {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(fail)?
            .as_nanos(),
    )
    .map_err(fail)
}

fn add_manage(
    calls: &mut Vec<IcpStakingCall>,
    governance: &candid::Principal,
    selector: NeuronSelector,
    command: ManageCommand,
    kind: IcpStakingCallKind,
    nonce_hex: Option<String>,
) -> Result<(), SpectraBridgeError> {
    let args = ManageNeuron {
        id: None,
        neuron_id_or_subaccount: Some(selector),
        command: Some(command),
    };
    calls.push(IcpStakingCall {
        canister: governance.to_text(),
        method: "manage_neuron".into(),
        argument_hex: hex::encode(candid::encode_one(args).map_err(fail)?),
        nonce_hex,
        kind,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_second_dissolve_delay_uses_execution_relative_official_sdk_argument() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/icp-staking-vectors.json"
        ))
        .unwrap();
        let mut calls = vec![];
        let governance =
            candid::Principal::from_text(Chain::Icp.icp_governance_id().unwrap()).unwrap();
        add_manage(
            &mut calls,
            &governance,
            NeuronSelector::Subaccount(
                hex::decode(fixture["subaccount"].as_str().unwrap()).unwrap(),
            ),
            increase_delay(1).unwrap(),
            IcpStakingCallKind::Configure,
            None,
        )
        .unwrap();
        // Candid permits different type-table ordering across implementations.
        // Decode the independently encoded SDK value and the local value into
        // the same official argument shape rather than pinning a table order.
        for argument in [
            calls[0].argument_hex.as_str(),
            fixture["calls"][1]["argument_hex"].as_str().unwrap(),
        ] {
            let command: ManageNeuron =
                candid::decode_one(&hex::decode(argument).unwrap()).unwrap();
            let Some(ManageCommand::Configure(Configure {
                operation: Some(ConfigureOperation::IncreaseDissolveDelay(delay)),
            })) = command.command
            else {
                panic!("relative dissolve delay")
            };
            assert_eq!(delay.additional_dissolve_delay_seconds, 1);
            let Some(NeuronSelector::Subaccount(account)) = command.neuron_id_or_subaccount else {
                panic!("derived neuron selector")
            };
            assert_eq!(hex::encode(account), fixture["subaccount"]);
            assert!(command.id.is_none());
        }
        assert!(increase_delay(0).is_err());
        assert!(increase_delay(63_115_201).is_err());
    }

    #[test]
    fn signing_rechecks_current_controller_unlock_amount_and_claimable_maturity() {
        let (key, mut prepared) = icp_staking::tests::new_neuron_fixture();
        let owner = controller(&key);
        let mut request = StakingRequest {
            chain_id: Chain::Icp,
            wallet_id: "wallet".into(),
            action: StakingAction::Unstake,
            position_id: Some("42".into()),
            validator_id: None,
            amount: None,
            lockup_seconds: None,
        };
        let mut neuron = Neuron {
            id: Some(NeuronId { id: 42 }),
            controller: Some(owner),
            account: hex::decode(&prepared.subaccount_hex).unwrap(),
            cached_neuron_stake_e8s: prepared.amount,
            neuron_fees_e8s: 0,
            maturity_e8s_equivalent: 200_000_000,
            staked_maturity_e8s_equivalent: None,
            dissolve_state: Some(DissolveState::DissolveDelaySeconds(600)),
            spawn_at_timestamp_seconds: None,
            maturity_disbursements_in_progress: None,
        };
        fn set_packet(
            prepared: &mut PreparedIcpStaking,
            request: &StakingRequest,
            neuron: &Neuron,
        ) {
            let (command, kind) = match request.action {
                StakingAction::Unstake => (
                    ManageCommand::Configure(Configure {
                        operation: Some(ConfigureOperation::StartDissolving(Empty {})),
                    }),
                    IcpStakingCallKind::Configure,
                ),
                StakingAction::Withdraw => (
                    withdraw_command(neuron, &prepared.sender, None).unwrap(),
                    IcpStakingCallKind::Disburse,
                ),
                StakingAction::ClaimRewards => (
                    claim_maturity_command(&prepared.sender).unwrap(),
                    IcpStakingCallKind::DisburseMaturity,
                ),
                _ => panic!("existing action"),
            };
            prepared.calls.clear();
            add_manage(
                &mut prepared.calls,
                &candid::Principal::from_text(Chain::Icp.icp_governance_id().unwrap()).unwrap(),
                NeuronSelector::NeuronId(neuron.id.clone().unwrap()),
                command,
                kind,
                None,
            )
            .unwrap();
        }
        prepared.fee = 0;
        set_packet(&mut prepared, &request, &neuron);
        validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 0).unwrap();
        neuron.controller = Some(candid::Principal::anonymous());
        assert!(
            validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 0).is_err()
        );
        neuron.controller = Some(owner);
        request.action = StakingAction::Withdraw;
        prepared.fee = 10_000;
        set_packet(&mut prepared, &request, &neuron);
        assert!(
            validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000)
                .is_err()
        );
        neuron.dissolve_state = Some(DissolveState::DissolveDelaySeconds(0));
        validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000).unwrap();
        neuron.cached_neuron_stake_e8s -= 1;
        assert!(
            validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000)
                .is_err()
        );
        request.action = StakingAction::ClaimRewards;
        prepared.amount = neuron.maturity_e8s_equivalent;
        prepared.fee = 0;
        set_packet(&mut prepared, &request, &neuron);
        validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000).unwrap();
        neuron.maturity_e8s_equivalent = 0;
        assert!(
            validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000)
                .is_err()
        );
    }
    #[test]
    fn full_and_partial_withdrawal_do_not_deduct_neuron_fees_twice() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/icp-staking-vectors.json"
        ))
        .unwrap();
        let (_, prepared) = icp_staking::tests::new_neuron_fixture();
        let neuron = Neuron {
            id: Some(NeuronId { id: 42 }),
            controller: None,
            account: hex::decode(&prepared.subaccount_hex).unwrap(),
            cached_neuron_stake_e8s: 200_000_000,
            neuron_fees_e8s: 10_000,
            maturity_e8s_equivalent: 0,
            staked_maturity_e8s_equivalent: None,
            dissolve_state: Some(DissolveState::DissolveDelaySeconds(0)),
            spawn_at_timestamp_seconds: None,
            maturity_disbursements_in_progress: None,
        };
        let full_sdk: ManageNeuron = candid::decode_one(
            &hex::decode(fixture["full_disburse_argument_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        let partial_sdk: ManageNeuron = candid::decode_one(
            &hex::decode(fixture["calls"][4]["argument_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        for (command, expected) in [
            (
                withdraw_command(&neuron, &prepared.sender, None).unwrap(),
                None,
            ),
            (full_sdk.command.unwrap(), None),
            (
                withdraw_command(&neuron, &prepared.sender, Some(100_000_000)).unwrap(),
                Some(100_010_000),
            ),
            (partial_sdk.command.unwrap(), Some(100_010_000)),
        ] {
            let ManageCommand::Disburse(disburse) = command else {
                panic!("Disburse")
            };
            assert_eq!(disburse.amount.as_ref().map(|amount| amount.e8s), expected);
            assert_eq!(
                hex::encode(disburse.to_account.unwrap().hash),
                prepared.sender
            );
            let principal = disburse
                .amount
                .as_ref()
                .map_or(net_stake(&neuron).unwrap(), |amount| {
                    amount.e8s - neuron.neuron_fees_e8s
                });
            assert_eq!(
                principal - 10_000,
                if expected.is_some() {
                    99_990_000
                } else {
                    199_980_000
                }
            );
        }
        assert!(withdraw_command(&neuron, &prepared.sender, Some(u64::MAX)).is_err());
    }

    #[test]
    fn maturity_claims_enforce_the_official_minimum_queue_limit_and_spawn_state() {
        let mut neuron = Neuron {
            id: Some(NeuronId { id: 42 }),
            controller: None,
            account: vec![0; 32],
            cached_neuron_stake_e8s: 0,
            neuron_fees_e8s: 0,
            maturity_e8s_equivalent: 100_000_000,
            staked_maturity_e8s_equivalent: None,
            dissolve_state: Some(DissolveState::DissolveDelaySeconds(0)),
            spawn_at_timestamp_seconds: None,
            maturity_disbursements_in_progress: None,
        };
        assert!(maturity_can_be_disbursed(&neuron).unwrap());
        neuron.maturity_e8s_equivalent -= 1;
        assert!(!maturity_can_be_disbursed(&neuron).unwrap());
        assert!(
            !position(&neuron, "owner", 10)
                .unwrap()
                .available_actions
                .contains(&StakingAction::ClaimRewards)
        );
        neuron.maturity_e8s_equivalent += 1;
        neuron.spawn_at_timestamp_seconds = Some(10);
        assert!(!maturity_can_be_disbursed(&neuron).unwrap());
        neuron.spawn_at_timestamp_seconds = None;
        let pending = MaturityDisbursement {
            amount_e8s: Some(100_000_000),
            timestamp_of_disbursement_seconds: Some(1),
            finalize_disbursement_timestamp_seconds: Some(604_801),
        };
        neuron.maturity_disbursements_in_progress = Some(vec![pending; 10]);
        assert!(!maturity_can_be_disbursed(&neuron).unwrap());
        neuron
            .maturity_disbursements_in_progress
            .as_mut()
            .unwrap()
            .pop();
        assert!(maturity_can_be_disbursed(&neuron).unwrap());
    }
    #[test]
    fn processing_and_unexpired_absent_ingress_cannot_be_renewed() {
        assert!(require_repairable_ingress(&IngressStatus::Processing, 100, 200).is_err());
        assert!(require_repairable_ingress(&IngressStatus::Pending, 100, 99).is_err());
        assert!(require_repairable_ingress(&IngressStatus::Pending, 100, 100).is_ok());
        // Pruning permits a fresh repair-state check. Neither state proves
        // funding success; repair still needs its independent exact proof.
        assert!(require_repairable_ingress(&IngressStatus::Unknown, 100, 200).is_ok());
        assert!(require_repairable_ingress(&IngressStatus::Done, 100, 200).is_ok());
    }

    #[test]
    fn repair_does_not_relock_an_existing_dissolving_or_unlocked_neuron() {
        let mut neuron = Neuron {
            id: Some(NeuronId { id: 42 }),
            controller: None,
            account: vec![1; 32],
            cached_neuron_stake_e8s: 100,
            neuron_fees_e8s: 0,
            maturity_e8s_equivalent: 0,
            staked_maturity_e8s_equivalent: None,
            dissolve_state: Some(DissolveState::WhenDissolvedTimestampSeconds(100)),
            spawn_at_timestamp_seconds: None,
            maturity_disbursements_in_progress: None,
        };
        assert_eq!(existing_neuron_delay(&neuron, 10).unwrap(), Some(90));
        assert_eq!(existing_neuron_delay(&neuron, 101).unwrap(), Some(0));
        neuron.dissolve_state = Some(DissolveState::DissolveDelaySeconds(0));
        assert_eq!(existing_neuron_delay(&neuron, 10).unwrap(), None);
        neuron.dissolve_state = None;
        assert!(existing_neuron_delay(&neuron, 10).is_err());
    }
    #[test]
    fn independent_governance_queries_keep_pending_maturity_separate_from_claimable_rewards() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/icp-staking-vectors.json"
        ))
        .unwrap();
        let bytes = hex::decode(fixture["query_replies"]["queued"].as_str().unwrap()).unwrap();
        let envelope: ciborium::Value = ciborium::from_reader(bytes.as_slice()).unwrap();
        let envelope = match envelope {
            ciborium::Value::Tag(_, value) => *value,
            value => value,
        };
        let ciborium::Value::Map(map) = envelope else {
            panic!("query envelope")
        };
        let reply = map
            .iter()
            .find(|(key, _)| key == &ciborium::Value::Text("reply".into()))
            .unwrap();
        let ciborium::Value::Map(reply) = &reply.1 else {
            panic!("query reply")
        };
        let arg = reply
            .iter()
            .find(|(key, _)| key == &ciborium::Value::Text("arg".into()))
            .unwrap();
        let ciborium::Value::Bytes(arg) = &arg.1 else {
            panic!("Candid reply")
        };
        let response: ListNeuronsResponse = candid::decode_one(arg).unwrap();
        let pending = position(
            &response.full_neurons[0],
            fixture["owner"].as_str().unwrap(),
            1_800_000_000,
        )
        .unwrap();
        assert_eq!(
            pending.claimable_rewards_smallest_unit.as_deref(),
            Some("0")
        );
        assert_eq!(
            pending.pending_rewards_smallest_unit.as_deref(),
            Some("1000000")
        );
        assert!(pending.rewards_unlock_time_unix.is_some());
        assert!(
            !pending
                .available_actions
                .contains(&StakingAction::ClaimRewards)
        );
    }

    #[test]
    fn neuron_positions_follow_real_dissolve_state_and_control() {
        let mut n = Neuron {
            id: Some(NeuronId { id: 42 }),
            controller: None,
            account: vec![1; 32],
            cached_neuron_stake_e8s: 100,
            neuron_fees_e8s: 2,
            maturity_e8s_equivalent: 3,
            staked_maturity_e8s_equivalent: None,
            dissolve_state: Some(DissolveState::DissolveDelaySeconds(100)),
            spawn_at_timestamp_seconds: None,
            maturity_disbursements_in_progress: None,
        };
        let active = position(&n, "owner", 10).unwrap();
        assert_eq!(active.staked_amount_smallest_unit, "98");
        assert!(active.available_actions.contains(&StakingAction::Unstake));
        n.dissolve_state = Some(DissolveState::WhenDissolvedTimestampSeconds(100));
        assert_eq!(
            position(&n, "owner", 10).unwrap().status,
            StakingPositionStatus::Unbonding
        );
        assert_eq!(
            position(&n, "owner", 101)
                .unwrap()
                .withdrawable_amount_smallest_unit,
            "98"
        );
        n.neuron_fees_e8s = 101;
        assert!(position(&n, "owner", 101).is_err());
    }
}

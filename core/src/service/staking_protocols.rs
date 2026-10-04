//! Account-chain staking uses the same immutable transaction and broadcast journal
//! as transfers. The chain's owned accounts, pools and objects authorise operations.
use super::*;
use crate::api::{aptos_indexer::AptosIndexerClient, fastnear::FastnearClient};
use crate::send::stages::{PreparedPayload, StoredSend};
use crate::staking::{StakingAction, StakingPosition, StakingPositionStatus, StakingRequest};
use futures::{StreamExt, TryStreamExt, stream};

fn staking_amount(request: &StakingRequest) -> Result<u128, SpectraBridgeError> {
    let value = request
        .amount
        .as_deref()
        .ok_or_else(|| SpectraBridgeError::invalid("Review an explicit staking amount"))?;
    let amount = crate::send::amount_input::parse_raw_amount(
        value,
        u32::from(request.chain_id.native_decimals()),
    )?;
    if amount == 0 {
        return Err(SpectraBridgeError::invalid(
            "Staking amount must be positive",
        ));
    }
    Ok(amount)
}
fn staking_target(request: &StakingRequest) -> Result<&str, SpectraBridgeError> {
    let target = if request.action == StakingAction::Stake {
        request.validator_id.as_deref()
    } else {
        request.position_id.as_deref()
    };
    target
        .ok_or_else(|| SpectraBridgeError::invalid("Select an owned staking position or validator"))
}
fn integer(body: &serde_json::Value, key: &str) -> Result<u64, SpectraBridgeError> {
    body[key]
        .as_str()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| SpectraBridgeError::invalid(format!("Missing staking {key}")))
}
fn require_balance(balance: u128, amount: u128, fee: u128) -> Result<(), SpectraBridgeError> {
    let required = amount
        .checked_add(fee)
        .ok_or_else(|| SpectraBridgeError::invalid("Staking balance overflow"))?;
    if balance < required {
        return Err(SpectraBridgeError::invalid(
            "Insufficient native balance for staking and network fee",
        ));
    }
    Ok(())
}
fn require_supported_action(request: &StakingRequest) -> Result<(), SpectraBridgeError> {
    if request.chain_id.is_testnet()
        || !matches!(
            request.chain_id,
            Chain::Solana | Chain::Sui | Chain::Aptos | Chain::Near
        )
    {
        return Err(SpectraBridgeError::invalid("Unsupported staking network"));
    }
    if request.action == StakingAction::ClaimRewards {
        return Err(SpectraBridgeError::invalid(
            "This protocol compounds rewards into stake; unstake and withdraw the owned position",
        ));
    }
    if request.action == StakingAction::Stake && request.position_id.is_some() {
        return Err(SpectraBridgeError::invalid(
            "A new stake selects a validator, not an existing position",
        ));
    }
    Ok(())
}

impl WalletService {
    async fn account_staking_endpoints(
        &self,
        chain: Chain,
    ) -> Result<Arc<Vec<String>>, SpectraBridgeError> {
        let api = chain
            .endpoint_apis()
            .first()
            .copied()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing staking API"))?;
        let endpoints = self
            .api_endpoints(chain, api, &[EndpointCapability::Staking])
            .await?;
        if endpoints.is_empty() {
            return Err(SpectraBridgeError::invalid(
                "No staking RPC endpoints configured",
            ));
        }
        Ok(Arc::new(endpoints))
    }

    pub(super) async fn prepare_account_staking(
        &self,
        request: &StakingRequest,
        owner: &str,
        public_key_hex: Option<&str>,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        require_supported_action(request)?;
        let amount = staking_amount(request)?;
        let target = staking_target(request)?;
        let chain = request.chain_id;
        if !crate::send::flow::is_valid_send_address(chain, target.to_string()) {
            return Err(SpectraBridgeError::invalid(
                "Invalid staking target for selected network",
            ));
        }
        let endpoints = self.account_staking_endpoints(chain).await?;
        crate::api::http::race(&endpoints,|endpoint|async move{
            self.validate_endpoint_network(chain,&endpoint).await?;
            let endpoints=Arc::new(vec![endpoint]);
            Ok(match chain {
                Chain::Solana=>{
                    let client=SolanaClient::new(endpoints);
                    let amount=u64::try_from(amount).map_err(SpectraBridgeError::invalid)?;
                    let blockhash=client.fetch_recent_blockhash().await?;
                    let (rent,seed)=if request.action==StakingAction::Stake{
                        let validators=client.fetch_staking_validators().await?;
                        if !validators.current.iter().any(|v|v.vote_pubkey==target){return Err(SpectraBridgeError::invalid("Select an active Solana vote account"));}
                        if amount<client.fetch_stake_minimum().await?{return Err(SpectraBridgeError::invalid("Stake is below the live minimum delegation"));}
                        (client.fetch_stake_rent().await?,Some(crate::store::new_event_id().replace('-',"")))
                    }else{
                        validate_sol_position(&client,owner,target,request.action,amount).await?;
                        (0,None)
                    };
                    let mut prepared=crate::send::solana::prepare_staking_data(owner,target,amount,rent,&blockhash,seed.as_deref(),request.action)?;
                    let fee=client.fetch_staking_message_fee(&prepared.message).await?;
                    prepared.network_fee=Some(fee);
                    let debit=if request.action==StakingAction::Stake{amount.checked_add(rent).ok_or_else(||SpectraBridgeError::invalid("Solana stake rent overflow"))?}else{0};
                    require_balance(u128::from(client.fetch_balance(owner).await?.lamports),u128::from(debit),u128::from(fee))?;
                    if !client.simulate_staking_message(&prepared.message).await?{return Err(SpectraBridgeError::invalid("Solana staking simulation refused this authority, amount or unlock state"));}
                    PreparedPayload::Solana(prepared)
                }
                Chain::Sui=>{
                    let client=SuiClient::new(endpoints);
                    let amount=u64::try_from(amount).map_err(SpectraBridgeError::invalid)?;
                    client.validate_staking_system(chain).await?;
                    let prepared=if request.action==StakingAction::Stake{
                        if amount<chain.sui_staking_minimum().unwrap(){return Err(SpectraBridgeError::invalid("Sui stake is below the protocol minimum"));}
                        if !client.fetch_staking_validators().await?.active_validators.iter().any(|v|v.sui_address==target){return Err(SpectraBridgeError::invalid("Select an active Sui validator"));}
                        crate::send::sui::prepare_staking(&client,owner,Some(target),amount,None,Chain::Sui.sui_staking_gas_budget().unwrap()).await?
                    }else{
                        let object=validate_sui_position(&client,owner,target,amount).await?;
                        crate::send::sui::prepare_staking(&client,owner,None,0,Some((target,&object)),Chain::Sui.sui_staking_gas_budget().unwrap()).await?
                    };
                    client.simulate_staking(&prepared.bytes).await?;
                    PreparedPayload::Sui(prepared)
                }
                Chain::Aptos=>{
                    let client=AptosClient::new(endpoints);
                    let amount=u64::try_from(amount).map_err(SpectraBridgeError::invalid)?;
                    validate_aptos_position(&client,owner,target,request.action,amount).await?;
                    let price=client.fetch_gas_price().await?;
                    let limit=chain.aptos_max_gas_amount().ok_or_else(||SpectraBridgeError::invalid("Missing Aptos gas limit"))?;
                    let public:[u8;32]=hex::decode(public_key_hex.ok_or_else(||SpectraBridgeError::invalid("Missing Aptos staking public key"))?)?.try_into().map_err(|_|SpectraBridgeError::invalid("Invalid Aptos public key"))?;
                    let sequence=client.fetch_account_info(owner).await?.0;let expiry=crate::store::now_unix() as u64+600;
                    let initial=crate::send::aptos::prepare_delegation(owner,target,amount,sequence,price,limit,expiry,chain.aptos_chain_id().unwrap(),request.action)?;
                    let (used,maximum)=client.simulate_staking(&initial.body,&public,true,amount).await?;
                    let gas=used.checked_mul(6).and_then(|v|v.checked_add(4)).map(|v|v/5).and_then(|v|v.checked_add(100)).ok_or_else(||SpectraBridgeError::invalid("Aptos simulated gas overflow"))?.min(maximum);
                    let fee=price.checked_mul(gas).ok_or_else(||SpectraBridgeError::invalid("Aptos gas overflow"))?;
                    require_balance(u128::from(client.fetch_balance(owner).await?.octas),if request.action==StakingAction::Stake{u128::from(amount)}else{0},u128::from(fee))?;
                    let mut prepared=crate::send::aptos::prepare_delegation(owner,target,amount,sequence,price,gas,expiry,chain.aptos_chain_id().unwrap(),request.action)?;
                    client.simulate_staking(&prepared.body,&public,false,amount).await?;
                    prepared.staking_public_key=Some(public);
                    PreparedPayload::Aptos(prepared)
                }
                Chain::Near=>{
                    let client=NearClient::new(endpoints);
                    validate_near_position(&client,owner,target,request.action,amount).await?;
                    let public:[u8;32]=hex::decode(public_key_hex.ok_or_else(||SpectraBridgeError::invalid("Missing NEAR staking public key"))?)?.try_into().map_err(|_|SpectraBridgeError::invalid("Invalid NEAR public key"))?;
                    let nonce=client.fetch_full_access_key_nonce(owner,&bs58::encode(public).into_string()).await?.checked_add(1).ok_or_else(||SpectraBridgeError::invalid("NEAR access-key nonce overflow"))?;
                    let deposit=if request.action==StakingAction::Stake{amount}else{0};
                    let method=match request.action{StakingAction::Stake=>"deposit_and_stake",StakingAction::Unstake=>"unstake",_=>"withdraw"};
                    let args=serde_json::to_vec(&if request.action==StakingAction::Stake{serde_json::json!({})}else{serde_json::json!({"amount":amount.to_string()})})?;
                    let fee=client.function_call_fee_budget(owner,target,method,args.len(),Chain::Near.near_staking_gas_limit().unwrap()).await?;
                    require_balance(client.fetch_spendable_balance(owner).await?,deposit,fee)?;
                    let hash=crate::derivation::solana::decode_b58_32(&client.fetch_latest_block_hash().await?)?;
                    let mut prepared=crate::send::near::PreparedNearFunctionCall::prepare(owner,public,nonce,target,method,args,Chain::Near.near_staking_gas_limit().unwrap(),deposit,hash);
                    prepared.fee_budget=fee.to_string();
                    PreparedPayload::NearFunctionCall(prepared)
                }
                _=>return Err(SpectraBridgeError::invalid("Unsupported staking network")),
            })
        }).await
    }

    pub(super) async fn fetch_account_staking_positions(
        &self,
        chain: Chain,
        owner: &str,
        _public_key_hex: Option<&str>,
        known_targets: &[String],
    ) -> Result<Vec<StakingPosition>, SpectraBridgeError> {
        let owner = owner.to_string();
        let endpoints = self.account_staking_endpoints(chain).await?;
        let mut targets = known_targets.to_vec();
        if chain == Chain::Aptos {
            let urls = self
                .api_endpoints(
                    chain,
                    crate::EndpointApi::AptosIndexer,
                    &[EndpointCapability::History],
                )
                .await?;
            targets.extend(
                AptosIndexerClient::new(Arc::new(urls), chain.aptos_chain_id().unwrap())
                    .fetch_delegation_pool_ids(&owner)
                    .await?,
            );
        } else if chain == Chain::Near {
            let urls = self
                .api_endpoints(
                    chain,
                    crate::EndpointApi::Fastnear,
                    &[EndpointCapability::Staking],
                )
                .await?;
            targets.extend(
                FastnearClient::new(Arc::new(urls))
                    .fetch_staking_pool_ids(&owner)
                    .await?,
            );
        }
        targets.sort();
        targets.dedup();
        crate::api::http::race(&endpoints, |endpoint| {
            let targets = targets.clone();
            let owner = owner.clone();
            Box::pin(async move {
                self.validate_endpoint_network(chain, &endpoint).await?;
                let endpoints = Arc::new(vec![endpoint]);
                match chain {
                    Chain::Solana => {
                        let client = SolanaClient::new(endpoints);
                        let epoch = client.fetch_staking_epoch().await?;
                        let blockhash = client.fetch_recent_blockhash().await?;
                        let mut positions = Vec::new();
                        for account in client.fetch_stake_accounts(&owner).await? {
                            let can_unstake = account.staker == owner
                                && account.deactivation_epoch == Some(u64::MAX);
                            let withdrawal = crate::send::solana::prepare_staking_data(
                                &owner,
                                &account.address,
                                account.lamports,
                                0,
                                &blockhash,
                                None,
                                StakingAction::Withdraw,
                            )?;
                            let fully_withdrawable = account.withdrawer == owner
                                && client.simulate_staking_message(&withdrawal.message).await?;
                            let mut withdrawable = if fully_withdrawable {
                                account.lamports
                            } else {
                                0
                            };
                            // Uncommitted lamports can leave an active stake account
                            // without removing its delegated principal or rent reserve.
                            let surplus = account.lamports.saturating_sub(
                                account.rent_reserve.saturating_add(account.delegated),
                            );
                            if !fully_withdrawable && surplus > 0 && account.withdrawer == owner {
                                let partial = crate::send::solana::prepare_staking_data(
                                    &owner,
                                    &account.address,
                                    surplus,
                                    0,
                                    &blockhash,
                                    None,
                                    StakingAction::Withdraw,
                                )?;
                                if client.simulate_staking_message(&partial.message).await? {
                                    withdrawable = surplus;
                                }
                            }
                            let unbonding =
                                account.deactivation_epoch.is_some_and(|v| v != u64::MAX)
                                    && !fully_withdrawable;
                            let mut actions = Vec::new();
                            if can_unstake {
                                actions.push(StakingAction::Unstake);
                            }
                            if withdrawable > 0 {
                                actions.push(StakingAction::Withdraw);
                            }
                            positions.push(StakingPosition {
                                id: account.address,
                                owner: owner.clone(),
                                validator_identifier: account.vote.unwrap_or_default(),
                                status: if fully_withdrawable {
                                    StakingPositionStatus::Withdrawable
                                } else if unbonding {
                                    StakingPositionStatus::Unbonding
                                } else if account.activation_epoch.is_some_and(|v| v >= epoch) {
                                    StakingPositionStatus::Activating
                                } else {
                                    StakingPositionStatus::Active
                                },
                                staked_amount_smallest_unit: if unbonding || fully_withdrawable {
                                    "0".into()
                                } else {
                                    account.delegated.to_string()
                                },
                                unbonding_amount_smallest_unit: if unbonding {
                                    account.delegated.to_string()
                                } else {
                                    "0".into()
                                },
                                withdrawable_amount_smallest_unit: withdrawable.to_string(),
                                claimable_rewards_smallest_unit: None,
                                pending_rewards_smallest_unit: None,
                                rewards_unlock_time_unix: None,
                                unlock_epoch: (account.lockup_epoch > 0)
                                    .then_some(account.lockup_epoch),
                                unlock_time_unix: u64::try_from(account.lockup_time)
                                    .ok()
                                    .filter(|v| *v > 0),
                                available_actions: actions,
                            });
                        }
                        Ok(positions)
                    }
                    Chain::Sui => {
                        let client = SuiClient::new(endpoints);
                        let mut positions = Vec::new();
                        for group in client.fetch_delegated_stakes(&owner).await? {
                            for stake in group.stakes {
                                let amount = stake
                                    .principal
                                    .parse::<u64>()
                                    .map_err(SpectraBridgeError::invalid)?;
                                let object = validate_sui_position(
                                    &client,
                                    &owner,
                                    &stake.staked_sui_id,
                                    amount,
                                )
                                .await?;
                                if object.pool_id != group.staking_pool
                                    || Some(object.activation_epoch)
                                        != stake.stake_active_epoch.parse().ok()
                                {
                                    return Err(SpectraBridgeError::invalid(
                                        "Sui stake object and delegation snapshot differ",
                                    ));
                                }
                                positions.push(StakingPosition {
                                    id: stake.staked_sui_id,
                                    owner: owner.clone(),
                                    validator_identifier: group.validator_address.clone(),
                                    status: if stake.status == "Active" {
                                        StakingPositionStatus::Active
                                    } else {
                                        StakingPositionStatus::Activating
                                    },
                                    staked_amount_smallest_unit: stake.principal,
                                    unbonding_amount_smallest_unit: "0".into(),
                                    withdrawable_amount_smallest_unit: "0".into(),
                                    claimable_rewards_smallest_unit: stake.estimated_reward,
                                    pending_rewards_smallest_unit: None,
                                    rewards_unlock_time_unix: None,
                                    unlock_epoch: None,
                                    unlock_time_unix: None,
                                    available_actions: vec![StakingAction::Unstake],
                                });
                            }
                        }
                        Ok(positions)
                    }
                    Chain::Aptos => {
                        let client = Arc::new(AptosClient::new(endpoints));
                        stream::iter(targets)
                            .map(|pool| {
                                let client = client.clone();
                                let owner = owner.clone();
                                async move {
                                    let value = client
                                        .fetch_delegation(&pool, &owner)
                                        .await?
                                        .ok_or_else(|| {
                                            SpectraBridgeError::invalid(
                                                "Indexed Aptos position is not a delegation pool",
                                            )
                                        })?;
                                    let unlocked = value
                                        .inactive
                                        .checked_add(value.pending_inactive)
                                        .ok_or_else(|| {
                                            SpectraBridgeError::invalid("Aptos stake overflow")
                                        })?;
                                    let mut actions = Vec::new();
                                    if value.active > 0 {
                                        actions.push(StakingAction::Unstake);
                                    }
                                    if value.withdrawable > 0 {
                                        actions.push(StakingAction::Withdraw);
                                    }
                                    Ok::<_, SpectraBridgeError>(
                                        (value.active > 0 || unlocked > 0).then(|| {
                                            StakingPosition {
                                                id: pool.clone(),
                                                owner: owner.clone(),
                                                validator_identifier: pool.clone(),
                                                status: if value.withdrawable > 0 {
                                                    StakingPositionStatus::Withdrawable
                                                } else if unlocked > 0 {
                                                    StakingPositionStatus::Unbonding
                                                } else {
                                                    StakingPositionStatus::Active
                                                },
                                                staked_amount_smallest_unit: value
                                                    .active
                                                    .to_string(),
                                                unbonding_amount_smallest_unit: unlocked
                                                    .saturating_sub(value.withdrawable)
                                                    .to_string(),
                                                withdrawable_amount_smallest_unit: value
                                                    .withdrawable
                                                    .to_string(),
                                                claimable_rewards_smallest_unit: None,
                                                pending_rewards_smallest_unit: None,
                                                rewards_unlock_time_unix: None,
                                                unlock_epoch: None,
                                                unlock_time_unix: (value.withdrawable == 0
                                                    && unlocked > 0)
                                                    .then_some(value.locked_until),
                                                available_actions: actions,
                                            }
                                        }),
                                    )
                                }
                            })
                            .buffer_unordered(8)
                            .try_collect::<Vec<_>>()
                            .await
                            .map(|positions| positions.into_iter().flatten().collect())
                    }
                    Chain::Near => {
                        let client = Arc::new(NearClient::new(endpoints));
                        stream::iter(targets)
                            .map(|pool| {
                                let client = client.clone();
                                let owner = owner.clone();
                                async move {
                                    let value =
                                        client.fetch_staking_pool(chain, &pool, &owner).await?;
                                    let active = value
                                        .account
                                        .staked_balance
                                        .parse::<u128>()
                                        .map_err(SpectraBridgeError::invalid)?;
                                    let unlocked = value
                                        .account
                                        .unstaked_balance
                                        .parse::<u128>()
                                        .map_err(SpectraBridgeError::invalid)?;
                                    let ready = value.account.can_withdraw && unlocked > 0;
                                    let mut actions = Vec::new();
                                    if active > 0 {
                                        actions.push(StakingAction::Unstake);
                                    }
                                    if ready {
                                        actions.push(StakingAction::Withdraw);
                                    }
                                    Ok::<_, SpectraBridgeError>((active > 0 || unlocked > 0).then(
                                        || StakingPosition {
                                            id: pool.clone(),
                                            owner: owner.clone(),
                                            validator_identifier: pool.clone(),
                                            status: if ready {
                                                StakingPositionStatus::Withdrawable
                                            } else if unlocked > 0 {
                                                StakingPositionStatus::Unbonding
                                            } else {
                                                StakingPositionStatus::Active
                                            },
                                            staked_amount_smallest_unit: active.to_string(),
                                            unbonding_amount_smallest_unit: if ready {
                                                "0".into()
                                            } else {
                                                unlocked.to_string()
                                            },
                                            withdrawable_amount_smallest_unit: if ready {
                                                unlocked.to_string()
                                            } else {
                                                "0".into()
                                            },
                                            claimable_rewards_smallest_unit: None,
                                            pending_rewards_smallest_unit: None,
                                            rewards_unlock_time_unix: None,
                                            unlock_epoch: None,
                                            unlock_time_unix: None,
                                            available_actions: actions,
                                        },
                                    ))
                                }
                            })
                            .buffer_unordered(8)
                            .try_collect::<Vec<_>>()
                            .await
                            .map(|positions| positions.into_iter().flatten().collect())
                    }
                    _ => Err(SpectraBridgeError::invalid("Unsupported staking network")),
                }
            })
                as futures::future::BoxFuture<'_, Result<Vec<StakingPosition>, SpectraBridgeError>>
        })
        .await
    }

    pub(super) async fn validate_account_staking_state(
        &self,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let request = stored
            .view
            .staking
            .as_ref()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing staking intent"))?;
        require_supported_action(request)?;
        let owner = &stored.view.sender;
        let chain = request.chain_id;
        if chain != stored.view.chain_id || request.wallet_id != stored.view.wallet_id {
            return Err(SpectraBridgeError::invalid(
                "Staking wallet or network differs from the reviewed artifact",
            ));
        }
        let amount = staking_amount(request)?;
        let target = staking_target(request)?;
        let endpoints = self.account_staking_endpoints(chain).await?;
        crate::api::http::race(&endpoints, |endpoint| async move {
            self.validate_endpoint_network(chain, &endpoint).await?;
            let endpoints = Arc::new(vec![endpoint]);
            match &stored.prepared {
                PreparedPayload::Solana(prepared) if chain == Chain::Solana => {
                    let client = SolanaClient::new(endpoints);
                    let amount = u64::try_from(amount).map_err(SpectraBridgeError::invalid)?;
                    let rent = if request.action == StakingAction::Stake {
                        if amount < client.fetch_stake_minimum().await?
                            || !client
                                .fetch_staking_validators()
                                .await?
                                .current
                                .iter()
                                .any(|v| v.vote_pubkey == target)
                        {
                            return Err(SpectraBridgeError::invalid(
                                "Solana validator or minimum delegation changed",
                            ));
                        }
                        client.fetch_stake_rent().await?
                    } else {
                        validate_sol_position(&client, owner, target, request.action, amount)
                            .await?;
                        0
                    };
                    let expected = crate::send::solana::prepare_staking_data(
                        owner,
                        target,
                        amount,
                        rent,
                        &prepared.blockhash,
                        prepared.account_seed.as_deref(),
                        request.action,
                    )?;
                    if expected.message != prepared.message {
                        return Err(SpectraBridgeError::invalid(
                            "Solana staking data or rent changed; review again",
                        ));
                    }
                    let fee = client.fetch_staking_message_fee(&prepared.message).await?;
                    if prepared.network_fee.is_none_or(|reviewed| fee > reviewed)
                        || prepared.stake_rent != Some(rent)
                    {
                        return Err(SpectraBridgeError::invalid(
                            "Solana fee or rent exceeds reviewed amount",
                        ));
                    }
                    require_balance(
                        u128::from(client.fetch_balance(owner).await?.lamports),
                        if request.action == StakingAction::Stake {
                            u128::from(amount) + u128::from(rent)
                        } else {
                            0
                        },
                        u128::from(fee),
                    )?;
                    if !client.simulate_staking_message(&prepared.message).await? {
                        return Err(SpectraBridgeError::invalid(
                            "Solana staking authority or withdrawal is no longer valid",
                        ));
                    }
                }
                PreparedPayload::Sui(prepared) if chain == Chain::Sui => {
                    let client = SuiClient::new(endpoints);
                    let amount = u64::try_from(amount).map_err(SpectraBridgeError::invalid)?;
                    client.validate_staking_system(chain).await?;
                    let mut gas = prepared.objects.clone();
                    let object = if request.action == StakingAction::Stake {
                        if !client
                            .fetch_staking_validators()
                            .await?
                            .active_validators
                            .iter()
                            .any(|v| v.sui_address == target)
                        {
                            return Err(SpectraBridgeError::invalid(
                                "Sui validator is no longer active",
                            ));
                        }
                        if amount < chain.sui_staking_minimum().unwrap() {
                            return Err(SpectraBridgeError::invalid(
                                "Sui stake is below the protocol minimum",
                            ));
                        }
                        None
                    } else {
                        let object = validate_sui_position(&client, owner, target, amount).await?;
                        let id = hex::decode(target.trim_start_matches("0x"))?;
                        let stored = gas.pop().ok_or_else(|| {
                            SpectraBridgeError::invalid("Missing reviewed Sui stake object")
                        })?;
                        if stored.id.as_slice() != id
                            || stored.version != object.version
                            || stored.digest != object.digest
                        {
                            return Err(SpectraBridgeError::invalid(
                                "Sui stake object changed; review again",
                            ));
                        }
                        Some(object)
                    };
                    for coin in &gas {
                        let address = format!("0x{}", hex::encode(coin.id));
                        // Fetch exact object metadata rather than assuming a coin is
                        // still owned because it was on a previous bounded page.
                        client
                            .validate_owned_sui_coin(
                                &address,
                                owner,
                                coin.version,
                                &coin.digest,
                                coin.balance,
                            )
                            .await?;
                    }
                    let expected = crate::send::sui::prepare_staking_data(
                        owner,
                        (request.action == StakingAction::Stake).then_some(target),
                        if request.action == StakingAction::Stake {
                            amount
                        } else {
                            0
                        },
                        object.as_ref().map(|o| (target, o)),
                        Chain::Sui.sui_staking_gas_budget().unwrap(),
                        client.fetch_reference_gas_price().await?,
                        &gas,
                    )?;
                    if expected.bytes != prepared.bytes
                        || prepared.gas_budget != Chain::Sui.sui_staking_gas_budget().unwrap()
                    {
                        return Err(SpectraBridgeError::invalid(
                            "Sui staking object, amount or gas changed; review again",
                        ));
                    }
                    client.simulate_staking(&prepared.bytes).await?;
                }
                PreparedPayload::Aptos(prepared) if chain == Chain::Aptos => {
                    let client = AptosClient::new(endpoints);
                    let amount = u64::try_from(amount).map_err(SpectraBridgeError::invalid)?;
                    validate_aptos_position(&client, owner, target, request.action, amount).await?;
                    let sequence = integer(&prepared.body, "sequence_number")?;
                    let price = integer(&prepared.body, "gas_unit_price")?;
                    let gas = integer(&prepared.body, "max_gas_amount")?;
                    let expiry = integer(&prepared.body, "expiration_timestamp_secs")?;
                    if client.fetch_account_info(owner).await?.0 != sequence
                        || expiry <= crate::store::now_unix() as u64
                    {
                        return Err(SpectraBridgeError::invalid(
                            "Aptos staking sequence or expiration changed",
                        ));
                    }
                    let expected = crate::send::aptos::prepare_delegation(
                        owner,
                        target,
                        amount,
                        sequence,
                        price,
                        gas,
                        expiry,
                        chain.aptos_chain_id().unwrap(),
                        request.action,
                    )?;
                    if expected.message != prepared.message || expected.body != prepared.body {
                        return Err(SpectraBridgeError::invalid(
                            "Aptos staking data differs from reviewed intent",
                        ));
                    }
                    client
                        .simulate_staking(
                            &prepared.body,
                            prepared.staking_public_key.as_ref().ok_or_else(|| {
                                SpectraBridgeError::invalid("Missing Aptos simulation key")
                            })?,
                            false,
                            amount,
                        )
                        .await?;
                    require_balance(
                        u128::from(client.fetch_balance(owner).await?.octas),
                        if request.action == StakingAction::Stake {
                            u128::from(amount)
                        } else {
                            0
                        },
                        u128::from(price)
                            .checked_mul(u128::from(gas))
                            .ok_or_else(|| SpectraBridgeError::invalid("Aptos gas overflow"))?,
                    )?;
                }
                PreparedPayload::NearFunctionCall(prepared) if chain == Chain::Near => {
                    let client = NearClient::new(endpoints);
                    validate_near_position(&client, owner, target, request.action, amount).await?;
                    let nonce = client
                        .fetch_full_access_key_nonce(
                            owner,
                            &bs58::encode(prepared.public_key).into_string(),
                        )
                        .await?
                        .checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::invalid("NEAR nonce overflow"))?;
                    let method = match request.action {
                        StakingAction::Stake => "deposit_and_stake",
                        StakingAction::Unstake => "unstake",
                        _ => "withdraw",
                    };
                    let deposit = if request.action == StakingAction::Stake {
                        amount
                    } else {
                        0
                    };
                    let args = serde_json::to_vec(&if request.action == StakingAction::Stake {
                        serde_json::json!({})
                    } else {
                        serde_json::json!({"amount":amount.to_string()})
                    })?;
                    let expected = crate::send::near::PreparedNearFunctionCall::prepare(
                        owner,
                        prepared.public_key,
                        nonce,
                        target,
                        method,
                        args,
                        Chain::Near.near_staking_gas_limit().unwrap(),
                        deposit,
                        prepared.block_hash,
                    );
                    if expected.message != prepared.message {
                        return Err(SpectraBridgeError::invalid(
                            "NEAR staking intent or nonce changed; review again",
                        ));
                    }
                    let fee = client
                        .function_call_fee_budget(
                            owner,
                            target,
                            &prepared.method,
                            prepared.args.len(),
                            prepared.gas,
                        )
                        .await?;
                    if fee
                        > prepared
                            .fee_budget
                            .parse::<u128>()
                            .map_err(SpectraBridgeError::invalid)?
                    {
                        return Err(SpectraBridgeError::invalid(
                            "NEAR fee exceeds reviewed budget",
                        ));
                    }
                    require_balance(client.fetch_spendable_balance(owner).await?, deposit, fee)?;
                }
                _ => {
                    return Err(SpectraBridgeError::invalid(
                        "Staking protocol differs from reviewed network",
                    ));
                }
            }
            Ok(())
        })
        .await
    }
}

async fn validate_sol_position(
    client: &SolanaClient,
    owner: &str,
    target: &str,
    action: StakingAction,
    amount: u64,
) -> Result<(), SpectraBridgeError> {
    let account = client.fetch_stake_account(target).await?;
    if action == StakingAction::Unstake {
        if account.staker != owner
            || account.deactivation_epoch != Some(u64::MAX)
            || amount != account.delegated
        {
            return Err(SpectraBridgeError::invalid(
                "Solana unstake needs the stake authority and the complete delegated amount",
            ));
        }
    } else if account.withdrawer != owner
        || amount > account.lamports
        || (amount != account.lamports
            && amount > account.lamports.saturating_sub(account.rent_reserve))
    {
        return Err(SpectraBridgeError::invalid(
            "Solana withdrawal authority or amount is invalid",
        ));
    }
    Ok(())
}
async fn validate_sui_position(
    client: &SuiClient,
    owner: &str,
    target: &str,
    amount: u64,
) -> Result<crate::api::sui_json_rpc::SuiStakedObject, SpectraBridgeError> {
    let object = client.fetch_staked_object(target, owner).await?;
    if object.principal != amount {
        return Err(SpectraBridgeError::invalid(
            "Sui unstake returns the complete StakedSui object; review its exact principal",
        ));
    }
    Ok(object)
}
async fn validate_aptos_position(
    client: &AptosClient,
    owner: &str,
    target: &str,
    action: StakingAction,
    amount: u64,
) -> Result<(), SpectraBridgeError> {
    let pool = client
        .fetch_delegation(target, owner)
        .await?
        .ok_or_else(|| SpectraBridgeError::invalid("Aptos target is not a delegation pool"))?;
    let minimum = Chain::Aptos.aptos_delegation_minimum().unwrap();
    if action == StakingAction::Stake {
        let fee = client.fetch_add_stake_fee(target, amount).await?;
        if pool
            .active
            .checked_add(amount - fee)
            .is_none_or(|v| v < minimum)
        {
            return Err(SpectraBridgeError::invalid(
                "Aptos delegated shares would fall below the protocol minimum after add-stake fee",
            ));
        }
    }
    match action {
        StakingAction::Stake if !pool.allowlisted => Err(SpectraBridgeError::invalid(
            "Delegator is not allowed by this Aptos pool",
        )),
        StakingAction::Unstake if amount > pool.active => Err(SpectraBridgeError::invalid(
            "Aptos unlock exceeds owned active stake",
        )),
        StakingAction::Withdraw if amount > pool.withdrawable => Err(SpectraBridgeError::invalid(
            "Aptos stake is still locked or withdrawal exceeds owned stake",
        )),
        _ => Ok(()),
    }
}
async fn validate_near_position(
    client: &NearClient,
    owner: &str,
    target: &str,
    action: StakingAction,
    amount: u128,
) -> Result<(), SpectraBridgeError> {
    let pool = client
        .fetch_staking_pool(Chain::Near, target, owner)
        .await?;
    let active = pool
        .account
        .staked_balance
        .parse::<u128>()
        .map_err(SpectraBridgeError::invalid)?;
    let unstaked = pool
        .account
        .unstaked_balance
        .parse::<u128>()
        .map_err(SpectraBridgeError::invalid)?;
    match action {
        StakingAction::Stake if !pool.approved => Err(SpectraBridgeError::invalid(
            "NEAR staking pool is not in the protocol whitelist",
        )),
        StakingAction::Stake if pool.paused => {
            Err(SpectraBridgeError::invalid("NEAR staking pool is paused"))
        }
        StakingAction::Unstake if amount > active => Err(SpectraBridgeError::invalid(
            "NEAR unstake exceeds owned stake",
        )),
        StakingAction::Withdraw if !pool.account.can_withdraw || amount > unstaked => Err(
            SpectraBridgeError::invalid("NEAR withdrawal is not available for this owned position"),
        ),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::keys::Ed25519Seed;

    fn fixtures() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/account-staking-vectors.json"
        ))
        .unwrap()
    }
    fn key() -> Ed25519Seed {
        Ed25519Seed::from_hex(&hex::encode([1u8; 32])).unwrap()
    }

    #[test]
    fn solana_staking_matches_official_sdk_messages_signatures_and_derived_account() {
        for vector in fixtures()["solana"].as_array().unwrap() {
            let action = match vector["name"].as_str().unwrap() {
                "stake" => StakingAction::Stake,
                "unstake" => StakingAction::Unstake,
                _ => StakingAction::Withdraw,
            };
            let owner = vector["owner"].as_str().unwrap();
            let seed = vector["seed"].as_str().unwrap();
            assert_eq!(
                crate::send::solana::stake_account_address(owner, seed).unwrap(),
                vector["account"]
            );
            let prepared = crate::send::solana::prepare_staking_data(
                owner,
                vector[if action == StakingAction::Stake {
                    "vote"
                } else {
                    "account"
                }]
                .as_str()
                .unwrap(),
                2_000_000_000,
                if action == StakingAction::Stake {
                    2_282_880
                } else {
                    0
                },
                vector["blockhash"].as_str().unwrap(),
                (action == StakingAction::Stake).then_some(seed),
                action,
            )
            .unwrap();
            assert_eq!(hex::encode(&prepared.message), vector["message"]);
            assert_eq!(
                hex::encode(prepared.sign(&key()).unwrap()),
                vector["signed"]
            );
            assert!(
                prepared
                    .sign(&Ed25519Seed::from_hex(&hex::encode([2u8; 32])).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn aptos_delegation_calls_match_official_sdk_signatures_and_local_hashes() {
        for vector in fixtures()["aptos"].as_array().unwrap() {
            let action = match vector["name"].as_str().unwrap() {
                "add_stake" => StakingAction::Stake,
                "unlock" => StakingAction::Unstake,
                _ => StakingAction::Withdraw,
            };
            let prepared = crate::send::aptos::prepare_delegation(
                vector["owner"].as_str().unwrap(),
                &format!("0x{}", "22".repeat(32)),
                2_000_000_000,
                7,
                100,
                12_000,
                1_800_000_000,
                1,
                action,
            )
            .unwrap();
            assert_eq!(hex::encode(&prepared.message), vector["message"]);
            let (body, hash) = prepared.clone().sign(&key()).unwrap();
            let body: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(
                body["signature"]["signature"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x"),
                vector["signature"]
            );
            assert_eq!(hash, vector["hash"]);
            assert!(
                prepared
                    .sign(&Ed25519Seed::from_hex(&hex::encode([2u8; 32])).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn sui_owned_object_staking_matches_official_sdk_ptbs_signatures_and_hashes() {
        let gas = [crate::send::sui::GasCoin {
            id: [0x33; 32],
            version: 7,
            digest: [0; 32],
            balance: 3_000_000_000,
        }];
        let object = crate::api::sui_json_rpc::SuiStakedObject {
            version: 8,
            digest: [0; 32],
            principal: 2_000_000_000,
            pool_id: "0x99".into(),
            activation_epoch: 10,
        };
        let validator = format!("0x{}", "22".repeat(32));
        let stake = format!("0x{}", "44".repeat(32));
        for vector in fixtures()["sui"].as_array().unwrap() {
            let add = vector["name"] == "stake";
            let prepared = crate::send::sui::prepare_staking_data(
                vector["owner"].as_str().unwrap(),
                add.then_some(validator.as_str()),
                if add { 2_000_000_000 } else { 0 },
                (!add).then_some((stake.as_str(), &object)),
                10_000_000,
                1000,
                &gas,
            )
            .unwrap();
            assert_eq!(hex::encode(&prepared.bytes), vector["raw"]);
            assert_eq!(prepared.transaction_digest(), vector["hash"]);
            assert_eq!(
                prepared.clone().sign(&key()).unwrap().1,
                vector["signature"]
            );
            assert!(
                prepared
                    .sign(&Ed25519Seed::from_hex(&hex::encode([2u8; 32])).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn near_pool_calls_match_official_sdk_and_reject_message_field_tampering() {
        for vector in fixtures()["near"].as_array().unwrap() {
            let method = vector["name"].as_str().unwrap();
            let add = method == "deposit_and_stake";
            let mut prepared = crate::send::near::PreparedNearFunctionCall::prepare(
                "alice.near",
                key().public_key(),
                7,
                "validator.poolv1.near",
                method,
                if add {
                    b"{}".to_vec()
                } else {
                    br#"{"amount":"2000000000"}"#.to_vec()
                },
                Chain::Near.near_staking_gas_limit().unwrap(),
                if add { 2_000_000_000 } else { 0 },
                [0x33; 32],
            );
            assert_eq!(hex::encode(&prepared.message), vector["message"]);
            let (signed, hash) = prepared.sign(&key()).unwrap();
            assert_eq!(hex::encode(signed), vector["signed"]);
            assert_eq!(hash, vector["hash"]);
            prepared.receiver = "foreign.poolv1.near".into();
            assert!(prepared.sign(&key()).is_err());
        }
    }
}

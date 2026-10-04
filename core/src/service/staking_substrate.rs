//! Owned Polkadot nomination-pool intents use the normal persisted send stages.
use super::*;
use crate::api::substrate_json_rpc::{SubstrateClient, pools::PoolMember};
use crate::send::polkadot_pools::{PoolCall, encode_pool_call};
use crate::send::stages::{PreparedPayload, StoredSend};
use crate::staking::{StakingAction, StakingPosition, StakingPositionStatus, StakingRequest};

fn position_id(pool: u32) -> String {
    format!("polkadot:pool:{pool}")
}
fn refusal(message: &str) -> SpectraBridgeError {
    SpectraBridgeError::invalid(message)
}
fn action_amount(request: &StakingRequest) -> Result<u128, SpectraBridgeError> {
    let value = crate::send::amount_input::parse_raw_amount(
        request
            .amount
            .as_deref()
            .ok_or_else(|| refusal("Staking amount is required"))?,
        10,
    )?;
    if value == 0 {
        return Err(refusal("Staking amount must be positive"));
    }
    Ok(value)
}
fn owned_member<'a>(
    request: &StakingRequest,
    member: Option<&'a PoolMember>,
) -> Result<&'a PoolMember, SpectraBridgeError> {
    let member = member.ok_or_else(|| refusal("Wallet has no nomination pool position"))?;
    if request.position_id.as_deref() != Some(position_id(member.pool_id).as_str()) {
        return Err(refusal(
            "Nomination pool position does not belong to this wallet",
        ));
    }
    Ok(member)
}

impl WalletService {
    pub(super) async fn prepare_substrate_staking(
        &self,
        request: &StakingRequest,
        owner: &str,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let eps = self
            .endpoints_for(Chain::Polkadot, &[EndpointCapability::Staking])
            .await;
        crate::api::http::race(&eps,|endpoint|async move{
            let client=SubstrateClient::new(Arc::new(vec![endpoint]));
            let account=crate::derivation::polkadot::decode_ss58(owner).map_err(SpectraBridgeError::invalid)?;
            let snapshot=client.nomination_pool_snapshot(&account).await?;
            let at=&snapshot.context.block_hash;
            let mut prerequisites=Vec::new();
            if let Some(member)=snapshot.member.as_ref(){
                if member.pool_needs_migration{prerequisites.push(PoolCall::MigratePool{pool_id:member.pool_id});}
                if member.needs_migration||member.pool_needs_migration{
                    let total=member.active_balance.checked_add(member.unbonding_balance).and_then(|n|n.checked_add(member.withdrawable_balance)).ok_or_else(||refusal("Nomination pool balance overflow"))?;
                    if total<snapshot.context.runtime.existential_deposit{return Err(refusal("Nomination pool balance is below the delegation migration deposit"));}
                    prerequisites.push(PoolCall::MigrateMember{member:account});
                }
                if member.pending_slash>=snapshot.context.runtime.existential_deposit{prerequisites.push(PoolCall::ApplySlash{member:account});}
            }
            let (call,spend)=match request.action{
                StakingAction::Stake=>{
                    let amount=action_amount(request)?;
                    if let Some(member)=snapshot.member.as_ref(){
                                            if request.validator_id.as_deref().is_some_and(|id|id!=member.pool_id.to_string()){
                            return Err(refusal("Existing pool membership must be withdrawn before choosing another pool"));
                        }
                        (PoolCall::BondExtra{amount},amount)
                    }else{
                        let id=request.validator_id.as_deref().ok_or_else(||refusal("Nomination pool is required"))?;
                        let pool=id.parse::<u32>().ok().filter(|pool|*pool>0&&pool.to_string()==id).ok_or_else(||refusal("Invalid nomination pool ID"))?;
                        let pool=client.pool_at_snapshot(pool,at).await?;
                        if !pool.is_open{return Err(refusal("Nomination pool is not open"));}
                        if pool.needs_migration{prerequisites.push(PoolCall::MigratePool{pool_id:pool.id});}
                        if amount<snapshot.minimum_join{return Err(refusal("Amount is below the current nomination pool minimum"));}
                        (PoolCall::Join{amount,pool_id:pool.id},amount)
                    }
                }
                StakingAction::Unstake=>{
                    let member=owned_member(request,snapshot.member.as_ref())?;
                    let amount=action_amount(request)?;
                    if amount>member.active_balance{return Err(refusal("Amount exceeds the owned nomination pool stake"));}
                    let points=if amount==member.active_balance{member.points}else{client.pool_points_for_balance(member.pool_id,amount,at).await?};
                    if points==0||points>member.points{return Err(refusal("Invalid nomination pool unbonding points"));}
                    (PoolCall::Unbond{member:account,points},0)
                }
                StakingAction::Withdraw=>{
                    let member=owned_member(request,snapshot.member.as_ref())?;
                    if member.withdrawable_balance==0{return Err(refusal("Nomination pool funds have not finished unbonding"));}
                    (PoolCall::Withdraw{member:account,slashing_spans:0},0)
                }
                StakingAction::ClaimRewards=>{
                    let member=owned_member(request,snapshot.member.as_ref())?;
                    if member.pending_rewards==0{return Err(refusal("Nomination pool has no claimable rewards"));}
                    (PoolCall::Claim,0)
                }
            };
            let call=if prerequisites.is_empty(){call}else{prerequisites.push(call);PoolCall::BatchAll(prerequisites)};
            let metadata=client.pool_call_metadata(at).await?;
            let bytes=encode_pool_call(&metadata,&call)?;
            let prepared=crate::send::polkadot::prepare_call(&client,Chain::Polkadot,owner,snapshot.context,bytes,spend).await?;
            Ok(PreparedPayload::Substrate(prepared))
        }).await
    }
    pub(super) async fn fetch_substrate_staking_positions(
        &self,
        owner: &str,
    ) -> Result<Vec<StakingPosition>, SpectraBridgeError> {
        let eps = self
            .endpoints_for(Chain::Polkadot, &[EndpointCapability::Staking])
            .await;
        crate::api::http::race(&eps, |endpoint| async move {
            let client = SubstrateClient::new(Arc::new(vec![endpoint]));
            let account = crate::derivation::polkadot::decode_ss58(owner)
                .map_err(SpectraBridgeError::invalid)?;
            let Some(member) = client.nomination_pool_snapshot(&account).await?.member else {
                return Ok(Vec::new());
            };
            let mut available_actions = Vec::new();
            {
                available_actions.push(StakingAction::Stake);
                if member.active_balance > 0 {
                    available_actions.push(StakingAction::Unstake);
                }
                if member.withdrawable_balance > 0 {
                    available_actions.push(StakingAction::Withdraw);
                }
                if member.pending_rewards > 0 {
                    available_actions.push(StakingAction::ClaimRewards);
                }
            }
            Ok(vec![StakingPosition {
                id: position_id(member.pool_id),
                owner: owner.into(),
                validator_identifier: member.pool_id.to_string(),
                status: if member.active_balance > 0 {
                    StakingPositionStatus::Active
                } else if member.withdrawable_balance > 0 {
                    StakingPositionStatus::Withdrawable
                } else if member.unbonding_balance > 0 {
                    StakingPositionStatus::Unbonding
                } else {
                    StakingPositionStatus::Inactive
                },
                staked_amount_smallest_unit: member.active_balance.to_string(),
                unbonding_amount_smallest_unit: member.unbonding_balance.to_string(),
                withdrawable_amount_smallest_unit: member.withdrawable_balance.to_string(),
                claimable_rewards_smallest_unit: Some(member.pending_rewards.to_string()),
                pending_rewards_smallest_unit: None,
                rewards_unlock_time_unix: None,
                unlock_epoch: member.next_unlock_era.map(u64::from),
                unlock_time_unix: None,
                available_actions,
            }])
        })
        .await
    }
    pub(super) async fn validate_substrate_staking_state(
        &self,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let request = stored
            .view
            .staking
            .as_ref()
            .ok_or_else(|| refusal("Staking intent is missing"))?;
        let PreparedPayload::Substrate(prepared) = &stored.prepared else {
            return Err(refusal("Invalid nomination pool transaction"));
        };
        let PreparedPayload::Substrate(current) = self
            .prepare_substrate_staking(request, &stored.view.sender)
            .await?
        else {
            unreachable!()
        };
        if prepared.runtime != current.runtime
            || prepared.call_data != current.call_data
            || prepared.nonce != current.nonce
            || prepared.amount != current.amount
            || current.fee > prepared.fee
        {
            return Err(refusal(
                "Nomination pool state or fee changed; build and review again",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::substrate_json_rpc::pools::tests::node;
    use crate::service::ChainEndpoints;
    use crate::staking::StakingAction;

    fn request(action: StakingAction, amount: Option<&str>) -> StakingRequest {
        StakingRequest {
            wallet_id: "owned-wallet".into(),
            chain_id: Chain::Polkadot,
            action,
            validator_id: Some("7".into()),
            position_id: Some("polkadot:pool:7".into()),
            amount: amount.map(str::to_string),
            lockup_seconds: None,
        }
    }
    async fn prepare(
        service: &WalletService,
        request: StakingRequest,
    ) -> crate::send::polkadot::PreparedPolkadotTransaction {
        let owner = crate::derivation::primitives::encode_ss58(&[7; 32], 0);
        let PreparedPayload::Substrate(value) = service
            .prepare_substrate_staking(&request, &owner)
            .await
            .unwrap()
        else {
            panic!("wrong protocol")
        };
        value
    }
    #[tokio::test]
    async fn all_pool_actions_build_reviewed_calls_from_owned_current_state() {
        let (_, state, server) = node().await;
        let service = WalletService::new(vec![ChainEndpoints {
            chain_id: Chain::Polkadot,
            capabilities: vec![EndpointCapability::Staking],
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/polkadot-staking-vectors.json"
        ))
        .unwrap();
        state.lock().unwrap().member = false;
        let join = prepare(&service, request(StakingAction::Stake, Some("1"))).await;
        assert_eq!(join.amount, 10_000_000_000);
        assert_eq!(
            format!("0x{}", hex::encode(join.call_data)),
            fixture["calls"]["join"]
        );
        state.lock().unwrap().member = true;
        for (action, amount, key, budget) in [
            (StakingAction::Stake, Some("1"), "bondExtra", 10_000_000_000),
            (StakingAction::Withdraw, None, "withdraw", 0),
            (StakingAction::ClaimRewards, None, "claim", 0),
        ] {
            let prepared = prepare(&service, request(action, amount)).await;
            assert_eq!(prepared.amount, budget);
            assert_eq!(
                format!("0x{}", hex::encode(prepared.call_data)),
                fixture["calls"][key]
            );
        }
        let unbond = prepare(&service, request(StakingAction::Unstake, Some("90"))).await;
        let (client, _state, _server) = node().await;
        let metadata = client
            .pool_call_metadata(&format!("0x{}", "11".repeat(32)))
            .await
            .unwrap();
        assert_eq!(
            unbond.call_data,
            encode_pool_call(
                &metadata,
                &PoolCall::Unbond {
                    member: [7; 32],
                    points: 1_000_000_000_000
                }
            )
            .unwrap()
        );
        assert_eq!(unbond.amount, 0);
        {
            let mut state = state.lock().unwrap();
            state.pool_migration = true;
            state.member_migration = true;
            state.pending_slash = join.runtime.existential_deposit;
        }
        let migrated = prepare(&service, request(StakingAction::ClaimRewards, None)).await;
        assert_eq!(
            format!("0x{}", hex::encode(migrated.call_data)),
            fixture["calls"]["batchAll"]
        );
        // A position ID from another pool cannot authorize a withdrawal.
        let mut foreign = request(StakingAction::Withdraw, None);
        foreign.position_id = Some("polkadot:pool:8".into());
        let owner = crate::derivation::primitives::encode_ss58(&[7; 32], 0);
        assert!(
            service
                .prepare_substrate_staking(&foreign, &owner)
                .await
                .unwrap_err()
                .to_string()
                .contains("does not belong")
        );
        state.lock().unwrap().free = 0;
        assert!(
            service
                .prepare_substrate_staking(&request(StakingAction::Stake, Some("1")), &owner)
                .await
                .is_err()
        );
    }
}

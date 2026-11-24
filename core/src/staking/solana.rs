//! Solana staking validator queries.

use crate::api::error::ApiError;
use crate::api::solana_json_rpc::{SolanaClient, VoteAccount};
use crate::staking::StakingValidator;

fn vote_account_to_validator(v: VoteAccount, is_active: bool, minimum: u64) -> StakingValidator {
    StakingValidator {
        identifier: v.vote_pubkey.clone(),
        display_name: format!("Validator {}", super::short_id(&v.vote_pubkey)),
        commission: Some(v.commission as f64 / 100.0),
        total_stake_smallest_unit: Some(v.activated_stake.to_string()),
        is_active,
        tags: if is_active {
            vec![]
        } else {
            vec!["delinquent".to_string()]
        },
        min_delegation_smallest_unit: Some(minimum.to_string()),
        uptime_pct: None,
        website: None,
        description: None,
    }
}

/// Vote-account directory and commission; reward APY is unavailable.
pub(crate) async fn fetch_validators(
    rpc_endpoints: Vec<String>,
) -> Result<Vec<StakingValidator>, ApiError> {
    let client = SolanaClient::new(std::sync::Arc::new(rpc_endpoints));
    let resp = client.fetch_staking_validators().await?;
    let minimum = client.fetch_stake_minimum().await?;

    let mut validators: Vec<StakingValidator> = resp
        .current
        .into_iter()
        .map(|v| vote_account_to_validator(v, true, minimum))
        .chain(
            resp.delinquent
                .into_iter()
                .map(|v| vote_account_to_validator(v, false, minimum)),
        )
        .collect();

    // Sort by activated stake descending, show top 100.
    validators.sort_by(|a, b| {
        let a_stake = a
            .total_stake_smallest_unit
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let b_stake = b
            .total_stake_smallest_unit
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        b_stake.cmp(&a_stake)
    });
    validators.truncate(100);

    Ok(validators)
}

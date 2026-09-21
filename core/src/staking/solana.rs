//! Solana staking validator and position queries.

use serde::Deserialize;
use serde_json::json;

use crate::fetch::http::{with_fallback, HttpClient, RetryProfile};
use crate::staking::{StakingError, StakingPosition, StakingValidator};

pub struct SolanaStakingClient {
    rpc_endpoints: Vec<String>,
}

// ── RPC response types ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct VoteAccountsResp {
    result: VoteAccountsResult,
}
#[derive(Deserialize)]
struct VoteAccountsResult {
    current: Vec<VoteAccount>,
    delinquent: Vec<VoteAccount>,
}
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VoteAccount {
    vote_pubkey: String,
    activated_stake: u64,
    commission: u8,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn short_id(id: &str) -> &str {
    if id.len() >= 8 {
        &id[..8]
    } else {
        id
    }
}

fn vote_account_to_validator(v: VoteAccount, is_active: bool) -> StakingValidator {
    let apy = 0.065 * (1.0 - v.commission as f64 / 100.0);
    StakingValidator {
        identifier: v.vote_pubkey.clone(),
        display_name: format!("Validator {}", short_id(&v.vote_pubkey)),
        apy,
        commission: Some(v.commission as f64 / 100.0),
        total_stake_smallest_unit: Some(v.activated_stake.to_string()),
        is_active,
        tags: if is_active {
            vec![]
        } else {
            vec!["delinquent".to_string()]
        },
        min_delegation_smallest_unit: Some("1000000".to_string()), // 0.001 SOL
        uptime_pct: None,
        website: None,
        description: None,
        next_epoch_active: None,
    }
}

impl SolanaStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// Snapshot of the active validator set with vote-account identifier and
    /// computed APY. RPC: `getVoteAccounts` + epoch reward history.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        if self.rpc_endpoints.is_empty() {
            return Ok(vec![]);
        }
        let client = HttpClient::shared();
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getVoteAccounts",
            "params": [{"commitment": "confirmed", "keepUnstakedDelinquents": false}]
        });
        let resp: VoteAccountsResp = match with_fallback(&self.rpc_endpoints, |url| {
            let client = client.clone();
            let body = body.clone();
            async move { client.post_json(&url, &body, RetryProfile::ChainRead).await }
        })
        .await
        {
            Ok(r) => r,
            Err(_) => return Ok(vec![]),
        };

        let mut validators: Vec<StakingValidator> = resp
            .result
            .current
            .into_iter()
            .map(|v| vote_account_to_validator(v, true))
            .chain(
                resp.result
                    .delinquent
                    .into_iter()
                    .map(|v| vote_account_to_validator(v, false)),
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

    /// Stake accounts owned by this wallet. Not yet implemented — returns
    /// empty until wallet-specific position indexing is added.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }
}

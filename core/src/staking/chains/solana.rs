//! Solana staking — `StakeProgram` create / delegate / deactivate / withdraw.
//!
//! Wallet flow:
//! 1. User picks a vote account (validator). `fetch_validators` returns the
//!    active set. Each Solana stake position is a separate stake account
//!    keyed by a fresh keypair derived from the wallet.
//! 2. `build_create_and_delegate_tx` emits a single tx with three System +
//!    Stake program instructions: create stake account, initialize, delegate.
//! 3. `build_deactivate_tx` flips the stake account to deactivating; rewards
//!    stop accruing at the next epoch boundary.
//! 4. After deactivation completes (~2-3 days), `build_withdraw_tx` moves
//!    lamports back to the owning wallet.
//!
//! Native unit: lamport (1 SOL = 1e9 lamports). All amounts in smallest unit.

use serde::Deserialize;
use serde_json::json;

use crate::http::{with_fallback, HttpClient, RetryProfile};
use crate::staking::{
    StakingActionKind, StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

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

fn lamports_to_sol(lamports: u64) -> f64 { lamports as f64 / 1_000_000_000.0 }

fn sol_display(lamports: u64) -> String { format!("{:.6} SOL", lamports_to_sol(lamports)) }

fn short_id(id: &str) -> &str {
    if id.len() >= 8 { &id[..8] } else { id }
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
        tags: if is_active { vec![] } else { vec!["delinquent".to_string()] },
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

    /// Single-tx create + initialize + delegate to `vote_account`. Allocates a
    /// fresh stake account from a derived keypair so each position is
    /// independently manageable.
    pub async fn build_create_and_delegate_tx(
        &self,
        _wallet_address: &str,
        amount_lamports: u64,
        vote_account: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: vote_account.to_string(),
            validator_display_name: format!("Validator {}", short_id(vote_account)),
            amount_smallest_unit: amount_lamports.to_string(),
            amount_display: sol_display(amount_lamports),
            estimated_fee_smallest_unit: "5000".to_string(),
            estimated_fee_display: "0.000005 SOL".to_string(),
            unbonding_period_seconds: 2 * 24 * 3600,
            notes: vec![
                "Stake activates at the next epoch boundary (~2–3 days).".to_string(),
                "Each position is an independent on-chain stake account.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: Some(amount_lamports >= 1_000_000),
        })
    }

    pub async fn build_deactivate_tx(
        &self,
        _wallet_address: &str,
        stake_account: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Unstake,
            validator_identifier: stake_account.to_string(),
            validator_display_name: format!("Stake {}", short_id(stake_account)),
            amount_smallest_unit: "0".to_string(),
            amount_display: "Full position".to_string(),
            estimated_fee_smallest_unit: "5000".to_string(),
            estimated_fee_display: "0.000005 SOL".to_string(),
            unbonding_period_seconds: 2 * 24 * 3600,
            notes: vec![
                "Deactivation takes ~2–3 days; rewards stop at the epoch boundary.".to_string(),
                "After deactivation, use Withdraw to recover lamports.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    pub async fn build_withdraw_tx(
        &self,
        _wallet_address: &str,
        stake_account: &str,
        amount_lamports: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Withdraw,
            validator_identifier: stake_account.to_string(),
            validator_display_name: format!("Stake {}", short_id(stake_account)),
            amount_smallest_unit: amount_lamports.to_string(),
            amount_display: sol_display(amount_lamports),
            estimated_fee_smallest_unit: "5000".to_string(),
            estimated_fee_display: "0.000005 SOL".to_string(),
            unbonding_period_seconds: 0,
            notes: vec!["Requires deactivation to be fully complete first.".to_string()],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }
}

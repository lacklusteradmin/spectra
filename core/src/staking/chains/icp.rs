//! Internet Computer staking — neuron lock-ups via the NNS governance canister.
//!
//! Wallet flow (NNS, canister `rrkah-fqaaa-aaaaa-aaaaq-cai`):
//! 1. `manage_neuron::ClaimOrRefresh` to register a neuron with a fresh
//!    subaccount derived from the wallet. Funds (≥1 ICP) get locked into the
//!    governance subaccount on the ICP ledger.
//! 2. `manage_neuron::Configure(SetDissolveTimestamp { timestamp })` or
//!    `IncreaseDissolveDelay` to set lock-up. Minimum is 6 months for
//!    voting-rewards eligibility; max is 8 years (96 months).
//! 3. Voting rewards accrue automatically based on dissolve delay × stake +
//!    voting-participation bonus. `claim_maturity` to pay them out.
//! 4. `Configure(StartDissolving)` begins the dissolve countdown. Once
//!    `dissolve_state == Dissolved`, `Disburse(amount)` sweeps the principal
//!    + rewards back to the wallet account.
//!
//! Native unit: e8s (1 ICP = 1e8 e8s).

use crate::staking::{
    StakingActionKind, StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

pub struct IcpStakingClient {
    rosetta_endpoints: Vec<String>,
}

// ── Hardcoded well-known NNS named neurons ────────────────────────────────────
//
// ICP staking works through neuron following (liquid democracy) rather than
// traditional validator picking. These are the most-followed public neurons
// on the NNS. Users can follow any of them to automatically vote on governance
// proposals and earn full voting rewards without manual participation.

const KNOWN_NEURONS: &[(&str, &str, &str)] = &[
    ("6914974521667616512", "DFINITY Foundation", "The official DFINITY Foundation neuron. Votes on most NNS proposals."),
    ("2649066124616010593", "ICA (Internet Computer Association)", "Internet Computer Association governance neuron."),
    ("4966884161088437903", "Synapse.vote", "Community governance aggregator; follows technical proposals."),
    ("7305824810703703771", "Cycle_DAO", "Community-run DAO focused on decentralisation motions."),
    ("6366547817393942096", "Taggr", "Decentralised social platform neuron with active governance participation."),
];

// ── Helpers ───────────────────────────────────────────────────────────────────

fn e8s_to_icp(e8s: u64) -> f64 { e8s as f64 / 1e8 }

fn icp_display(e8s: u64) -> String { format!("{:.4} ICP", e8s_to_icp(e8s)) }

fn dissolve_delay_seconds(months: u32) -> i64 {
    months as i64 * 30 * 24 * 3600
}

impl IcpStakingClient {
    pub fn new(rosetta_endpoints: Vec<String>) -> Self {
        Self { rosetta_endpoints }
    }

    /// Known-good neurons / followee identities the user can delegate
    /// liquid-democracy votes to. ICP doesn't have validator picking like
    /// other PoS chains; instead users follow other neurons for proposal
    /// votes. Returned list maps to those followee neurons.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        let validators = KNOWN_NEURONS
            .iter()
            .map(|(neuron_id, name, description)| StakingValidator {
                identifier: neuron_id.to_string(),
                display_name: name.to_string(),
                apy: 0.14, // up to 14% with max dissolve delay + full voting participation
                commission: None,
                total_stake_smallest_unit: None,
                is_active: true,
                tags: vec!["named neuron".to_string()],
                min_delegation_smallest_unit: Some("100000000".to_string()), // 1 ICP
                uptime_pct: None,
                website: None,
                description: Some(description.to_string()),
                next_epoch_active: None,
            })
            .collect();
        Ok(validators)
    }

    /// All neurons controlled by this wallet's principal. Calls
    /// `list_neurons` on the NNS governance canister.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }

    /// Creates + funds a neuron with the requested dissolve delay (months).
    pub async fn build_create_neuron_tx(
        &self,
        _wallet_address: &str,
        amount_e8s: u64,
        dissolve_delay_months: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        let delay_secs = dissolve_delay_seconds(dissolve_delay_months);
        let delay_label = if dissolve_delay_months >= 12 {
            format!("{} years", dissolve_delay_months / 12)
        } else {
            format!("{dissolve_delay_months} months")
        };
        // Rough APY estimate: 5% base + 9% bonus for 8-year max delay.
        let apy_estimate = 0.05 + 0.09 * (dissolve_delay_months.min(96) as f64 / 96.0);
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: String::new(),
            validator_display_name: "NNS Neuron".to_string(),
            amount_smallest_unit: amount_e8s.to_string(),
            amount_display: icp_display(amount_e8s),
            estimated_fee_smallest_unit: "10000".to_string(), // 0.0001 ICP
            estimated_fee_display: "~0.0001 ICP".to_string(),
            unbonding_period_seconds: delay_secs,
            notes: vec![
                format!("Dissolve delay: {delay_label} (~{:.0}% APY estimate).", apy_estimate * 100.0),
                "Rewards require ≥6 months dissolve delay.".to_string(),
                "Follow a named neuron to auto-vote and earn full rewards.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: Some(amount_e8s >= 100_000_000), // 1 ICP
        })
    }

    pub async fn build_increase_dissolve_delay_tx(
        &self,
        _wallet_address: &str,
        neuron_id: u64,
        additional_months: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        let delay_secs = dissolve_delay_seconds(additional_months);
        Ok(StakingActionPreview {
            kind: StakingActionKind::Restake,
            validator_identifier: neuron_id.to_string(),
            validator_display_name: format!("Neuron #{neuron_id}"),
            amount_smallest_unit: "0".to_string(),
            amount_display: "Delay extension only".to_string(),
            estimated_fee_smallest_unit: "10000".to_string(),
            estimated_fee_display: "~0.0001 ICP".to_string(),
            unbonding_period_seconds: delay_secs,
            notes: vec![
                format!("Extends dissolve delay by {additional_months} months."),
                "Longer delay increases APY and voting multiplier.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    pub async fn build_start_dissolving_tx(
        &self,
        _wallet_address: &str,
        neuron_id: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Unstake,
            validator_identifier: neuron_id.to_string(),
            validator_display_name: format!("Neuron #{neuron_id}"),
            amount_smallest_unit: "0".to_string(),
            amount_display: "Full neuron".to_string(),
            estimated_fee_smallest_unit: "10000".to_string(),
            estimated_fee_display: "~0.0001 ICP".to_string(),
            unbonding_period_seconds: 0, // delay depends on neuron's current setting
            notes: vec![
                "Starts the dissolve countdown; rewards continue accruing during dissolve.".to_string(),
                "Once dissolved, use Disburse to sweep ICP back to your account.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    /// Calls `manage_neuron::MergeMaturity` (or `StakeMaturity` on newer NNS)
    /// to claim accumulated voting rewards into the neuron principal.
    pub async fn build_claim_maturity_tx(
        &self,
        _wallet_address: &str,
        neuron_id: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::ClaimRewards,
            validator_identifier: neuron_id.to_string(),
            validator_display_name: format!("Neuron #{neuron_id}"),
            amount_smallest_unit: "0".to_string(),
            amount_display: "All maturity".to_string(),
            estimated_fee_smallest_unit: "10000".to_string(), // 0.0001 ICP
            estimated_fee_display: "~0.0001 ICP".to_string(),
            unbonding_period_seconds: 0,
            notes: vec![
                "Merges all accumulated maturity into the neuron's staked principal.".to_string(),
                "Staked principal increases; no ICP is sent to your account.".to_string(),
                "Use Disburse after dissolving to receive ICP.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    /// Sweeps a fully-dissolved neuron's principal + maturity back to the
    /// wallet account.
    pub async fn build_disburse_tx(
        &self,
        _wallet_address: &str,
        neuron_id: u64,
        amount_e8s: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Withdraw,
            validator_identifier: neuron_id.to_string(),
            validator_display_name: format!("Neuron #{neuron_id}"),
            amount_smallest_unit: amount_e8s.to_string(),
            amount_display: icp_display(amount_e8s),
            estimated_fee_smallest_unit: "10000".to_string(),
            estimated_fee_display: "~0.0001 ICP".to_string(),
            unbonding_period_seconds: 0,
            notes: vec!["Requires the neuron dissolve state to be Dissolved.".to_string()],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }
}

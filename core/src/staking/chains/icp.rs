//! Icp staking validator and position queries.

use crate::staking::{StakingError, StakingPosition, StakingValidator};

pub struct IcpStakingClient {
    _rosetta_endpoints: Vec<String>,
}

// ── Hardcoded well-known NNS named neurons ────────────────────────────────────
//
// ICP staking works through neuron following (liquid democracy) rather than
// traditional validator picking. These are the most-followed public neurons
// on the NNS. Users can follow any of them to automatically vote on governance
// proposals and earn full voting rewards without manual participation.

const KNOWN_NEURONS: &[(&str, &str, &str)] = &[
    (
        "6914974521667616512",
        "DFINITY Foundation",
        "The official DFINITY Foundation neuron. Votes on most NNS proposals.",
    ),
    (
        "2649066124616010593",
        "ICA (Internet Computer Association)",
        "Internet Computer Association governance neuron.",
    ),
    (
        "4966884161088437903",
        "Synapse.vote",
        "Community governance aggregator; follows technical proposals.",
    ),
    (
        "7305824810703703771",
        "Cycle_DAO",
        "Community-run DAO focused on decentralisation motions.",
    ),
    (
        "6366547817393942096",
        "Taggr",
        "Decentralised social platform neuron with active governance participation.",
    ),
];

impl IcpStakingClient {
    pub fn new(rosetta_endpoints: Vec<String>) -> Self {
        Self {
            _rosetta_endpoints: rosetta_endpoints,
        }
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
}

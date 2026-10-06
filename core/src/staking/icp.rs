//! Known NNS neurons are voting follow targets, not APR-bearing validators.
use crate::api::error::ApiError;
use crate::api::icp_replica::{IcpReplicaClient, ListKnownNeuronsResponse};
use crate::send::{icp_staking, keys::Ed25519Seed};
use crate::staking::{StakingError, StakingValidator};
use std::sync::Arc;

pub struct IcpStakingClient {
    endpoints: Vec<String>,
}
impl IcpStakingClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self { endpoints }
    }
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        // Public directory: an ephemeral sender authenticates this read, with
        // no wallet identity, secret-store lookup or funding capability.
        let key = Ed25519Seed::from_hex(&hex::encode(rand::random::<[u8; 32]>()))
            .map_err(ApiError::invalid)?;
        let query = icp_staking::signed_query(
            "list_known_neurons",
            &candid::encode_args(()).map_err(ApiError::decode)?,
            &key,
        )
        .map_err(ApiError::invalid)?;
        let bytes = IcpReplicaClient::new(Arc::new(self.endpoints.clone()))
            .query(&query)
            .await?;
        let response: ListKnownNeuronsResponse =
            candid::decode_one(&bytes).map_err(ApiError::decode)?;
        response
            .known_neurons
            .into_iter()
            .map(|neuron| {
                let id = neuron
                    .id
                    .filter(|id| id.id > 0)
                    .ok_or_else(|| ApiError::decode("Known NNS neuron has no ID"))?;
                let data = neuron
                    .known_neuron_data
                    .ok_or_else(|| ApiError::decode("Known NNS neuron has no metadata"))?;
                Ok(StakingValidator {
                    identifier: id.id.to_string(),
                    display_name: data.name,
                    commission: None,
                    total_stake_smallest_unit: None,
                    is_active: true,
                    tags: vec!["NNS known neuron".into()],
                    min_delegation_smallest_unit: None,
                    uptime_pct: None,
                    website: None,
                    description: data.description,
                })
            })
            .collect()
    }
}

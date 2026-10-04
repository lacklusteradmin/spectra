//! NEAR delegated-pool discovery. Indexed amounts are never used for signing;
//! the selected network's pool contracts own balances and withdrawal readiness.
use crate::api::{
    error::{ApiError, OrDecode},
    http::{HttpClient, RetryProfile, race},
};
use serde_json::Value;
use std::sync::Arc;

pub(crate) struct FastnearClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}
impl FastnearClient {
    pub(crate) fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }
    pub(crate) async fn fetch_staking_pool_ids(
        &self,
        owner: &str,
    ) -> Result<Vec<String>, ApiError> {
        if !valid_account_id(owner) {
            return Err(ApiError::invalid("Invalid NEAR staking owner"));
        }
        race(&self.endpoints, |base| async move {
            let value: Value = self
                .client
                .get_json(
                    &format!("{}/v1/account/{owner}/staking", base.trim_end_matches('/')),
                    RetryProfile::ChainRead,
                )
                .await?;
            if value["account_id"].as_str() != Some(owner) {
                return Err(ApiError::decode("Fastnear staking owner mismatch"));
            }
            let rows = value["pools"]
                .as_array()
                .or_decode("Fastnear: missing staking pools")?;
            if rows.len() > 10_000 {
                return Err(ApiError::decode("Fastnear staking pool list exceeds limit"));
            }
            let mut ids = std::collections::BTreeSet::new();
            for row in rows {
                let id = row["pool_id"]
                    .as_str()
                    .filter(|id| valid_account_id(id))
                    .or_decode("Fastnear: invalid staking pool")?;
                ids.insert(id.to_string());
            }
            Ok(ids.into_iter().collect())
        })
        .await
    }
}
fn valid_account_id(id: &str) -> bool {
    (2..=64).contains(&id.len())
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        })
}

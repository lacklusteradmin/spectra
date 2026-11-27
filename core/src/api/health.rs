//! Read-only checks against the endpoint actually configured: that it is the
//! network it is configured for, asked through its API's client as a send
//! asks it, and, where its API cannot say, that it answers the API's own
//! reads.
use crate::api::endpoint::{Endpoint, EndpointApi, EndpointCapability};
use crate::api::error::ApiError;
use crate::api::http::{HttpClient, RetryProfile};
use crate::registry::Chain;
use serde_json::Value;

const ZERO_EVM: &str = "0x0000000000000000000000000000000000000000";
const ZERO_TRON: &str = "T9yD14Nj9j7xAB4dbGeiX9h8unkKHxuWwb";

enum Response {
    /// A positive block height, bare.
    Height,
    /// An Etherscan-style account listing: `status` 0 or 1 over an array.
    History,
    /// An ICP replica's CBOR status, read by its client.
    IcpReplica,
    /// Any non-null value at this pointer.
    Field(&'static str),
}

struct Check {
    url: String,
    response: Response,
}

impl Check {
    fn get(url: String, response: Response) -> Self {
        Self { url, response }
    }

    async fn run(&self) -> Result<(), ApiError> {
        if matches!(&self.response, Response::IcpReplica) {
            return crate::api::icp_replica::IcpReplicaClient::new(std::sync::Arc::new(vec![
                self.url.clone(),
            ]))
            .health()
            .await;
        }
        let value: Value = HttpClient::shared()
            .get_json(&self.url, RetryProfile::Diagnostics)
            .await?;
        self.validate(&value)
            .map_err(|reason| ApiError::decode(format!("{reason}: {value}")))
    }

    fn validate(&self, value: &Value) -> Result<(), &'static str> {
        if value.get("error").is_some_and(|v| !v.is_null())
            || value.get("success") == Some(&Value::Bool(false))
            || value.get("ok") == Some(&Value::Bool(false))
        {
            return Err("API error");
        }
        let valid = match self.response {
            Response::Height => value.as_u64().is_some_and(|height| height > 0),
            Response::History => {
                value["result"].is_array() && matches!(value["status"].as_str(), Some("0" | "1"))
            }
            Response::IcpReplica => return Err("ICP replica health requires CBOR decoding"),
            Response::Field(pointer) => value.pointer(pointer).is_some_and(|v| !v.is_null()),
        };
        if valid {
            Ok(())
        } else {
            Err("invalid health response")
        }
    }
}

/// The reads that show an endpoint of an API that cannot name its network
/// is serving that API. An API that can name it is checked by
/// [`crate::api::network::verify_network`] instead.
fn reads(endpoint: &Endpoint, capabilities: &[EndpointCapability]) -> Vec<Check> {
    use EndpointApi::*;
    let base = endpoint.url.trim_end_matches('/');
    let get = |suffix: &str, field| Check::get(format!("{base}{suffix}"), Response::Field(field));
    match endpoint.api {
        Esplora => vec![Check::get(
            format!("{base}/blocks/tip/height"),
            Response::Height,
        )],
        Blockbook => vec![get("/api/v2", "/blockbook/bestHeight")],
        Blockcypher => vec![get("", "/height")],
        Whatsonchain => vec![get("/chain/info", "/blocks")],
        BchRestV2 => vec![get("/blockchain/getBlockchainInfo", "/blocks")],
        UnisatOpenApi => vec![get("/v1/indexer/blockchain/info", "/data/blocks")],
        Insight => vec![get("/status", "/blocks")],
        KaspaRest => vec![get("/info/network", "/networkName")],
        Blockscout => [
            ("txlist", EndpointCapability::History),
            ("tokentx", EndpointCapability::TokenHistory),
        ]
        .into_iter()
        .filter(|(_, capability)| capabilities.contains(capability))
        .map(|(action, _)| {
            Check::get(
                crate::api::blockscout::explorer_query_url(
                    base,
                    &format!(
                        "module=account&action={action}&address={ZERO_EVM}&sort=desc&page=1&offset=1"
                    ),
                ),
                Response::History,
            )
        })
        .collect(),
        TrongridV1 => vec![get(&format!("/{ZERO_TRON}"), "/data")],
        Nearblocks => vec![get("/stats", "/data/total_txns")],
        Fastnear => vec![get(
            "/v1/account/0000000000000000000000000000000000000000000000000000000000000000/staking",
            "/pools",
        )],
        // The burn address, which every network has.
        Filfox => vec![get(
            "/address/f099/transfers?pageSize=1&page=0&type=transfer",
            "/totalCount",
        )],
        IcpReplica => vec![Check::get(base.into(), Response::IcpReplica)],
        _ => vec![],
    }
}

/// Whether `endpoint` was checked at all, whether it passed, and what was
/// found.
pub(crate) async fn probe(
    chain: Chain,
    endpoint: &Endpoint,
    capabilities: &[EndpointCapability],
) -> (bool, bool, String) {
    match crate::api::network::verify_network(chain, endpoint.api, &endpoint.url).await {
        Ok(true) => return (true, true, "network verified".into()),
        Err(error) => return (true, false, format!("network: {error}")),
        Ok(false) => {}
    }
    let checks = reads(endpoint, capabilities);
    if checks.is_empty() {
        return (
            false,
            false,
            "no check for this API and its declared capabilities".into(),
        );
    }
    // A retry waits out a blip. A test scripts every answer, so it has none.
    let retry_pause = std::time::Duration::from_millis(if cfg!(test) { 0 } else { 600 });
    for check in &checks {
        let mut result = check.run().await;
        if result.is_err() {
            tokio::time::sleep(retry_pause).await;
            result = check.run().await;
        }
        if let Err(error) = result {
            return (true, false, format!("{}: {error}", check.url));
        }
    }
    (
        true,
        true,
        "answered; its API cannot name its network".into(),
    )
}

#[cfg(test)]
#[path = "tests/health.rs"]
mod tests;

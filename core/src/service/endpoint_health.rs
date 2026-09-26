//! Read-only protocol checks against the endpoint actually configured.
use crate::fetch::http::{HttpClient, RetryProfile};
use crate::registry::{Chain, EvmHistorySource};
use crate::{AppCoreEndpointRecord, EndpointApi};
use serde_json::{Value, json};

const ZERO_EVM: &str = "0x0000000000000000000000000000000000000000";
const ZERO_TRON: &str = "T9yD14Nj9j7xAB4dbGeiX9h8unkKHxuWwb";
const XRP_GENESIS: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";

enum Response {
    RpcHex(Option<u64>),
    RpcValue,
    Height,
    History,
    Rosetta,
    Monero,
    Xrpl,
    Field(&'static str),
}

struct Check {
    url: String,
    body: Option<Value>,
    response: Response,
}

impl Check {
    fn get(url: String, response: Response) -> Self {
        Self {
            url,
            body: None,
            response,
        }
    }

    fn rpc(url: &str, method: &str, params: Value, response: Response) -> Self {
        Self {
            url: url.into(),
            body: Some(json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params})),
            response,
        }
    }

    fn label(&self) -> &str {
        self.body
            .as_ref()
            .and_then(|v| v["method"].as_str())
            .unwrap_or(&self.url)
    }

    async fn run(&self, chain: Chain) -> Result<(), String> {
        let client = HttpClient::shared();
        let value: Value = match &self.body {
            Some(body) => {
                client
                    .post_json(&self.url, body, RetryProfile::Diagnostics)
                    .await?
            }
            None => {
                client
                    .get_json(&self.url, RetryProfile::Diagnostics)
                    .await?
            }
        };
        self.validate(chain, &value)
            .map_err(|reason| format!("{reason}: {value}"))
    }

    fn validate(&self, chain: Chain, value: &Value) -> Result<(), &'static str> {
        if value.get("error").is_some_and(|v| !v.is_null())
            || value.get("success") == Some(&Value::Bool(false))
            || value.get("ok") == Some(&Value::Bool(false))
        {
            return Err("API error");
        }
        let valid = match self.response {
            Response::RpcHex(expected) => value
                .get("result")
                .and_then(Value::as_str)
                .and_then(|v| v.strip_prefix("0x"))
                .is_some_and(|v| {
                    !v.is_empty()
                        && v.bytes().all(|c| c.is_ascii_hexdigit())
                        && expected.is_none_or(|expected| {
                            u64::from_str_radix(v, 16).ok() == Some(expected)
                        })
                }),
            Response::RpcValue => {
                value.get("result").is_some_and(|v| !v.is_null())
                    && value.pointer("/result/error").is_none()
                    && value.pointer("/result/status").and_then(Value::as_str) != Some("error")
            }
            Response::Height => value.as_u64().is_some_and(|height| height > 0),
            Response::History => {
                value["result"].is_array() && matches!(value["status"].as_str(), Some("0" | "1"))
            }
            Response::Rosetta => value["network_identifiers"]
                .as_array()
                .is_some_and(|v| !v.is_empty()),
            Response::Monero => {
                value.pointer("/result/nettype").and_then(Value::as_str)
                    == chain.monero_network_name().ok()
                    && value
                        .pointer("/result/synchronized")
                        .and_then(Value::as_bool)
                        == Some(true)
            }
            Response::Xrpl => {
                value.pointer("/result/status").and_then(Value::as_str) == Some("success")
                    && value
                        .pointer("/result/info/validated_ledger/seq")
                        .and_then(Value::as_u64)
                        .is_some()
            }
            Response::Field(pointer) => value.pointer(pointer).is_some_and(|v| !v.is_null()),
        };
        if valid {
            Ok(())
        } else {
            Err("invalid health response or wrong network")
        }
    }
}

fn checks(chain: Chain, record: &AppCoreEndpointRecord) -> Result<Vec<Check>, String> {
    use EndpointApi::*;
    let Some(api) = record.api else {
        return Ok(vec![]);
    };
    let base = record.endpoint.trim_end_matches('/');
    let get = |suffix: &str, field| Check::get(format!("{base}{suffix}"), Response::Field(field));
    let rpc = |method: &str| Check::rpc(base, method, json!([]), Response::RpcValue);
    let checks = match api {
        EvmJsonRpc | TronJsonRpc => vec![
            Check::rpc(base, "eth_chainId", json!([]), Response::RpcHex(if api == EvmJsonRpc { Some(chain.evm_chain_id()?) } else { None })),
            Check::rpc(base, "eth_blockNumber", json!([]), Response::RpcHex(None)),
            Check::rpc(base, "eth_getBalance", json!([ZERO_EVM, "latest"]), Response::RpcHex(None)),
        ],
        SolanaJsonRpc => vec![rpc("getHealth"), rpc("getSlot")],
        SuiJsonRpc => vec![rpc("sui_getLatestCheckpointSequenceNumber")],
        NearJsonRpc => vec![rpc("status")],
        SubstrateJsonRpc => vec![rpc("chain_getHeader")],
        XrplJsonRpc => vec![Check::rpc(base, "server_info", json!([{}]), Response::Xrpl)],
        MoneroDaemonRpc => vec![Check::rpc(&format!("{base}/json_rpc"), "get_info", json!({}), Response::Monero)],
        Esplora => vec![Check::get(format!("{base}/blocks/tip/height"), Response::Height)],
        Blockscout => ["txlist", "tokentx"].into_iter()
            .filter(|action| record.capabilities.iter().any(|cap| cap == if *action == "txlist" { "history" } else { "token-history" }))
            .map(|action| crate::fetch::evm::explorer_query_url(
                EvmHistorySource::Open(base),
                &format!("module=account&action={action}&address={ZERO_EVM}&sort=desc&page=1&offset=1"),
            ).map(|url| Check::get(url, Response::History)))
            .collect::<Result<Vec<_>, _>>()?,
        IcpRosetta => vec![Check {
            url: format!("{base}/network/list"), body: Some(json!({"metadata":{}})), response: Response::Rosetta,
        }],
        TronHttp => vec![Check {
            url: format!("{base}/wallet/getnowblock"), body: Some(json!({})), response: Response::Field("/blockID"),
        }],
        TrongridV1 => vec![get(&format!("/{ZERO_TRON}"), "/data")],
        Xrpscan => vec![get(&format!("/{XRP_GENESIS}"), "/xrpBalance")],
        BlockchainInfo => vec![get("?active=1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa&n=1", "/wallet")],
        Blockbook => vec![get("/api/v2", "/blockbook/bestHeight")],
        Blockcypher => vec![get("", "/height")],
        AptosRest => vec![get("", "/ledger_version")],
        Whatsonchain => vec![get("/chain/info", "/blocks")],
        Koios => vec![get("/tip", "/0/block_no")],
        ToncenterV2 => vec![get("/getMasterchainInfo", "/result/last/seqno")],
        ToncenterV3 => vec![get("/masterchainInfo", "/last/seqno")],
        Horizon => vec![get("/fee_stats", "/last_ledger")],
        Nearblocks => vec![get("/stats", "/stats/0")],
        SubstrateSidecar => vec![get("/transaction/material", "/at/hash")],
        Insight => vec![get("/status", "/blocks")],
        KaspaRest => vec![get("/info/network", "/networkName")],
        // These legacy directory entries include operation URL prefixes.
        // An explicit probe must still belong to the configured service.
        Blockchair | BchRestV2 => {
            let url = record.probe_url.as_deref().ok_or("no health path for this API")?;
            let endpoint = reqwest::Url::parse(base).map_err(|e| e.to_string())?;
            let probe = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
            if endpoint.origin() != probe.origin() {
                return Err("health probe must use the configured endpoint origin".into());
            }
            vec![Check::get(url.into(), Response::Field(if api == Blockchair { "/data/blocks" } else { "/blocks" }))]
        }
        SochainV2 | Tronscan | MoneroWalletRpc | MoneroLightWallet => {
            return Err("no read-only health check for this API".into());
        }
    };
    if checks.is_empty() {
        return Err("no health check for the declared capabilities".into());
    }
    Ok(checks)
}

pub(super) async fn probe(chain: Chain, record: &AppCoreEndpointRecord) -> (bool, bool, String) {
    if record.api.is_none() {
        return (false, false, "an explorer link, not an API".into());
    }
    let checks = match checks(chain, record) {
        Ok(checks) => checks,
        Err(error) => return (false, false, error),
    };
    for check in &checks {
        let mut result = check.run(chain).await;
        if result.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            result = check.run(chain).await;
        }
        if let Err(error) = result {
            return (true, false, format!("{}: {error}", check.label()));
        }
    }
    (
        true,
        true,
        format!(
            "read checks passed: {}",
            checks
                .iter()
                .map(Check::label)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

#[cfg(test)]
mod tests;

//! Read-only protocol checks against the endpoint actually configured: does
//! it answer, and as the network it is configured for.
use crate::api::endpoint::{Endpoint, EndpointApi, EndpointCapability};
use crate::api::error::ApiError;
use crate::api::http::{HttpClient, RetryProfile};
use crate::registry::{Chain, EvmHistorySource};
use serde_json::{Value, json};

const ZERO_EVM: &str = "0x0000000000000000000000000000000000000000";
const ZERO_TRON: &str = "T9yD14Nj9j7xAB4dbGeiX9h8unkKHxuWwb";

enum Response {
    RpcHex(Option<u64>),
    RpcValue,
    Height,
    History,
    Rosetta,
    Monero,
    Xrpl,
    IcpReplica,
    /// A Cosmos node's `node_info`, naming the network's chain id.
    CosmosNetwork,
    /// An Algorand node's or indexer's genesis id and hash, which name the
    /// network every transaction binds.
    AlgorandGenesis,
    /// A VeChainThor node's block 0, naming the network's genesis id.
    ThorGenesis,
    /// A Lotus node's `StateNetworkName`, naming the network.
    FilecoinNetwork,
    /// A Lotus node's `ChainGetGenesis`, naming the genesis block.
    FilecoinGenesis,
    /// The Tezos chain id at this pointer: a node answers it bare, TzKT's
    /// head under `chainId`.
    TezosChainId(&'static str),
    /// A Starknet node's `starknet_chainId`, naming the network's chain id.
    StarknetChain,
    /// A Hedera mirror node's block 0, naming the network's first record
    /// file.
    HederaGenesis,
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

    async fn run(&self, chain: Chain) -> Result<(), ApiError> {
        if matches!(&self.response, Response::IcpReplica) {
            return crate::api::icp_replica::IcpReplicaClient::new(std::sync::Arc::new(vec![
                self.url.clone(),
            ]))
            .health()
            .await;
        }
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
            .map_err(|reason| ApiError::decode(format!("{reason}: {value}")))
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
            Response::IcpReplica => return Err("ICP replica health requires CBOR decoding"),
            Response::CosmosNetwork => {
                value
                    .pointer("/default_node_info/network")
                    .and_then(Value::as_str)
                    == chain.cosmos().map(|network| network.chain_id)
            }
            Response::AlgorandGenesis => chain.algorand_network().is_ok_and(|network| {
                use base64::Engine as _;
                value["genesis-id"].as_str() == Some(network.genesis_id)
                    && value["genesis-hash"].as_str().is_some_and(|hash| {
                        base64::engine::general_purpose::STANDARD.decode(hash).ok()
                            == Some(network.genesis_hash.to_vec())
                    })
            }),
            Response::ThorGenesis => {
                value.get("number").and_then(Value::as_u64) == Some(0)
                    && value.get("id").and_then(Value::as_str)
                        == chain.thor().map(|network| network.genesis_id)
            }
            Response::FilecoinNetwork => {
                value.get("result").and_then(Value::as_str) == chain.filecoin_network_name().ok()
            }
            Response::FilecoinGenesis => {
                value.pointer("/result/Cids/0/~1").and_then(Value::as_str)
                    == chain.filecoin_genesis_cid().ok()
            }
            Response::TezosChainId(pointer) => {
                value.pointer(pointer).and_then(Value::as_str)
                    == chain.tezos().map(|network| network.chain_id)
            }
            Response::StarknetChain => value
                .get("result")
                .and_then(Value::as_str)
                .and_then(crate::derivation::starknet::parse_felt)
                .zip(chain.starknet())
                .is_some_and(|(answer, network)| {
                    answer
                        == starknet_crypto::Felt::from_bytes_be_slice(network.chain_id.as_bytes())
                }),
            Response::HederaGenesis => {
                value.pointer("/hash").and_then(Value::as_str)
                    == chain.hedera().map(|network| network.genesis_block_hash)
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

fn checks(
    chain: Chain,
    endpoint: &Endpoint,
    capabilities: &[EndpointCapability],
) -> Result<Vec<Check>, ApiError> {
    use EndpointApi::*;
    let api = endpoint.api;
    let base = endpoint.url.trim_end_matches('/');
    let get = |suffix: &str, field| Check::get(format!("{base}{suffix}"), Response::Field(field));
    let rpc = |method: &str| Check::rpc(base, method, json!([]), Response::RpcValue);
    let checks = match api {
        EvmJsonRpc => vec![
            Check::rpc(base, "eth_chainId", json!([]), Response::RpcHex(Some(chain.evm_chain_id()?))),
            Check::rpc(base, "eth_blockNumber", json!([]), Response::RpcHex(None)),
            Check::rpc(base, "eth_getBalance", json!([ZERO_EVM, "latest"]), Response::RpcHex(None)),
        ],
        SolanaJsonRpc => vec![rpc("getHealth"), rpc("getSlot")],
        NearJsonRpc => vec![rpc("status")],
        SubstrateJsonRpc => vec![rpc("chain_getHeader")],
        XrplJsonRpc => vec![Check::rpc(base, "server_info", json!([{}]), Response::Xrpl)],
        MoneroDaemonRpc => vec![Check::rpc(&format!("{base}/json_rpc"), "get_info", json!({}), Response::Monero)],
        Esplora => vec![Check::get(format!("{base}/blocks/tip/height"), Response::Height)],
        Blockscout => ["txlist", "tokentx"].into_iter()
            .filter(|action| capabilities.contains(&if *action == "txlist" { EndpointCapability::History } else { EndpointCapability::TokenHistory }))
            .map(|action| crate::api::blockscout::explorer_query_url(
                EvmHistorySource::Open(base),
                &format!("module=account&action={action}&address={ZERO_EVM}&sort=desc&page=1&offset=1"),
            ).map(|url| Check::get(url, Response::History)))
            .collect::<Result<Vec<_>, _>>()?,
        IcpRosetta => vec![Check {
            url: format!("{base}/network/list"), body: Some(json!({"metadata":{}})), response: Response::Rosetta,
        }],
        IcpReplica => {
            if chain != Chain::Icp {
                return Err(ApiError::invalid("ICP replica is not supported on this network"));
            }
            vec![Check::get(base.into(), Response::IcpReplica)]
        },
        TronHttp => vec![Check {
            url: format!("{base}/wallet/getnowblock"), body: Some(json!({})), response: Response::Field("/blockID"),
        }],
        TrongridV1 => vec![get(&format!("/{ZERO_TRON}"), "/data")],
        Blockbook => vec![get("/api/v2", "/blockbook/bestHeight")],
        Blockcypher => vec![get("", "/height")],
        AptosRest => vec![get("", "/ledger_version")],
        AptosIndexer => vec![],
        Algod => vec![
            Check::get(format!("{base}/v2/transactions/params"), Response::AlgorandGenesis),
            get("/v2/status", "/last-round"),
        ],
        AlgorandIndexer => vec![
            Check::get(format!("{base}/v2/blocks/0"), Response::AlgorandGenesis),
            get("/health", "/round"),
        ],
        Whatsonchain => vec![get("/chain/info", "/blocks")],
        Koios => vec![get("/tip", "/0/block_no")],
        ToncenterV2 => vec![get("/getMasterchainInfo", "/result/last/seqno")],
        ToncenterV3 => vec![get("/masterchainInfo", "/last/seqno")],
        Horizon => vec![get("/fee_stats", "/last_ledger")],
        Nearblocks => vec![get("/stats", "/data/total_txns")],
        Fastnear => vec![get("/v1/account/0000000000000000000000000000000000000000000000000000000000000000/staking", "/pools")],
        Insight => vec![get("/status", "/blocks")],
        KaspaRest => vec![get("/info/network", "/networkName")],
        // A gateway that does not serve `node_info` cannot be verified, and
        // claims no verification.
        CosmosRest => capabilities
            .contains(&EndpointCapability::Verification)
            .then(|| Check::get(format!("{base}/cosmos/base/tendermint/v1beta1/node_info"), Response::CosmosNetwork))
            .into_iter()
            .chain([get("/cosmos/base/tendermint/v1beta1/blocks/latest", "/block/header/height")])
            .collect(),
        LotusJsonRpc => vec![
            Check::rpc(base, "Filecoin.StateNetworkName", json!([]), Response::FilecoinNetwork),
            Check::rpc(base, "Filecoin.ChainGetGenesis", json!([]), Response::FilecoinGenesis),
            rpc("Filecoin.ChainHead"),
        ],
        // The burn address, which every network has.
        Filfox => vec![get("/address/f099/transfers?pageSize=1&page=0&type=transfer", "/totalCount")],
        BchRestV2 => vec![get("/blockchain/getBlockchainInfo", "/blocks")],
        UnisatOpenApi => vec![get("/v1/indexer/blockchain/info", "/data/blocks")],
        ThorRest => vec![
            Check::get(format!("{base}/blocks/0"), Response::ThorGenesis),
            get("/blocks/best", "/number"),
        ],
        TezosRpc => vec![
            Check::get(format!("{base}/chains/main/chain_id"), Response::TezosChainId("")),
            get("/chains/main/blocks/head/header", "/level"),
        ],
        Tzkt => vec![Check::get(format!("{base}/v1/head"), Response::TezosChainId("/chainId"))],
        StarknetJsonRpc => vec![
            Check::rpc(base, "starknet_chainId", json!([]), Response::StarknetChain),
            rpc("starknet_blockNumber"),
        ],
        HederaMirror => vec![
            Check::get(format!("{base}/api/v1/blocks/0"), Response::HederaGenesis),
            get("/api/v1/network/exchangerate", "/current_rate/cent_equivalent"),
        ],
        // The address book, and the network of the mirror node serving it.
        HederaHapi => vec![
            Check::get(
                format!(
                    "{}/api/v1/blocks/0",
                    crate::api::hedera_mirror::mirror_of_address_book(base)?
                ),
                Response::HederaGenesis,
            ),
            Check::get(base.to_string(), Response::Field("/nodes/0/node_cert_hash")),
        ],
        // Checked through their own clients: gRPC, Litecoin's peer-to-peer
        // protocol, Chronik's protobuf and Sui's GraphQL.
        Lightwalletd | LitecoinP2p | Chronik | SuiGraphql => vec![],
    };
    if checks.is_empty() {
        return Err(ApiError::invalid(
            "no health check for the declared capabilities",
        ));
    }
    Ok(checks)
}

/// Whether `endpoint` was checked at all, whether it passed, and what was
/// found.
pub(crate) async fn probe(
    chain: Chain,
    endpoint: &Endpoint,
    capabilities: &[EndpointCapability],
) -> (bool, bool, String) {
    if endpoint.api == EndpointApi::Blockbook && chain.mainnet_counterpart() == Chain::Peercoin {
        return match crate::api::blockbook::BlockbookClient::new(
            std::sync::Arc::new(vec![endpoint.url.clone()]),
            chain,
        )
        .verify_peercoin_network()
        .await
        {
            Ok(()) => (true, true, "Peercoin network and precision verified".into()),
            Err(error) => (true, false, error.to_string()),
        };
    }
    if endpoint.api == EndpointApi::Chronik {
        return match crate::api::chronik::ChronikClient::new(
            std::sync::Arc::new(vec![endpoint.url.clone()]),
            chain,
        )
        .verify_network()
        .await
        {
            Ok(()) => (true, true, "Chronik network verified".into()),
            Err(error) => (true, false, error.to_string()),
        };
    }
    if endpoint.api == EndpointApi::Lightwalletd {
        return match crate::api::lightwalletd::LightwalletdClient::new(std::sync::Arc::new(vec![
            endpoint.url.clone(),
        ]))
        .session(chain)
        .await
        {
            Ok(session) => (
                true,
                true,
                format!("lightwalletd network verified at height {}", session.tip),
            ),
            Err(error) => (true, false, error.to_string()),
        };
    }
    if endpoint.api == EndpointApi::LitecoinP2p {
        return match crate::api::litecoin_p2p::LitecoinP2pClient::new(std::sync::Arc::new(vec![
            endpoint.url.clone(),
        ]))
        .session(chain, &Default::default())
        .await
        {
            Ok(session) => (
                true,
                true,
                format!(
                    "Litecoin node at height {} offers MWEB light-client service",
                    session.peer_height
                ),
            ),
            Err(error) => (true, false, error.to_string()),
        };
    }
    if endpoint.api == EndpointApi::AptosIndexer {
        let Ok(expected) = chain.aptos_chain_id() else {
            return (
                false,
                false,
                "Aptos indexer is not supported on this network".into(),
            );
        };
        return match crate::api::aptos_indexer::AptosIndexerClient::new(
            std::sync::Arc::new(vec![endpoint.url.clone()]),
            expected,
        )
        .verify_network()
        .await
        {
            Ok(()) => (true, true, "Aptos indexer network verified".into()),
            Err(error) => (true, false, error.to_string()),
        };
    }
    if endpoint.api == EndpointApi::SuiGraphql {
        return match crate::api::sui_graphql::SuiClient::new(std::sync::Arc::new(vec![
            endpoint.url.clone(),
        ]))
        .latest_checkpoint(chain)
        .await
        {
            Ok(checkpoint) => (
                true,
                true,
                format!("Sui network verified at checkpoint {checkpoint}"),
            ),
            Err(error) => (true, false, error.to_string()),
        };
    }
    if endpoint.api == EndpointApi::SubstrateJsonRpc && chain.is_substrate() {
        return match crate::api::substrate_json_rpc::SubstrateClient::new(std::sync::Arc::new(
            vec![endpoint.url.clone()],
        ))
        .polkadot_context(chain)
        .await
        {
            Ok(_) => (true, true, "Genesis and runtime metadata verified".into()),
            Err(error) => (true, false, error.to_string()),
        };
    }
    let checks = match checks(chain, endpoint, capabilities) {
        Ok(checks) => checks,
        Err(error) => return (false, false, error.to_string()),
    };
    // A retry waits out a blip. A test scripts every answer, so it has none.
    let retry_pause = std::time::Duration::from_millis(if cfg!(test) { 0 } else { 600 });
    for check in &checks {
        let mut result = check.run(chain).await;
        if result.is_err() {
            tokio::time::sleep(retry_pause).await;
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
#[path = "tests/health.rs"]
mod tests;

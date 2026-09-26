//! One directory for built-in and user-supplied API endpoints.
use super::*;
use crate::{AppCoreEndpointRecord, EndpointApi};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpoint {
    pub chain_id: String,
    pub api: EndpointApi,
    pub endpoint: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EndpointDirectoryEntry {
    pub record: AppCoreEndpointRecord,
    pub api_name: String,
    pub is_built_in: bool,
}

impl CustomEndpoint {
    pub(crate) fn validated(
        chain_id: String,
        api: String,
        endpoint: String,
        mut capabilities: Vec<String>,
    ) -> Result<Self, String> {
        let chain = Chain::from_str_id(&chain_id).ok_or("Unknown endpoint network")?;
        let catalog = crate::app_core::endpoint_catalog()?;
        let api = catalog
            .endpoint_records
            .iter()
            .filter(|r| r.chain_id == chain.str_id())
            .filter_map(|r| r.api)
            .chain(
                [
                    EndpointSlot::Primary,
                    EndpointSlot::Secondary,
                    EndpointSlot::Explorer,
                ]
                .into_iter()
                .filter_map(|slot| chain.endpoint_api(slot)),
            )
            .find(|value| value.as_str() == api)
            .ok_or("API type is not supported by this network")?;
        let supported = crate::endpoint_api::endpoint_capability_options(chain_id.clone(), api);
        if capabilities.is_empty() || capabilities.iter().any(|c| !supported.contains(c)) {
            return Err("Select at least one capability supported by this adapter".into());
        }
        capabilities.sort();
        capabilities.dedup();
        if endpoint
            .trim()
            .chars()
            .any(|c| c.is_whitespace() || c == ',')
        {
            return Err("Enter one endpoint URL".into());
        }
        let parsed =
            reqwest::Url::parse(endpoint.trim()).map_err(|_| "Enter a valid HTTP or HTTPS URL")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err("Enter an HTTP or HTTPS URL without credentials or a fragment".into());
        }
        let endpoint = parsed.to_string().trim_end_matches('/').to_string();
        if catalog
            .endpoint_records
            .iter()
            .any(|r| r.endpoint.trim_end_matches('/') == endpoint)
        {
            return Err("This URL is already in the built-in directory".into());
        }
        Ok(Self {
            chain_id,
            api,
            endpoint,
            capabilities,
        })
    }

    fn record(&self) -> Result<AppCoreEndpointRecord, String> {
        Ok(AppCoreEndpointRecord {
            id: format!(
                "custom:{}:{}:{}",
                self.chain_id,
                self.api.as_str(),
                self.endpoint
            ),
            api: Some(self.api),
            chain_id: self.chain_id.clone(),
            endpoint: self.endpoint.clone(),
            capabilities: self.capabilities.clone(),
            probe_url: None,
            explorer_label: None,
            tx_suffix: String::new(),
        })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn endpoint_directory(
        &self,
    ) -> Result<Vec<EndpointDirectoryEntry>, SpectraBridgeError> {
        let catalog = crate::app_core::endpoint_catalog()?;
        let custom = self
            .wallet_state
            .read()
            .await
            .settings
            .custom_endpoints
            .clone();
        let mut entries: Vec<_> = catalog
            .endpoint_records
            .iter()
            .cloned()
            .map(|record| EndpointDirectoryEntry {
                api_name: record
                    .api
                    .map(|api| api.as_str().into())
                    .unwrap_or_default(),
                record,
                is_built_in: true,
            })
            .collect();
        for endpoint in custom {
            entries.push(EndpointDirectoryEntry {
                api_name: endpoint.api.as_str().into(),
                record: endpoint.record()?,
                is_built_in: false,
            });
        }
        Ok(entries)
    }
}

impl WalletService {
    /// All requirements are conjunctive. Fallback never widens this set.
    pub(crate) async fn endpoints_for(
        &self,
        chain_id: &str,
        required: &[&str],
    ) -> Arc<Vec<String>> {
        let urls = self.configured_endpoint_urls(chain_id).await;
        let (network, slot) = match chain_id.split_once(':') {
            Some((network, "secondary")) => (network, EndpointSlot::Secondary),
            Some((network, "explorer")) => (network, EndpointSlot::Explorer),
            _ => (chain_id, EndpointSlot::Primary),
        };
        let Some(chain) = Chain::from_str_id(network) else {
            return Arc::new(vec![]);
        };
        let Ok(directory) = self.endpoint_directory().await else {
            return Arc::new(vec![]);
        };
        let index = self.endpoints.read().await;
        Arc::new(
            urls.iter()
                .filter(|url| {
                    let matching: Vec<_> = directory
                        .iter()
                        .filter(|e| {
                            e.record.endpoint.trim_end_matches('/') == url.trim_end_matches('/')
                        })
                        .collect();
                    if matching.is_empty() {
                        return index.capabilities.get(chain_id).is_some_and(|caps| {
                            required.iter().all(|c| caps.iter().any(|v| v == c))
                        });
                    }
                    matching.iter().any(|e| {
                        e.record.chain_id == network
                            && e.record.api == chain.endpoint_api(slot)
                            && required
                                .iter()
                                .all(|c| e.record.capabilities.iter().any(|v| v == c))
                    })
                })
                .cloned()
                .collect(),
        )
    }

    pub(crate) async fn api_endpoints(
        &self,
        chain: Chain,
        api: EndpointApi,
        required: &[&str],
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let mut urls = self.custom_api_endpoints(chain, api, required).await;
        for record in &crate::app_core::endpoint_catalog()?.endpoint_records {
            if record.chain_id == chain.str_id()
                && record.api == Some(api)
                && required
                    .iter()
                    .all(|c| record.capabilities.iter().any(|v| v == c))
                && !urls.contains(&record.endpoint)
            {
                urls.push(record.endpoint.clone());
            }
        }
        Ok(urls)
    }

    pub(crate) async fn custom_api_endpoints(
        &self,
        chain: Chain,
        api: EndpointApi,
        required: &[&str],
    ) -> Vec<String> {
        self.wallet_state
            .read()
            .await
            .settings
            .custom_endpoints
            .iter()
            .filter(|e| {
                e.chain_id == chain.str_id()
                    && e.api == api
                    && required
                        .iter()
                        .all(|c| e.capabilities.iter().any(|v| v == c))
            })
            .map(|e| e.endpoint.clone())
            .collect()
    }
}

impl WalletService {
    /// The same slot lists used by requests, including persisted custom URLs.
    pub async fn configured_endpoints(&self) -> Vec<ChainEndpoints> {
        let mut rows = Vec::new();
        for chain in Chain::all() {
            for slot in [
                EndpointSlot::Primary,
                EndpointSlot::Secondary,
                EndpointSlot::Explorer,
            ] {
                if chain.endpoint_api(slot).is_some() {
                    let chain_id = chain.endpoint_str_id(slot);
                    rows.push(ChainEndpoints {
                        capabilities: vec![],
                        endpoints: self
                            .configured_endpoint_urls(&chain_id)
                            .await
                            .as_ref()
                            .clone(),
                        chain_id,
                    });
                }
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::state::{AppSettingUpdate, StateCommand, StateEvent};
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn add(
        service: &WalletService,
        chain: &str,
        api: &str,
        url: &str,
    ) -> crate::store::state::StateTransition {
        service
            .apply_state_command(StateCommand::SetAppSetting {
                update: AppSettingUpdate::AddCustomEndpoint {
                    capabilities: match api {
                        "blockscout" => vec!["history".into()],
                        "trongrid-v1" => vec!["token-discovery".into()],
                        _ => vec!["balance".into(), "broadcast".into()],
                    },
                    chain_id: chain.into(),
                    api: api.into(),
                    endpoint: url.into(),
                },
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn custom_nodes_use_the_same_api_adapters_and_survive_reopening() {
        let db = std::env::temp_dir()
            .join(format!(
                "spectra-endpoints-{}.sqlite",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .to_string_lossy()
            .into_owned();
        let service = WalletService::new_catalog().unwrap();
        service.open_state(db.clone()).await.unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/solana"))
            .and(body_partial_json(
                serde_json::json!({"method":"getBalance"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"value":12345}}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bsv/address/test/balance"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"confirmed":456,"unconfirmed":0})),
            )
            .expect(1)
            .mount(&server)
            .await;
        for (chain, api, suffix) in [
            ("solana", "solana-json-rpc", "solana"),
            ("bitcoin-sv", "whatsonchain", "bsv"),
        ] {
            let result = add(&service, chain, api, &format!("{}/{suffix}", server.uri())).await;
            assert_eq!(result.events, vec![StateEvent::AppSettingChanged]);
        }
        let reopened = WalletService::new_catalog().unwrap();
        reopened.open_state(db.clone()).await.unwrap();
        assert_eq!(
            reopened
                .fetch_native_balance_summary("solana".into(), "test".into())
                .await
                .unwrap()
                .smallest_unit,
            "12345"
        );
        assert_eq!(
            reopened
                .fetch_native_balance_summary("bitcoin-sv".into(), "test".into())
                .await
                .unwrap()
                .smallest_unit,
            "456"
        );
        assert_eq!(
            reopened
                .endpoint_directory()
                .await
                .unwrap()
                .iter()
                .filter(|row| !row.is_built_in)
                .count(),
            2
        );
        assert!(
            !reopened
                .configured_endpoint_urls("solana-devnet")
                .await
                .iter()
                .any(|url| url.contains(&server.uri()))
        );
    }

    #[tokio::test]
    async fn custom_indexer_apis_are_used_separately_from_primary_rpc() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/blockscout/api"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"status":"1","message":"OK","result":[]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/accounts/test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":[{"trc20":[]}]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let service = WalletService::new_catalog().unwrap();
        assert_eq!(
            add(
                &service,
                "ethereum",
                "blockscout",
                &format!("{}/blockscout", server.uri())
            )
            .await
            .events,
            vec![StateEvent::AppSettingChanged]
        );
        assert_eq!(
            add(
                &service,
                "tron",
                "trongrid-v1",
                &format!("{}/v1/accounts", server.uri())
            )
            .await
            .events,
            vec![StateEvent::AppSettingChanged]
        );
        service
            .fetch_evm_history_page("ethereum".into(), "test".into(), vec![], 1, 10)
            .await
            .unwrap();
        assert!(
            service
                .discover_token_balances("tron".into(), "test".into())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !service
                .configured_endpoint_urls("ethereum")
                .await
                .iter()
                .any(|url| url.contains(&server.uri()))
        );
        assert!(
            !service
                .configured_endpoint_urls("tron")
                .await
                .iter()
                .any(|url| url.contains(&server.uri()))
        );
    }

    #[tokio::test]
    async fn invalid_and_duplicate_endpoints_leave_state_unchanged() {
        let service = WalletService::new_catalog().unwrap();
        for (chain, api, url) in [
            ("solana", "esplora", "https://node.example"),
            ("missing", "esplora", "https://node.example"),
            ("bitcoin", "esplora", "file:///tmp/node"),
            ("bitcoin", "esplora", "https://a.example,nope"),
            ("bitcoin", "esplora", "https://user:secret@node.example"),
            ("bitcoin", "esplora", "https://blockstream.info/api/"),
        ] {
            assert_eq!(
                add(&service, chain, api, url).await.events,
                vec![StateEvent::AppSettingRejected]
            );
        }
        assert!(
            service
                .app_state()
                .await
                .settings
                .custom_endpoints
                .is_empty()
        );
        assert_eq!(
            add(
                &service,
                "bitcoin",
                "esplora",
                " https://node.example/api/ "
            )
            .await
            .events,
            vec![StateEvent::AppSettingChanged]
        );
        assert_eq!(
            add(&service, "bitcoin", "esplora", "https://node.example/api")
                .await
                .events,
            vec![StateEvent::AppSettingRejected]
        );
        assert_eq!(
            add(&service, "bitcoin", "esplora", "https://other.example/api")
                .await
                .events,
            vec![StateEvent::AppSettingChanged]
        );
        assert_eq!(
            &service.configured_endpoint_urls("bitcoin").await[..2],
            &["https://other.example/api", "https://node.example/api"]
        );
    }
    #[tokio::test]
    async fn same_api_endpoints_keep_independent_capabilities_and_requests() {
        let service = WalletService::new_catalog().unwrap();
        let db =
            std::env::temp_dir().join(format!("endpoint-caps-{}.db", crate::store::new_event_id()));
        service
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        let balance = MockServer::start().await;
        let broadcast = MockServer::start().await;
        for (url, capabilities) in [
            (balance.uri(), vec!["balance".into()]),
            (broadcast.uri(), vec!["broadcast".into()]),
        ] {
            let result = service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: AppSettingUpdate::AddCustomEndpoint {
                        chain_id: "ethereum".into(),
                        api: "evm-json-rpc".into(),
                        endpoint: url,
                        capabilities,
                    },
                })
                .await
                .unwrap();
            assert_eq!(result.events, vec![StateEvent::AppSettingChanged]);
        }
        Mock::given(method("POST"))
            .and(body_partial_json(
                serde_json::json!({"method":"eth_getBalance"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"jsonrpc":"2.0","id":1,"result":"0x2a"})),
            )
            .expect(1)
            .mount(&balance)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(
                serde_json::json!({"method":"eth_chainId"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"jsonrpc":"2.0","id":1,"result":"0x1"})),
            )
            .expect(1)
            .mount(&broadcast)
            .await;
        let reopened = WalletService::new_catalog().unwrap();
        reopened
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        assert_eq!(
            reopened
                .fetch_native_balance_summary(
                    "ethereum".into(),
                    "0x1111111111111111111111111111111111111111".into()
                )
                .await
                .unwrap()
                .smallest_unit,
            "42"
        );
        assert!(
            reopened
                .validate_broadcast_endpoint(Chain::Ethereum, &balance.uri())
                .await
                .is_err()
        );
        reopened
            .validate_broadcast_endpoint(Chain::Ethereum, &broadcast.uri())
            .await
            .unwrap();
        let rows = reopened.endpoint_directory().await.unwrap();
        let own: Vec<_> = rows.iter().filter(|r| !r.is_built_in).collect();
        assert_eq!(own.len(), 2);
        assert_eq!(
            own.iter()
                .find(|r| r.record.endpoint == balance.uri())
                .unwrap()
                .record
                .capabilities,
            ["balance"]
        );
        assert!(
            own.iter()
                .all(|r| r.record.probe_url.is_none() && r.record.explorer_label.is_none())
        );
        assert!(
            !reopened
                .send_endpoints("ethereum".into())
                .await
                .unwrap()
                .contains(&balance.uri())
        );
        assert!(
            reopened
                .send_endpoints("ethereum".into())
                .await
                .unwrap()
                .contains(&broadcast.uri())
        );
        assert_eq!(balance.received_requests().await.unwrap().len(), 1);
        assert_eq!(broadcast.received_requests().await.unwrap().len(), 1);
        // Even explicit transport overrides cannot widen a saved declaration.
        reopened
            .update_endpoints(vec![ChainEndpoints {
                chain_id: "ethereum".into(),
                endpoints: vec![balance.uri()],
                capabilities: vec!["broadcast".into()],
            }])
            .await
            .unwrap();
        assert!(
            reopened
                .send_endpoints("ethereum".into())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            reopened
                .validate_broadcast_endpoint(Chain::Ethereum, &balance.uri())
                .await
                .is_err()
        );
    }

    #[test]
    fn custom_capabilities_are_explicit_validated_and_canonical() {
        for caps in [vec![], vec!["made-up".into()], vec!["history".into()]] {
            assert!(
                CustomEndpoint::validated(
                    "ethereum".into(),
                    "evm-json-rpc".into(),
                    "https://node.example".into(),
                    caps
                )
                .is_err()
            );
        }
        let endpoint = CustomEndpoint::validated(
            "ethereum".into(),
            "evm-json-rpc".into(),
            "https://node.example".into(),
            vec!["fee".into(), "balance".into(), "fee".into()],
        )
        .unwrap();
        assert_eq!(endpoint.capabilities, ["balance", "fee"]);
        assert_eq!(endpoint.record().unwrap().capabilities, ["balance", "fee"]);
    }
    #[tokio::test]
    async fn evm_preview_routes_balance_fee_and_context_to_separate_nodes() {
        let service = WalletService::new_catalog().unwrap();
        let balance = MockServer::start().await;
        let fees = MockServer::start().await;
        let context = MockServer::start().await;
        for (server, cap) in [
            (&balance, "balance"),
            (&fees, "fee"),
            (&context, "verification"),
        ] {
            service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: AppSettingUpdate::AddCustomEndpoint {
                        chain_id: "ethereum".into(),
                        api: "evm-json-rpc".into(),
                        endpoint: server.uri(),
                        capabilities: vec![cap.into()],
                    },
                })
                .await
                .unwrap();
            Mock::given(method("POST"))
                .respond_with(move |request: &wiremock::Request| {
                    let body: serde_json::Value = request.body_json().unwrap();
                    let result = match (cap, body["method"].as_str().unwrap()) {
                        ("balance", "eth_getBalance") => json!("0xde0b6b3a7640000"),
                        ("verification", "eth_getTransactionCount") => json!("0x7"),
                        ("fee", "eth_estimateGas") => json!("0x5208"),
                        ("fee", "eth_feeHistory") => {
                            json!({"baseFeePerGas":["0x1"],"reward":[["0x2"]]})
                        }
                        _ => panic!("Request escaped its declared capability: {cap}: {body}"),
                    };
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                })
                .expect(if cap == "fee" { 2 } else { 1 })
                .mount(server)
                .await;
        }
        service
            .fetch_evm_send_preview_json(
                "ethereum",
                format!("0x{}", "11".repeat(20)),
                format!("0x{}", "22".repeat(20)),
                "1".into(),
                "0x".into(),
            )
            .await
            .unwrap();
        assert_eq!(balance.received_requests().await.unwrap().len(), 1);
        assert_eq!(fees.received_requests().await.unwrap().len(), 2);
        assert_eq!(context.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fallback_and_submission_never_widen_capabilities() {
        let service = WalletService::new_catalog().unwrap();
        let balance = MockServer::start().await;
        let failing_balance = MockServer::start().await;
        let broadcast = MockServer::start().await;
        for (server, caps) in [
            (&balance, vec!["balance".into()]),
            (&failing_balance, vec!["balance".into()]),
            (&broadcast, vec!["broadcast".into()]),
        ] {
            service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: AppSettingUpdate::AddCustomEndpoint {
                        chain_id: "ethereum".into(),
                        api: "evm-json-rpc".into(),
                        endpoint: server.uri(),
                        capabilities: caps,
                    },
                })
                .await
                .unwrap();
        }
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&failing_balance)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"eth_getBalance"})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":"0x2a"})),
            )
            .expect(1)
            .mount(&balance)
            .await;
        for (rpc, result) in [
            ("eth_chainId", "0x1"),
            ("eth_sendRawTransaction", "0xaccepted"),
        ] {
            Mock::given(method("POST"))
                .and(body_partial_json(json!({"method":rpc})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":result})),
                )
                .expect(1)
                .mount(&broadcast)
                .await;
        }
        // Restrict the fixture to loopback, retaining the saved per-endpoint declarations.
        service
            .update_endpoints(vec![ChainEndpoints {
                chain_id: "ethereum".into(),
                endpoints: vec![broadcast.uri(), failing_balance.uri(), balance.uri()],
                capabilities: vec![],
            }])
            .await
            .unwrap();
        assert_eq!(
            service
                .fetch_native_balance_summary("ethereum".into(), "test".into())
                .await
                .unwrap()
                .smallest_unit,
            "42"
        );
        assert_eq!(broadcast.received_requests().await.unwrap().len(), 0);
        let result = service
            .broadcast_raw("ethereum", "0xdeadbeef".into())
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap()["txid"],
            "0xaccepted"
        );
        assert!(
            !failing_balance
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(balance.received_requests().await.unwrap().len(), 1);
        assert_eq!(broadcast.received_requests().await.unwrap().len(), 2);
        service
            .update_endpoints(vec![ChainEndpoints {
                chain_id: "ethereum".into(),
                endpoints: vec![balance.uri()],
                capabilities: vec!["broadcast".into()],
            }])
            .await
            .unwrap();
        assert!(
            service
                .broadcast_raw("ethereum", "0xdeadbeef".into())
                .await
                .is_err()
        );
        assert_eq!(balance.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn whatsonchain_broadcast_uses_adapter_base_not_operation_path() {
        let service = WalletService::new_catalog().unwrap();
        assert_eq!(
            service.send_endpoints("bitcoin-sv".into()).await.unwrap(),
            ["https://api.whatsonchain.com/v1/bsv/main"]
        );
    }
}

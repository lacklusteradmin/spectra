//! One directory for built-in and user-supplied API endpoints.
use super::*;
use crate::api::endpoint::{Endpoint, EndpointApi, EndpointCapability};
use crate::catalog::endpoints::EndpointRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpoint {
    pub chain_id: crate::registry::Chain,
    pub api: EndpointApi,
    pub endpoint: String,
    pub capabilities: Vec<EndpointCapability>,
}

#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EndpointDirectoryEntry {
    pub record: EndpointRecord,
    pub api_name: String,
    pub is_built_in: bool,
}

impl CustomEndpoint {
    pub(crate) fn validated(
        chain: Chain,
        api: EndpointApi,
        endpoint: String,
        mut capabilities: Vec<EndpointCapability>,
    ) -> Result<Self, SpectraBridgeError> {
        let catalog = crate::catalog::endpoints::catalog();
        if !chain.compatible_endpoint_apis().contains(&api) {
            return Err(SpectraBridgeError::failure(
                "API type is not supported by this network",
            ));
        }
        let supported = crate::api::endpoint::endpoint_capability_options(chain, api);
        if capabilities.is_empty() || capabilities.iter().any(|c| !supported.contains(c)) {
            return Err(SpectraBridgeError::failure(
                "Select at least one capability supported by this adapter",
            ));
        }
        capabilities.sort();
        capabilities.dedup();
        if endpoint
            .trim()
            .chars()
            .any(|c| c.is_whitespace() || c == ',')
        {
            return Err(SpectraBridgeError::failure("Enter one endpoint URL"));
        }
        // A Litecoin node is reached peer to peer, at `tcp://host:port`.
        let peer_to_peer = api == EndpointApi::LitecoinP2p;
        let parsed = reqwest::Url::parse(endpoint.trim()).map_err(|_| {
            SpectraBridgeError::failure(if peer_to_peer {
                "Enter a node as tcp://host:port"
            } else {
                "Enter a valid HTTP or HTTPS URL"
            })
        })?;
        let scheme_fits = if peer_to_peer {
            parsed.scheme() == "tcp"
                && matches!(parsed.path(), "" | "/")
                && parsed.query().is_none()
        } else {
            matches!(parsed.scheme(), "http" | "https")
        };
        if !scheme_fits
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(SpectraBridgeError::failure(if peer_to_peer {
                "Enter a node as tcp://host:port"
            } else {
                "Enter an HTTP or HTTPS URL without credentials or a fragment"
            }));
        }
        let endpoint = parsed.to_string().trim_end_matches('/').to_string();
        if catalog
            .records
            .iter()
            .any(|r| r.endpoint.trim_end_matches('/') == endpoint)
        {
            return Err(SpectraBridgeError::failure(
                "This URL is already in the built-in directory",
            ));
        }
        Ok(Self {
            chain_id: chain,
            api,
            endpoint,
            capabilities,
        })
    }

    fn record(&self) -> Result<EndpointRecord, SpectraBridgeError> {
        Ok(EndpointRecord {
            id: format!(
                "custom:{}:{}:{}",
                self.chain_id,
                self.api.as_str(),
                self.endpoint
            ),
            api: self.api,
            chain_id: self.chain_id,
            endpoint: self.endpoint.clone(),
            capabilities: self.capabilities.clone(),
        })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn endpoint_directory(
        &self,
    ) -> Result<Vec<EndpointDirectoryEntry>, SpectraBridgeError> {
        let this = self.clone();
        crate::service::worker::run(async move {
            let this = &this;
            let custom = this
                .wallet_state
                .read()
                .await
                .settings
                .custom_endpoints
                .clone();
            let mut entries: Vec<_> = this
                .built_in_endpoints()
                .iter()
                .cloned()
                .map(|record| EndpointDirectoryEntry {
                    api_name: record.api.as_str().into(),
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
        })
        .await
    }
}

impl WalletService {
    pub(crate) fn built_in_endpoints(&self) -> Arc<Vec<EndpointRecord>> {
        self.built_in_endpoints.read().clone()
    }

    /// Every endpoint `chain` is read and sent through, of every API it
    /// takes: the user's own, then the built-in ones unless the user chose
    /// theirs alone, a built-in one at a URL the user saved giving way to the
    /// user's. Endpoints named for the chain's own APIs replace every
    /// configured one of those APIs.
    pub(crate) async fn endpoint_records(&self, chain: Chain) -> Vec<EndpointRecord> {
        let mut records = self.configured_records(chain).await;
        if let Some(named) = self.endpoint_overrides.read().get(&chain) {
            records.retain(|record| !chain.endpoint_apis().contains(&record.api));
            records.splice(0..0, named.iter().cloned());
        }
        records
    }

    /// `endpoint_records` before any named for this service's life.
    async fn configured_records(&self, chain: Chain) -> Vec<EndpointRecord> {
        let (custom, custom_only) = {
            let state = self.wallet_state.read().await;
            (
                state
                    .settings
                    .custom_endpoints
                    .iter()
                    .filter(|endpoint| endpoint.chain_id == chain)
                    .cloned()
                    .collect::<Vec<_>>(),
                state.settings.custom_endpoints_only.contains(&chain),
            )
        };
        let mut records: Vec<EndpointRecord> =
            custom.iter().filter_map(|e| e.record().ok()).collect();
        if !custom_only {
            for record in self.built_in_endpoints().iter() {
                if record.chain_id == chain
                    && !records
                        .iter()
                        .any(|r| same_url(&r.endpoint, &record.endpoint))
                {
                    records.push(record.clone());
                }
            }
        }
        records
    }

    /// Read and send on `chain` through `urls`, for this service's life. A
    /// URL configured for one of the network's APIs is used as configured;
    /// any other speaks the network's first API, declared for everything
    /// Spectra's adapter for it does there. Those of the network's own APIs
    /// replace every configured one of them.
    pub async fn use_only_endpoints(
        &self,
        chain: Chain,
        urls: Vec<String>,
    ) -> Result<(), SpectraBridgeError> {
        let configured = self.configured_records(chain).await;
        let api = chain.default_api().ok_or_else(|| {
            SpectraBridgeError::invalid(format!("{} takes no endpoints", chain.str_id()))
        })?;
        let named: Vec<EndpointRecord> = urls
            .into_iter()
            .map(|url| {
                configured
                    .iter()
                    .find(|record| same_url(&record.endpoint, &url))
                    .cloned()
                    .unwrap_or_else(|| EndpointRecord {
                        id: format!("named:{url}"),
                        api,
                        chain_id: chain,
                        capabilities: crate::api::endpoint::endpoint_capability_options(chain, api),
                        endpoint: url,
                    })
            })
            .filter(|record| chain.endpoint_apis().contains(&record.api))
            .collect();
        if !named.is_empty() {
            self.endpoint_overrides.write().insert(chain, named);
        }
        Ok(())
    }

    /// Each network's own endpoints as requests use them.
    pub async fn configured_endpoints(&self) -> Vec<(Chain, Vec<String>)> {
        let mut rows = Vec::new();
        for chain in Chain::all().filter(|chain| !chain.endpoint_apis().is_empty()) {
            rows.push((
                chain,
                self.chain_endpoints(chain, &[])
                    .await
                    .into_iter()
                    .map(|endpoint| endpoint.url)
                    .collect(),
            ));
        }
        rows
    }

    /// Every endpoint of the chain's own APIs that declares each capability
    /// in `required`, with the API it speaks. The list has no order:
    /// requests go to all of them at once.
    pub(crate) async fn chain_endpoints(
        &self,
        chain: Chain,
        required: &[EndpointCapability],
    ) -> Vec<Endpoint> {
        self.endpoint_records(chain)
            .await
            .into_iter()
            .filter(|record| {
                chain.endpoint_apis().contains(&record.api)
                    && required.iter().all(|c| record.capabilities.contains(c))
                    && crate::api::http::may_contact(&record.endpoint)
            })
            .map(|record| Endpoint {
                api: record.api,
                url: record.endpoint,
            })
            .collect()
    }

    /// The URLs of `chain_endpoints`, for a client that speaks the chain's
    /// only API. The UTXO family has several and uses `utxo_client`.
    pub(crate) async fn endpoints_for(
        &self,
        chain: Chain,
        required: &[EndpointCapability],
    ) -> Arc<Vec<String>> {
        Arc::new(
            self.chain_endpoints(chain, required)
                .await
                .into_iter()
                .map(|endpoint| endpoint.url)
                .collect(),
        )
    }

    /// The UTXO family's client over every URL of every API it speaks.
    pub(crate) async fn utxo_client(
        &self,
        chain: Chain,
        required: &[EndpointCapability],
    ) -> crate::api::utxo::UtxoClient {
        crate::api::utxo::UtxoClient::new(chain, self.chain_endpoints(chain, required).await)
    }

    /// The API a configured URL speaks on `chain`, when it is one of the
    /// chain's own.
    pub(crate) async fn endpoint_api(&self, chain: Chain, url: &str) -> Option<EndpointApi> {
        self.chain_endpoints(chain, &[])
            .await
            .into_iter()
            .find(|endpoint| same_url(&endpoint.url, url))
            .map(|endpoint| endpoint.api)
    }

    /// The URLs for one API on `chain` that declare every capability in
    /// `required`. How indexers and secondary services are found; the
    /// chain's own list is `chain_endpoints`.
    pub(crate) async fn api_endpoints(
        &self,
        chain: Chain,
        api: EndpointApi,
        required: &[EndpointCapability],
    ) -> Result<Vec<String>, SpectraBridgeError> {
        Ok(self
            .endpoint_records(chain)
            .await
            .into_iter()
            .filter(|record| {
                record.api == api
                    && required.iter().all(|c| record.capabilities.contains(c))
                    && crate::api::http::may_contact(&record.endpoint)
            })
            .map(|record| record.endpoint)
            .collect())
    }
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

impl WalletService {
    /// The API and URLs of a chain with one API. The UTXO family has several
    /// and goes through `utxo_client` instead.
    pub(crate) async fn fetch_endpoints(
        &self,
        chain: Chain,
        required: &[EndpointCapability],
    ) -> Result<(EndpointApi, Arc<Vec<String>>), SpectraBridgeError> {
        // A chain of one API reads through it; one of several, through the
        // one API that serves `required` (Hedera's mirror node, not the
        // consensus nodes it submits to).
        let apis = chain.endpoint_apis();
        let serving: Vec<EndpointApi> = if let [api] = apis {
            vec![*api]
        } else {
            apis.iter()
                .copied()
                .filter(|api| {
                    let options = crate::api::endpoint::endpoint_capability_options(chain, *api);
                    required
                        .iter()
                        .all(|capability| options.contains(capability))
                })
                .collect()
        };
        let [api] = serving.as_slice() else {
            return Err(SpectraBridgeError::failure(format!(
                "{} has no single fetch API",
                chain.str_id()
            )));
        };
        Ok((*api, self.endpoints_for(chain, required).await))
    }
}

/// A test's endpoints for one network: each URL speaking the network's
/// first API and declaring `capabilities`, as far as that API's adapter does
/// them there.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct ChainEndpoints {
    pub(crate) capabilities: Vec<EndpointCapability>,
    pub(crate) chain_id: Chain,
    pub(crate) endpoints: Vec<String>,
}

#[cfg(test)]
impl ChainEndpoints {
    fn records(&self) -> Vec<EndpointRecord> {
        let Some(api) = self.chain_id.default_api() else {
            return vec![];
        };
        let supported = crate::api::endpoint::endpoint_capability_options(self.chain_id, api);
        self.endpoints
            .iter()
            .map(|url| EndpointRecord {
                id: format!("test:{}:{url}", self.chain_id),
                api,
                chain_id: self.chain_id,
                endpoint: url.clone(),
                capabilities: self
                    .capabilities
                    .iter()
                    .copied()
                    .filter(|c| supported.contains(c))
                    .collect(),
            })
            .collect()
    }
}

#[cfg(test)]
impl WalletService {
    /// A service built in with `endpoints` alone.
    pub(crate) fn serving(endpoints: Vec<ChainEndpoints>) -> Result<Arc<Self>, SpectraBridgeError> {
        let service = Self::new(vec![])?;
        *service.built_in_endpoints.write() =
            Arc::new(endpoints.iter().flat_map(ChainEndpoints::records).collect());
        Ok(service)
    }

    /// Replace the built-in endpoints of each network `endpoints` names.
    pub(crate) async fn serve(
        &self,
        endpoints: Vec<ChainEndpoints>,
    ) -> Result<(), SpectraBridgeError> {
        let mut built_in = self.built_in_endpoints.write();
        let mut records: Vec<EndpointRecord> = built_in
            .iter()
            .filter(|record| !endpoints.iter().any(|e| e.chain_id == record.chain_id))
            .cloned()
            .collect();
        records.extend(endpoints.iter().flat_map(ChainEndpoints::records));
        *built_in = Arc::new(records);
        Ok(())
    }
}

#[cfg(test)]
mod destination_activity_tests {
    use super::*;
    #[test]
    fn activity_distinguishes_an_unused_address_from_a_previously_emptied_one() {
        for (balance, history, expected) in [
            (true, false, SendDestinationActivity::Unused),
            (true, true, SendDestinationActivity::EmptyPreviouslyUsed),
            (false, false, SendDestinationActivity::Funded),
            (false, true, SendDestinationActivity::Funded),
        ] {
            assert_eq!(
                SendDestinationRisk::from_probe(balance, history).activity,
                expected
            );
        }
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
        chain: Chain,
        api: EndpointApi,
        url: &str,
    ) -> crate::store::state::StateTransition {
        service
            .apply_state_command(StateCommand::SetAppSetting {
                update: AppSettingUpdate::AddCustomEndpoint {
                    capabilities: match api {
                        EndpointApi::Blockscout => vec![
                            EndpointCapability::History,
                            EndpointCapability::TokenHistory,
                        ],
                        EndpointApi::TrongridV1 => vec![EndpointCapability::TokenDiscovery],
                        _ => vec![EndpointCapability::Balance, EndpointCapability::Broadcast],
                    },
                    chain_id: chain,
                    api,
                    endpoint: url.into(),
                },
            })
            .await
            .unwrap()
    }

    /// Every network and API the catalog serves takes a custom endpoint of
    /// that API, declaring every capability the adapter supports: a Litecoin
    /// node as `tcp://host:port`, any other as an HTTPS URL.
    #[test]
    fn every_catalog_network_and_api_takes_a_custom_endpoint() {
        let mut pairs: Vec<(Chain, EndpointApi)> = Vec::new();
        for record in &crate::catalog::endpoints::catalog().records {
            if !pairs.contains(&(record.chain_id, record.api)) {
                pairs.push((record.chain_id, record.api));
            }
        }
        let mut accepted = 0;
        for (index, (chain, api)) in pairs.into_iter().enumerate() {
            let capabilities = crate::api::endpoint::endpoint_capability_options(chain, api);
            if capabilities.is_empty() {
                continue;
            }
            let url = if api == EndpointApi::LitecoinP2p {
                format!("tcp://custom-{index}.example:9333")
            } else {
                format!("https://custom-{index}.example/api")
            };
            let endpoint = CustomEndpoint::validated(chain, api, url.clone(), capabilities.clone())
                .unwrap_or_else(|error| panic!("{chain} {}: {error}", api.as_str()));
            let mut declared = capabilities;
            declared.sort();
            declared.dedup();
            assert_eq!(
                endpoint,
                CustomEndpoint {
                    chain_id: chain,
                    api,
                    endpoint: url,
                    capabilities: declared,
                }
            );
            accepted += 1;
        }
        assert!(accepted > 100, "{accepted}");
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
        service.opened(db.clone()).await.unwrap();
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
            (
                crate::registry::Chain::Solana,
                EndpointApi::SolanaJsonRpc,
                "solana",
            ),
            (
                crate::registry::Chain::BitcoinSV,
                EndpointApi::Whatsonchain,
                "bsv",
            ),
        ] {
            let result = add(&service, chain, api, &format!("{}/{suffix}", server.uri())).await;
            assert_eq!(result.events, vec![StateEvent::AppSettingChanged]);
        }
        let reopened = WalletService::new_catalog().unwrap();
        reopened.opened(db.clone()).await.unwrap();
        assert_eq!(
            reopened
                .fetch_native_balance_summary(crate::registry::Chain::Solana, "test".into())
                .await
                .unwrap()
                .smallest_unit,
            "12345"
        );
        assert_eq!(
            reopened
                .fetch_native_balance_summary(crate::registry::Chain::BitcoinSV, "test".into())
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
                .endpoints_for(crate::registry::Chain::SolanaDevnet, &[])
                .await
                .iter()
                .any(|url| url.contains(&server.uri()))
        );
    }

    #[tokio::test]
    async fn custom_indexer_apis_are_used_separately_from_primary_rpc() {
        let server = MockServer::start().await;
        // One history page reads three lists: the address's transactions,
        // and its ERC-721 and ERC-1155 transfers.
        Mock::given(method("GET"))
            .and(path("/blockscout/api"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"status":"1","message":"OK","result":[]})),
            )
            .expect(3)
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
                crate::registry::Chain::Ethereum,
                EndpointApi::Blockscout,
                &format!("{}/blockscout", server.uri())
            )
            .await
            .events,
            vec![StateEvent::AppSettingChanged]
        );
        assert_eq!(
            add(
                &service,
                crate::registry::Chain::Tron,
                EndpointApi::TrongridV1,
                &format!("{}/v1/accounts", server.uri())
            )
            .await
            .events,
            vec![StateEvent::AppSettingChanged]
        );
        service
            .fetch_evm_history_page(
                crate::registry::Chain::Ethereum,
                "test".into(),
                vec![],
                1,
                10,
            )
            .await
            .unwrap();
        let accounts = service
            .api_endpoints(
                crate::registry::Chain::Tron,
                EndpointApi::TrongridV1,
                &[EndpointCapability::TokenDiscovery],
            )
            .await
            .unwrap();
        assert!(
            crate::api::trongrid_v1::TrongridClient::new(Arc::new(accounts))
                .fetch_trc20_holdings("test")
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !service
                .endpoints_for(crate::registry::Chain::Ethereum, &[])
                .await
                .iter()
                .any(|url| url.contains(&server.uri()))
        );
        assert!(
            !service
                .endpoints_for(crate::registry::Chain::Tron, &[])
                .await
                .iter()
                .any(|url| url.contains(&server.uri()))
        );
    }

    #[tokio::test]
    async fn invalid_and_duplicate_endpoints_leave_state_unchanged() {
        let service = WalletService::new_catalog().unwrap();
        for (chain, api, url) in [
            (
                crate::registry::Chain::Solana,
                EndpointApi::Esplora,
                "https://node.example",
            ),
            (
                crate::registry::Chain::Bitcoin,
                EndpointApi::Esplora,
                "file:///tmp/node",
            ),
            (
                crate::registry::Chain::Bitcoin,
                EndpointApi::Esplora,
                "https://a.example,nope",
            ),
            (
                crate::registry::Chain::Bitcoin,
                EndpointApi::Esplora,
                "https://user:secret@node.example",
            ),
            (
                crate::registry::Chain::Bitcoin,
                EndpointApi::Esplora,
                "https://blockstream.info/api/",
            ),
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
                crate::registry::Chain::Bitcoin,
                EndpointApi::Esplora,
                " https://node.example/api/ "
            )
            .await
            .events,
            vec![StateEvent::AppSettingChanged]
        );
        assert_eq!(
            add(
                &service,
                crate::registry::Chain::Bitcoin,
                EndpointApi::Esplora,
                "https://node.example/api"
            )
            .await
            .events,
            vec![StateEvent::AppSettingRejected]
        );
        assert_eq!(
            add(
                &service,
                crate::registry::Chain::Bitcoin,
                EndpointApi::Esplora,
                "https://other.example/api"
            )
            .await
            .events,
            vec![StateEvent::AppSettingChanged]
        );
        assert_eq!(
            &service
                .endpoints_for(crate::registry::Chain::Bitcoin, &[])
                .await[..2],
            &["https://other.example/api", "https://node.example/api"]
        );
    }
    #[tokio::test]
    async fn same_api_endpoints_keep_independent_capabilities_and_requests() {
        let service = WalletService::new_catalog().unwrap();
        let db = std::env::temp_dir().join(format!("endpoint-caps-{}.db", crate::store::new_id()));
        service.opened(db.to_string_lossy().into()).await.unwrap();
        let balance = MockServer::start().await;
        let broadcast = MockServer::start().await;
        for (url, capabilities) in [
            (balance.uri(), vec![EndpointCapability::Balance]),
            (broadcast.uri(), vec![EndpointCapability::Broadcast]),
        ] {
            let result = service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: AppSettingUpdate::AddCustomEndpoint {
                        chain_id: crate::registry::Chain::Ethereum,
                        api: crate::api::endpoint::EndpointApi::EvmJsonRpc,
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
        reopened.opened(db.to_string_lossy().into()).await.unwrap();
        assert_eq!(
            reopened
                .fetch_native_balance_summary(
                    crate::registry::Chain::Ethereum,
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
            [EndpointCapability::Balance]
        );
        assert!(
            !reopened
                .send_endpoints(crate::registry::Chain::Ethereum)
                .await
                .unwrap()
                .contains(&balance.uri())
        );
        assert!(
            reopened
                .send_endpoints(crate::registry::Chain::Ethereum)
                .await
                .unwrap()
                .contains(&broadcast.uri())
        );
        assert_eq!(balance.received_requests().await.unwrap().len(), 1);
        assert_eq!(broadcast.received_requests().await.unwrap().len(), 1);
        // Naming a saved endpoint alone cannot widen its declaration.
        reopened
            .use_only_endpoints(crate::registry::Chain::Ethereum, vec![balance.uri()])
            .await
            .unwrap();
        assert!(
            reopened
                .send_endpoints(crate::registry::Chain::Ethereum)
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
        for caps in [vec![], vec![EndpointCapability::History]] {
            assert!(
                CustomEndpoint::validated(
                    crate::registry::Chain::Ethereum,
                    crate::api::endpoint::EndpointApi::EvmJsonRpc,
                    "https://node.example".into(),
                    caps
                )
                .is_err()
            );
        }
        let endpoint = CustomEndpoint::validated(
            crate::registry::Chain::Ethereum,
            crate::api::endpoint::EndpointApi::EvmJsonRpc,
            "https://node.example".into(),
            vec![
                EndpointCapability::Fee,
                EndpointCapability::Balance,
                EndpointCapability::Fee,
            ],
        )
        .unwrap();
        assert_eq!(
            endpoint.capabilities,
            [EndpointCapability::Balance, EndpointCapability::Fee]
        );
        assert_eq!(
            endpoint.record().unwrap().capabilities,
            [EndpointCapability::Balance, EndpointCapability::Fee]
        );
    }
    #[tokio::test]
    async fn evm_preview_routes_balance_fee_and_context_to_separate_nodes() {
        let service = WalletService::new_catalog().unwrap();
        let balance = MockServer::start().await;
        let fees = MockServer::start().await;
        let context = MockServer::start().await;
        for (server, cap) in [
            (&balance, EndpointCapability::Balance),
            (&fees, EndpointCapability::Fee),
            (&context, EndpointCapability::Verification),
        ] {
            service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: AppSettingUpdate::AddCustomEndpoint {
                        chain_id: crate::registry::Chain::Ethereum,
                        api: crate::api::endpoint::EndpointApi::EvmJsonRpc,
                        endpoint: server.uri(),
                        capabilities: vec![cap],
                    },
                })
                .await
                .unwrap();
            Mock::given(method("POST"))
                .respond_with(move |request: &wiremock::Request| {
                    let body: serde_json::Value = request.body_json().unwrap();
                    let result = match (cap, body["method"].as_str().unwrap()) {
                        (EndpointCapability::Balance, "eth_getBalance") => {
                            json!("0xde0b6b3a7640000")
                        }
                        (EndpointCapability::Verification, "eth_getTransactionCount") => {
                            json!("0x7")
                        }
                        (EndpointCapability::Fee, "eth_estimateGas") => json!("0x5208"),
                        (EndpointCapability::Fee, "eth_feeHistory") => {
                            json!({"baseFeePerGas":["0x1"],"reward":[["0x2"]]})
                        }
                        _ => panic!("Request escaped its declared capability: {cap:?}: {body}"),
                    };
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                })
                .expect(if cap == EndpointCapability::Fee { 2 } else { 1 })
                .mount(server)
                .await;
        }
        service
            .fetch_evm_send_preview(
                crate::registry::Chain::Ethereum,
                format!("0x{}", "11".repeat(20)),
                crate::send::evm::TransferCall {
                    to: format!("0x{}", "22".repeat(20)),
                    value: 1,
                    data: Vec::new(),
                },
                None,
                None,
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
            (&balance, vec![EndpointCapability::Balance]),
            (&failing_balance, vec![EndpointCapability::Balance]),
            (&broadcast, vec![EndpointCapability::Broadcast]),
        ] {
            service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: AppSettingUpdate::AddCustomEndpoint {
                        chain_id: crate::registry::Chain::Ethereum,
                        api: crate::api::endpoint::EndpointApi::EvmJsonRpc,
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
            .use_only_endpoints(
                crate::registry::Chain::Ethereum,
                vec![broadcast.uri(), failing_balance.uri(), balance.uri()],
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .fetch_native_balance_summary(crate::registry::Chain::Ethereum, "test".into())
                .await
                .unwrap()
                .smallest_unit,
            "42"
        );
        assert_eq!(broadcast.received_requests().await.unwrap().len(), 0);
        assert_eq!(
            service
                .broadcast_raw(
                    crate::registry::Chain::Ethereum,
                    crate::send::payload::SignedPayload::Hex("0xdeadbeef".into()),
                )
                .await
                .unwrap(),
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
            .use_only_endpoints(crate::registry::Chain::Ethereum, vec![balance.uri()])
            .await
            .unwrap();
        assert!(
            service
                .broadcast_raw(
                    crate::registry::Chain::Ethereum,
                    crate::send::payload::SignedPayload::Hex("0xdeadbeef".into()),
                )
                .await
                .is_err()
        );
        assert_eq!(balance.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn whatsonchain_broadcast_uses_adapter_base_not_operation_path() {
        let service = WalletService::new_catalog().unwrap();
        assert_eq!(
            service
                .send_endpoints(crate::registry::Chain::BitcoinSV)
                .await
                .unwrap(),
            ["https://api.whatsonchain.com/v1/bsv/main"]
        );
    }
}

#[cfg(test)]
mod a_chain_reads_through_its_own_apis {
    use super::*;

    /// Litecoin's Esplora and BlockCypher rows are both its own endpoints;
    /// neither is set aside for the other, and no indexer of another API is
    /// handed to its clients.
    #[tokio::test]
    async fn a_chain_uses_every_api_it_speaks_and_no_other() {
        let service = WalletService::new_catalog().unwrap();
        let litecoin = service
            .chain_endpoints(crate::registry::Chain::Litecoin, &[])
            .await;
        for url in [
            "https://litecoinspace.org/api",
            "https://api.blockcypher.com/v1/ltc/main",
        ] {
            assert!(litecoin.iter().any(|endpoint| endpoint.url == url), "{url}");
        }
        for chain in crate::registry::Chain::all() {
            for endpoint in service.chain_endpoints(chain, &[]).await {
                assert!(chain.endpoint_apis().contains(&endpoint.api), "{chain}");
            }
        }
    }

    /// A URL named for a network's own APIs replaces every configured one of
    /// them for the service's life; its indexers stay, and an indexer named
    /// is used as configured.
    #[tokio::test]
    async fn named_endpoints_replace_the_networks_own_and_keep_its_indexers() {
        let service = WalletService::new_catalog().unwrap();
        let chain = crate::registry::Chain::Ethereum;
        let indexers = service
            .api_endpoints(chain, EndpointApi::Blockscout, &[])
            .await
            .unwrap();
        assert!(!indexers.is_empty());
        service
            .use_only_endpoints(chain, vec!["http://127.0.0.1:1/rpc".into()])
            .await
            .unwrap();
        let own = service.chain_endpoints(chain, &[]).await;
        assert_eq!(own.len(), 1);
        assert_eq!(
            (own[0].api, own[0].url.as_str()),
            (EndpointApi::EvmJsonRpc, "http://127.0.0.1:1/rpc")
        );
        assert_eq!(
            service.endpoint_records(chain).await[0].capabilities,
            crate::api::endpoint::endpoint_capability_options(chain, EndpointApi::EvmJsonRpc)
        );
        assert_eq!(
            service
                .api_endpoints(chain, EndpointApi::Blockscout, &[])
                .await
                .unwrap(),
            indexers
        );
        // An indexer named alone is used as configured, and leaves the
        // network's own endpoints as they were.
        service
            .use_only_endpoints(chain, vec![indexers[0].clone()])
            .await
            .unwrap();
        assert_eq!(service.chain_endpoints(chain, &[]).await, own);
    }
}

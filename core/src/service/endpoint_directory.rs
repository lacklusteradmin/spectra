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
    ) -> Result<Self, String> {
        let chain = Chain::from_str_id(&chain_id).ok_or("Unknown endpoint network")?;
        let catalog = crate::app_core::endpoint_catalog()?;
        let api = catalog
            .endpoint_records
            .iter()
            .filter(|r| r.chain_id == chain.str_id())
            .filter_map(|r| r.api)
            .find(|value| value.as_str() == api)
            .ok_or("API type is not in this network's endpoint directory")?;
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
        })
    }

    fn record(&self) -> Result<AppCoreEndpointRecord, String> {
        let catalog = crate::app_core::endpoint_catalog()?;
        let template = catalog
            .endpoint_records
            .iter()
            .find(|r| r.chain_id == self.chain_id && r.api == Some(self.api))
            .ok_or("Endpoint API is absent from the directory")?;
        let mut record = template.clone();
        record.id = format!(
            "custom:{}:{}:{}",
            self.chain_id,
            self.api.as_str(),
            self.endpoint
        );
        record.endpoint = self.endpoint.clone();
        record.probe_url = template.probe_url.as_ref().and_then(|url| {
            url.strip_prefix(&template.endpoint)
                .map(|suffix| format!("{}{suffix}", self.endpoint))
        });
        Ok(record)
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
    pub(crate) async fn api_endpoints(
        &self,
        chain: Chain,
        api: EndpointApi,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let mut urls = self.custom_api_endpoints(chain, api).await;
        for record in &crate::app_core::endpoint_catalog()?.endpoint_records {
            if record.chain_id == chain.str_id()
                && record.api == Some(api)
                && !urls.contains(&record.endpoint)
            {
                urls.push(record.endpoint.clone());
            }
        }
        Ok(urls)
    }

    pub(crate) async fn custom_api_endpoints(&self, chain: Chain, api: EndpointApi) -> Vec<String> {
        self.wallet_state
            .read()
            .await
            .settings
            .custom_endpoints
            .iter()
            .filter(|e| e.chain_id == chain.str_id() && e.api == api)
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
                        endpoints: self.endpoints_for(&chain_id).await.as_ref().clone(),
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
        assert!(!reopened
            .endpoints_for("solana-devnet")
            .await
            .iter()
            .any(|url| url.contains(&server.uri())));
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
        assert!(service
            .discover_token_balances("tron".into(), "test".into())
            .await
            .unwrap()
            .is_empty());
        assert!(!service
            .endpoints_for("ethereum")
            .await
            .iter()
            .any(|url| url.contains(&server.uri())));
        assert!(!service
            .endpoints_for("tron")
            .await
            .iter()
            .any(|url| url.contains(&server.uri())));
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
        assert!(service
            .app_state()
            .await
            .settings
            .custom_endpoints
            .is_empty());
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
            &service.endpoints_for("bitcoin").await[..2],
            &["https://other.example/api", "https://node.example/api"]
        );
    }
}

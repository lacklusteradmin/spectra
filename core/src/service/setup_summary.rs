//! What a wallet on a network will do once added, and whom its first refresh
//! asks — for the setup page's last step, before anything is committed.
//!
//! The capabilities and limits are registry facts; the endpoints are the ones
//! configured now, so the page can name them and the user can change them
//! before the wallet's address is sent anywhere.

use super::*;
use crate::{EndpointApi, endpoint_capability_options};

/// How a network serves one kind of read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityCoverage {
    /// A configured endpoint serves it.
    Configured,
    /// Spectra reads it, but only from an endpoint the user adds: a custom
    /// indexer, or any provider on a network with no built-in one.
    NeedsCustomEndpoint,
    /// No provider Spectra speaks serves it on this network.
    Unavailable,
}

/// A limit a wallet on the network lives with, said before it is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletSetupLimit {
    /// The ledger creates an account only once it holds the network's
    /// reserve, so a first receive below it fails (XRP, Stellar).
    AccountReserve,
    /// ADA payments leave token-bearing inputs untouched, so ADA held beside
    /// native assets cannot pay (Cardano).
    TokenBearingInputsUntouched,
    /// Shielded funds are found by scanning blocks on this device from the
    /// restore height on, and only a wallet restored from its seed phrase
    /// holds them (Zcash).
    ShieldedScan,
    /// Balance and history come from scanning blocks on this device from the
    /// restore height on, through a daemon (Monero).
    ScansOnDevice,
}

/// What a new account on a reserve network must receive to exist, read from
/// the network as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AccountReserve {
    pub chain: Chain,
    /// The minimum, as an exact decimal in the native coin.
    pub amount: String,
    pub symbol: String,
}

/// One endpoint the first refresh reads from, with what it is asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SetupEndpoint {
    pub endpoint: String,
    pub api: EndpointApi,
    pub capabilities: Vec<EndpointCapability>,
    pub is_built_in: bool,
}

/// What a wallet on one network will do, and whom it asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSetupSummary {
    pub chain: Chain,
    pub balance: CapabilityCoverage,
    pub history: CapabilityCoverage,
    /// Token discovery, or `None` on a network that hosts no tokens.
    pub token_discovery: Option<CapabilityCoverage>,
    pub staking: bool,
    pub limits: Vec<WalletSetupLimit>,
    /// The endpoints the first refresh reads from, the user's first.
    pub endpoints: Vec<SetupEndpoint>,
    /// Whether only the user's endpoints are used on this network.
    pub custom_endpoints_only: bool,
}

/// The reads a wallet's refresh makes; sending, fees and staking come later
/// and are asked for when used.
const REFRESH_READS: [EndpointCapability; 6] = [
    EndpointCapability::Balance,
    EndpointCapability::History,
    EndpointCapability::TokenHistory,
    EndpointCapability::TokenDiscovery,
    EndpointCapability::TokenBalance,
    EndpointCapability::Utxo,
];

/// The limits a wallet on `chain` lives with, from the registry.
pub(crate) fn wallet_setup_limits(chain: Chain) -> Vec<WalletSetupLimit> {
    let family = chain.mainnet_counterpart();
    [
        (
            chain.requires_account_reserve(),
            WalletSetupLimit::AccountReserve,
        ),
        (
            family == Chain::Cardano,
            WalletSetupLimit::TokenBearingInputsUntouched,
        ),
        (family == Chain::Zcash, WalletSetupLimit::ShieldedScan),
        (chain.scans_for_balance(), WalletSetupLimit::ScansOnDevice),
    ]
    .into_iter()
    .filter_map(|(applies, limit)| applies.then_some(limit))
    .collect()
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// What a wallet on `chain` will do once added — its reads, staking and
    /// limits from the registry — and the endpoints its first refresh reads
    /// from as configured now. Contacts nothing.
    pub async fn wallet_setup_summary(&self, chain: Chain) -> WalletSetupSummary {
        let custom_endpoints_only = self.uses_custom_endpoints_only(chain).await;
        let apis = chain.compatible_endpoint_apis();
        let configured = self.configured_endpoints_on(chain).await;
        let coverage = |capability: EndpointCapability| {
            if configured
                .iter()
                .any(|endpoint| endpoint.capabilities.contains(&capability))
            {
                CapabilityCoverage::Configured
            } else if apis
                .iter()
                .any(|api| endpoint_capability_options(chain, *api).contains(&capability))
            {
                CapabilityCoverage::NeedsCustomEndpoint
            } else {
                CapabilityCoverage::Unavailable
            }
        };
        // A scanning wallet reads both from the daemon it verifies.
        let (balance, history) = if chain.scans_for_balance() {
            let daemon = coverage(EndpointCapability::Verification);
            (daemon, daemon)
        } else {
            (
                coverage(EndpointCapability::Balance),
                coverage(EndpointCapability::History),
            )
        };
        let reads: &[EndpointCapability] = if chain.scans_for_balance() {
            &[EndpointCapability::Verification]
        } else {
            &REFRESH_READS
        };
        let endpoints = reading(&configured, reads);
        WalletSetupSummary {
            chain,
            balance,
            history,
            token_discovery: chain
                .hosts_tokens()
                .then(|| coverage(EndpointCapability::TokenDiscovery)),
            staking: chain.supports_staking(),
            limits: wallet_setup_limits(chain),
            endpoints,
            custom_endpoints_only,
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The least a new account on `chain` must receive for the ledger to
    /// create it — XRP's base reserve, Stellar's two base reserves — read
    /// from a verified endpoint, so a receive screen can say it before the
    /// first payment. Refuses a network without a reserve.
    pub async fn account_reserve(
        &self,
        chain: Chain,
    ) -> Result<AccountReserve, SpectraBridgeError> {
        if !chain.requires_account_reserve() {
            return Err(crate::derivation::error::DerivationError::refused(
                "%@ has no account reserve.",
                [chain.chain_display_name()],
            )
            .into());
        }
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let smallest = if chain.mainnet_counterpart() == Chain::Xrp {
            let client = crate::api::xrpl_json_rpc::XrplClient::new(endpoints);
            client.verify_network(chain).await?;
            u128::from(client.fetch_base_reserve().await?)
        } else {
            let client = crate::api::horizon::HorizonClient::new(endpoints);
            client.verify_network(chain).await?;
            u128::from(client.fetch_base_reserve().await?) * 2
        };
        Ok(AccountReserve {
            chain,
            amount: crate::decimal::from_units(smallest, u32::from(chain.native_decimals())),
            symbol: chain.coin_symbol().to_string(),
        })
    }
}

impl WalletService {
    /// Every endpoint configured on `chain` in an API it speaks: the user's
    /// first, then the catalog's unless the user uses only theirs there.
    pub(crate) async fn configured_endpoints_on(&self, chain: Chain) -> Vec<SetupEndpoint> {
        let settings = self.wallet_state.read().await.settings.clone();
        let apis = chain.compatible_endpoint_apis();
        let mut configured: Vec<SetupEndpoint> = settings
            .custom_endpoints
            .iter()
            .filter(|endpoint| endpoint.chain_id == chain && apis.contains(&endpoint.api))
            .map(|endpoint| SetupEndpoint {
                endpoint: endpoint.endpoint.clone(),
                api: endpoint.api,
                capabilities: endpoint.capabilities.clone(),
                is_built_in: false,
            })
            .collect();
        if !settings.custom_endpoints_only.contains(&chain) {
            configured.extend(
                crate::endpoints::records_for_chain(chain, &[])
                    .into_iter()
                    .filter(|record| apis.contains(&record.api))
                    .map(|record| SetupEndpoint {
                        endpoint: record.endpoint,
                        api: record.api,
                        capabilities: record.capabilities,
                        is_built_in: true,
                    }),
            );
        }
        configured
    }
}

/// The endpoints in `configured` that serve any of `reads`, each with just
/// those it serves.
pub(crate) fn reading(
    configured: &[SetupEndpoint],
    reads: &[EndpointCapability],
) -> Vec<SetupEndpoint> {
    configured
        .iter()
        .filter_map(|endpoint| {
            let capabilities: Vec<_> = endpoint
                .capabilities
                .iter()
                .copied()
                .filter(|capability| reads.contains(capability))
                .collect();
            (!capabilities.is_empty()).then(|| SetupEndpoint {
                capabilities,
                ..endpoint.clone()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With the user's endpoints alone, neither the chain's transport nor
    /// an indexer reaches a catalog URL, and the summary names only theirs;
    /// turned off, the catalog's return after the user's.
    #[tokio::test]
    async fn custom_endpoints_only_keeps_every_catalog_url_out() {
        use crate::store::state::{AppSettingUpdate, StateCommand};
        let directory = std::env::temp_dir().join(crate::store::new_event_id());
        std::fs::create_dir_all(&directory).unwrap();
        let service = WalletService::new_catalog().unwrap();
        service
            .open_state(directory.join("state.db").to_string_lossy().into_owned())
            .await
            .unwrap();
        let chain = Chain::BaseSepolia;
        let set = |update| StateCommand::SetAppSetting { update };
        for (url, api, capabilities) in [
            (
                "http://127.0.0.1:1/rpc",
                "evm-json-rpc",
                vec![EndpointCapability::Balance],
            ),
            (
                "http://127.0.0.1:1/scout",
                "blockscout",
                vec![EndpointCapability::History],
            ),
        ] {
            service
                .apply_state_command(set(AppSettingUpdate::AddCustomEndpoint {
                    capabilities,
                    chain_id: chain,
                    api: api.into(),
                    endpoint: url.into(),
                }))
                .await
                .unwrap();
        }
        let catalog = crate::endpoints::records_for_chain(chain, &[]);
        assert!(!catalog.is_empty());
        let reaches_catalog = |urls: &[String]| {
            urls.iter()
                .any(|url| catalog.iter().any(|record| record.endpoint == *url))
        };
        let history = || {
            service.api_endpoints(
                chain,
                EndpointApi::Blockscout,
                &[EndpointCapability::History],
            )
        };
        assert!(reaches_catalog(
            &service.configured_endpoint_urls(chain).await
        ));
        assert!(reaches_catalog(&history().await.unwrap()));
        service
            .apply_state_command(set(AppSettingUpdate::CustomEndpointsOnly {
                chain_id: chain,
                value: true,
            }))
            .await
            .unwrap();
        assert_eq!(
            *service.configured_endpoint_urls(chain).await,
            ["http://127.0.0.1:1/rpc"]
        );
        assert_eq!(history().await.unwrap(), ["http://127.0.0.1:1/scout"]);
        let summary = service.wallet_setup_summary(chain).await;
        assert!(summary.custom_endpoints_only);
        assert!(
            summary
                .endpoints
                .iter()
                .all(|endpoint| !endpoint.is_built_in)
        );
        assert_eq!(summary.endpoints.len(), 2);
        service
            .apply_state_command(set(AppSettingUpdate::CustomEndpointsOnly {
                chain_id: chain,
                value: false,
            }))
            .await
            .unwrap();
        let urls = service.configured_endpoint_urls(chain).await;
        assert_eq!(urls[0], "http://127.0.0.1:1/rpc");
        assert!(reaches_catalog(&urls));
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// The reserve is read from a verified endpoint: XRP's base reserve, and
    /// two of Stellar's; a network without one is refused, and so is an
    /// endpoint on another network.
    #[tokio::test]
    async fn the_account_reserve_is_the_networks_own() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};
        let xrpl = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(|request: &Request| {
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                let result = match body["method"].as_str().unwrap() {
                    "server_info" => json!({"info": {"network_id": 0}}),
                    "server_state" => {
                        json!({"state": {"validated_ledger": {"reserve_base": 1_000_000}}})
                    }
                    other => panic!("unexpected XRPL call {other}"),
                };
                ResponseTemplate::new(200).set_body_json(json!({"result": result}))
            })
            .mount(&xrpl)
            .await;
        let horizon = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "network_passphrase": "Public Global Stellar Network ; September 2015"
            })))
            .mount(&horizon)
            .await;
        Mock::given(method("GET"))
            .and(path("/ledgers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "_embedded": {"records": [{"base_reserve_in_stroops": 5_000_000}]}
            })))
            .mount(&horizon)
            .await;
        let endpoints = |chain, server: &MockServer| ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        };
        let service = WalletService::new(vec![
            endpoints(Chain::Xrp, &xrpl),
            endpoints(Chain::Stellar, &horizon),
            endpoints(Chain::XrpTestnet, &xrpl),
        ])
        .unwrap();
        let xrp = service.account_reserve(Chain::Xrp).await.unwrap();
        assert_eq!((xrp.amount.as_str(), xrp.symbol.as_str()), ("1", "XRP"));
        let xlm = service.account_reserve(Chain::Stellar).await.unwrap();
        assert_eq!((xlm.amount.as_str(), xlm.symbol.as_str()), ("1", "XLM"));
        assert!(service.account_reserve(Chain::Ethereum).await.is_err());
        // The testnet's endpoint answers as mainnet: refused.
        assert!(service.account_reserve(Chain::XrpTestnet).await.is_err());
    }

    /// A read Spectra serves only from an endpoint the user adds needs one:
    /// Dash's test network has no built-in provider at all, and BNB Smart
    /// Chain no built-in history indexer. A read a built-in endpoint serves is
    /// configured, until the user uses only their own and has none.
    #[tokio::test]
    async fn a_read_no_configured_endpoint_serves_needs_a_custom_one() {
        use crate::store::state::{AppSettingUpdate, StateCommand};
        use CapabilityCoverage::{Configured, NeedsCustomEndpoint};
        let service = WalletService::new(vec![]).unwrap();
        let dash = service.wallet_setup_summary(Chain::DashTestnet).await;
        assert_eq!(
            (dash.balance, dash.history),
            (NeedsCustomEndpoint, NeedsCustomEndpoint)
        );
        assert!(dash.endpoints.is_empty());
        let bnb = service.wallet_setup_summary(Chain::BnbChain).await;
        assert_eq!(
            (bnb.balance, bnb.history),
            (Configured, NeedsCustomEndpoint)
        );
        let base = service.wallet_setup_summary(Chain::BaseSepolia).await;
        assert_eq!(base.balance, Configured);
        assert!(!base.endpoints.is_empty());
        assert!(base.endpoints.iter().all(|endpoint| endpoint.is_built_in));
        service
            .apply_state_command(StateCommand::SetAppSetting {
                update: AppSettingUpdate::CustomEndpointsOnly {
                    chain_id: Chain::BaseSepolia,
                    value: true,
                },
            })
            .await
            .unwrap();
        let only = service.wallet_setup_summary(Chain::BaseSepolia).await;
        assert!(only.custom_endpoints_only);
        assert_eq!(only.balance, NeedsCustomEndpoint);
        assert!(only.endpoints.is_empty());
    }

    /// The limits are the registry's, on the networks the product names.
    #[test]
    fn limits_follow_the_registry() {
        use WalletSetupLimit::*;
        assert_eq!(wallet_setup_limits(Chain::Xrp), [AccountReserve]);
        assert_eq!(wallet_setup_limits(Chain::StellarTestnet), [AccountReserve]);
        assert_eq!(
            wallet_setup_limits(Chain::Cardano),
            [TokenBearingInputsUntouched]
        );
        assert_eq!(wallet_setup_limits(Chain::Zcash), [ShieldedScan]);
        assert_eq!(wallet_setup_limits(Chain::Monero), [ScansOnDevice]);
        assert!(wallet_setup_limits(Chain::Bitcoin).is_empty());
    }
}

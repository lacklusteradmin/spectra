//! Durable diagnostics are core state; platforms supply events, never replacement lists.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DiagnosticState {
    pub degraded: HashMap<String, String>,
    pub last_good_unix: HashMap<String, f64>,
    pub logs: Vec<DiagnosticLog>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DiagnosticLog {
    pub id: String,
    pub timestamp_unix: f64,
    pub input: DiagnosticLogInput,
}
/// How serious a diagnostic log line is.
///
/// A free string before, checked against a list on append and parsed back by
/// each reader with a fallback for anything else — the app dropped a line whose
/// level it did not recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLogLevel {
    Debug,
    Info,
    Warning,
    Error,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DiagnosticLogInput {
    pub level: DiagnosticLogLevel,
    pub category: String,
    pub message: String,
    pub chain_name: Option<String>,
    pub wallet_id: Option<String>,
    pub transaction_hash: Option<String>,
    pub source: Option<String>,
    pub metadata: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
pub enum DiagnosticCommand {
    Append { input: DiagnosticLogInput },
    Healthy { chain_name: String },
    Synced { chain_name: String },
    Degraded { chain_name: String, detail: String },
    ClearLogs { chain_name: Option<String> },
    Reset,
}
impl DiagnosticState {
    fn append(&mut self, mut input: DiagnosticLogInput) {
        input.category = input.category.trim().into();
        input.message = input.message.trim().into();
        for text in [
            &mut input.chain_name,
            &mut input.wallet_id,
            &mut input.transaction_hash,
            &mut input.source,
            &mut input.metadata,
        ] {
            *text = text
                .take()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        self.logs.insert(
            0,
            DiagnosticLog {
                id: crate::store::new_transaction_id(),
                timestamp_unix: crate::store::now_unix(),
                input,
            },
        );
        self.logs.truncate(800);
    }
    pub(crate) fn forget_wallet(&mut self, wallet_id: &str) {
        self.logs
            .retain(|l| l.input.wallet_id.as_deref() != Some(wallet_id));
    }
}
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn diagnostic_state(&self) -> DiagnosticState {
        self.wallet_state.read().await.diagnostics.clone()
    }
    pub async fn apply_diagnostic_command(
        &self,
        command: DiagnosticCommand,
    ) -> Result<DiagnosticState, SpectraBridgeError> {
        let chain_name = match &command {
            DiagnosticCommand::Healthy { chain_name }
            | DiagnosticCommand::Synced { chain_name }
            | DiagnosticCommand::Degraded { chain_name, .. } => Some(chain_name),
            _ => None,
        };
        if chain_name.is_some_and(|s| Chain::from_display_name(s).is_none()) {
            return Err("unknown diagnostic chain".into());
        }
        let result = self
            .mutate_persisted_state(move |state| {
                let d = &mut state.diagnostics;
                match command {
                    DiagnosticCommand::Append { input } => d.append(input),
                    DiagnosticCommand::Synced { chain_name } => {
                        d.last_good_unix
                            .insert(chain_name, crate::store::now_unix());
                    }
                    DiagnosticCommand::Healthy { chain_name } => {
                        d.last_good_unix
                            .insert(chain_name.clone(), crate::store::now_unix());
                        if d.degraded.remove(&chain_name).is_some() {
                            d.append(sync_log(
                                chain_name,
                                DiagnosticLogLevel::Info,
                                "Chain recovered".into(),
                            ));
                        }
                    }
                    DiagnosticCommand::Degraded { chain_name, detail } => {
                        let classified =
                            crate::diagnostics::diagnostics_classify_degraded_detail(detail);
                        if classified.indicates_live_success {
                            d.last_good_unix
                                .insert(chain_name.clone(), crate::store::now_unix());
                        }
                        d.degraded
                            .insert(chain_name.clone(), classified.normalized.clone());
                        d.append(sync_log(
                            chain_name,
                            DiagnosticLogLevel::Warning,
                            classified.normalized,
                        ));
                    }
                    DiagnosticCommand::ClearLogs { chain_name } => d
                        .logs
                        .retain(|l| chain_name.is_some() && l.input.chain_name != chain_name),
                    DiagnosticCommand::Reset => *d = DiagnosticState::default(),
                }
                vec![crate::store::state::StateEvent::DiagnosticsChanged]
            })
            .await?;
        Ok(result.state.diagnostics)
    }
}
fn sync_log(chain: String, level: DiagnosticLogLevel, message: String) -> DiagnosticLogInput {
    DiagnosticLogInput {
        level,
        category: "Chain Sync".into(),
        message,
        chain_name: Some(chain),
        wallet_id: None,
        transaction_hash: None,
        source: Some("network".into()),
        metadata: None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn diagnostic_intents_persist_and_recover() {
        let service = WalletService::new(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "diagnostic-owned-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        service
            .apply_diagnostic_command(DiagnosticCommand::Degraded {
                chain_name: "Solana".into(),
                detail: "timeout".into(),
            })
            .await
            .unwrap();
        let d = service
            .apply_diagnostic_command(DiagnosticCommand::Healthy {
                chain_name: "Solana".into(),
            })
            .await
            .unwrap();
        assert!(d.degraded.is_empty());
        assert_eq!(d.logs.len(), 2);
        assert!(d.last_good_unix.contains_key("Solana"));
        let reopened = WalletService::new(vec![]).unwrap();
        assert_eq!(
            reopened
                .open_state(path.to_string_lossy().into())
                .await
                .unwrap()
                .diagnostics,
            d
        );
    }
}

/// Diagnostics of the service's selected network and first configured RPC.
/// The selected endpoint is explicit in the result; a failure never silently
/// tests a different provider and reports it as the configured node.
#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ConfiguredSelfTestReport {
    pub chain_id: String,
    pub rpc_endpoint: Option<String>,
    pub results: Vec<crate::diagnostics::self_tests::ChainSelfTestResult>,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn run_configured_self_tests(
        &self,
        chain_id: String,
    ) -> Result<ConfiguredSelfTestReport, SpectraBridgeError> {
        use crate::diagnostics::self_tests::{self_tests_run_chain, self_tests_run_evm_rpc};
        let requested = chain_for_id(&chain_id)?;
        let chain = if requested == requested.mainnet_counterpart() {
            self.app_state()
                .await
                .settings
                .selected_chain_for_family(requested)
        } else {
            requested
        };
        let mut results = self_tests_run_chain(chain.chain_display_name().into());
        let rpc_endpoint = if chain.is_evm() {
            let endpoints = self.configured_endpoint_urls(chain.str_id()).await;
            let rpc = endpoints
                .first()
                .ok_or("No RPC configured for this network")?
                .clone();
            results.extend(
                self_tests_run_evm_rpc(chain.str_id().into(), rpc.clone(), rpc.clone()).await,
            );
            Some(rpc)
        } else {
            None
        };
        Ok(ConfiguredSelfTestReport {
            chain_id: chain.str_id().into(),
            rpc_endpoint,
            results,
        })
    }
}

#[cfg(test)]
mod configured_tests {
    use super::*;
    use serde_json::json;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

    #[tokio::test]
    async fn configured_diagnostics_follow_selected_network_and_report_wrong_chain() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|r: &Request| {
                let request: serde_json::Value = r.body_json().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => "0xaa36a7", // Sepolia
                    "eth_blockNumber" => "0x123",
                    other => panic!("unexpected {other}"),
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
            })
            .mount(&server)
            .await;
        let service = WalletService::new_catalog().unwrap();
        for chain in ["Ethereum", "Ethereum Sepolia"] {
            service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                        capabilities: vec![
                            "balance".into(),
                            "fee".into(),
                            "broadcast".into(),
                            "verification".into(),
                        ],
                        chain_id: crate::registry::Chain::from_display_name(chain)
                            .unwrap()
                            .str_id()
                            .into(),
                        api: "evm-json-rpc".into(),
                        endpoint: server.uri(),
                    },
                })
                .await
                .unwrap();
        }
        service
            .apply_state_command(StateCommand::SelectChainForFamily {
                chain_id: "ethereum-sepolia".into(),
            })
            .await
            .unwrap();
        let selected = service
            .run_configured_self_tests("ethereum".into())
            .await
            .unwrap();
        assert_eq!(selected.chain_id, "ethereum-sepolia");
        assert_eq!(
            selected.rpc_endpoint.as_deref(),
            Some(server.uri().as_str())
        );
        assert!(
            selected.results.iter().all(|r| r.passed),
            "{:?}",
            selected.results
        );
        service
            .apply_state_command(StateCommand::SelectChainForFamily {
                chain_id: "ethereum".into(),
            })
            .await
            .unwrap();
        let mainnet = service
            .run_configured_self_tests("ethereum".into())
            .await
            .unwrap();
        assert!(mainnet
            .results
            .iter()
            .any(|r| r.name == "RPC Chain ID" && !r.passed));
        let explicit = service
            .run_configured_self_tests("ethereum-sepolia".into())
            .await
            .unwrap();
        assert!(explicit.results.iter().all(|r| r.passed));
        assert_eq!(server.received_requests().await.unwrap().len(), 6);
    }
}

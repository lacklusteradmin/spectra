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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DiagnosticLogInput {
    pub level: String,
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
        if let DiagnosticCommand::Append { input } = &command {
            if !["debug", "info", "warning", "error"].contains(&input.level.as_str()) {
                return Err("invalid diagnostic level".into());
            }
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
                            d.append(sync_log(chain_name, "info", "Chain recovered".into()));
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
                        d.append(sync_log(chain_name, "warning", classified.normalized));
                    }
                    DiagnosticCommand::ClearLogs { chain_name } => d
                        .logs
                        .retain(|l| chain_name.is_some() && l.input.chain_name != chain_name),
                    DiagnosticCommand::Reset => *d = DiagnosticState::default(),
                }
                vec![crate::store::state::StateEvent {
                    kind: "diagnosticsChanged".into(),
                    subject_id: None,
                }]
            })
            .await?;
        Ok(result.state.diagnostics)
    }
}
fn sync_log(chain: String, level: &str, message: String) -> DiagnosticLogInput {
    DiagnosticLogInput {
        level: level.into(),
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
        let service = WalletService::new_typed(vec![]).unwrap();
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
        let reopened = WalletService::new_typed(vec![]).unwrap();
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

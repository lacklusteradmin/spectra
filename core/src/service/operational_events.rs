//! Per-chain views of the shared durable diagnostic log.
use super::*;
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn append_chain_operational_event(
        &self,
        chain_name: String,
        level: crate::store::ChainOperationalEventLevel,
        message: String,
        transaction_hash: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let level = match level {
            crate::store::ChainOperationalEventLevel::Info => "info",
            crate::store::ChainOperationalEventLevel::Warning => "warning",
            crate::store::ChainOperationalEventLevel::Error => "error",
        };
        self.apply_diagnostic_command(DiagnosticCommand::Append {
            input: DiagnosticLogInput {
                level: level.into(),
                category: "Chain Operations".into(),
                message,
                chain_name: Some(chain_name),
                transaction_hash,
                wallet_id: None,
                source: Some("core".into()),
                metadata: None,
            },
        })
        .await
        .map(|_| ())
    }
    pub async fn operational_events(
        &self,
        chain_name: String,
    ) -> Vec<crate::store::ChainOperationalEventRecord> {
        self.diagnostic_state()
            .await
            .logs
            .into_iter()
            .filter(|l| l.input.chain_name.as_deref() == Some(&chain_name))
            .take(200)
            .map(|l| crate::store::ChainOperationalEventRecord {
                id: l.id,
                timestamp_unix: l.timestamp_unix,
                chain_name: chain_name.clone(),
                message: l.input.message,
                transaction_hash: l.input.transaction_hash,
                level: match l.input.level.as_str() {
                    "warning" => crate::store::ChainOperationalEventLevel::Warning,
                    "error" => crate::store::ChainOperationalEventLevel::Error,
                    _ => crate::store::ChainOperationalEventLevel::Info,
                },
            })
            .collect()
    }
    pub async fn clear_operational_events(
        &self,
        chain_name: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        self.apply_diagnostic_command(DiagnosticCommand::ClearLogs { chain_name })
            .await
            .map(|_| ())
    }
}

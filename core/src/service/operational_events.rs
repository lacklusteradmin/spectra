//! Per-chain views of the shared durable diagnostic log.
use super::*;
impl WalletService {
    /// Record something that happened on a chain. Internal: the app writes its
    /// events through `apply_diagnostic_command`, and nothing else called this
    /// across the boundary.
    pub async fn append_chain_operational_event(
        &self,
        chain_name: String,
        level: DiagnosticLogLevel,
        message: String,
        transaction_hash: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        self.apply_diagnostic_command(DiagnosticCommand::Append {
            input: DiagnosticLogInput {
                level,
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
    /// Forget a chain's events, or every chain's. Internal, like the append.
    pub async fn clear_operational_events(
        &self,
        chain_name: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        self.apply_diagnostic_command(DiagnosticCommand::ClearLogs { chain_name })
            .await
            .map(|_| ())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// One chain's log lines, newest first, at most 200.
    ///
    /// The durable log's own rows. This returned a second record shape with a
    /// second level enum, mapped from the first with a fallback that turned an
    /// unknown level into `info`.
    pub async fn operational_events(&self, chain_name: String) -> Vec<DiagnosticLog> {
        self.diagnostic_state()
            .await
            .logs
            .into_iter()
            .filter(|l| l.input.chain_name.as_deref() == Some(&chain_name))
            .take(200)
            .collect()
    }
}

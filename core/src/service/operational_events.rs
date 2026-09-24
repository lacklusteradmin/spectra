//! Per-chain views of the shared durable diagnostic log.
use super::*;
impl WalletService {
    /// Record something core did. Core writes the events for work it performs —
    /// status changes, broadcasts, rescans, self-tests, refresh failures — so
    /// every front end, the CLI included, gets the same log. Front ends append
    /// only what failed on their side of the boundary. English: the log is read
    /// by whoever debugs it. Best effort: an unwritable log never fails the work.
    pub(crate) async fn record_event(
        &self,
        level: DiagnosticLogLevel,
        category: &str,
        message: String,
        chain_id: Option<String>,
        transaction_hash: Option<String>,
    ) {
        let _ = self
            .apply_diagnostic_command(DiagnosticCommand::Append {
                input: DiagnosticLogInput {
                    level,
                    category: category.into(),
                    message,
                    chain_id,
                    transaction_hash,
                    wallet_id: None,
                    source: Some("core".into()),
                    metadata: None,
                },
            })
            .await;
    }

    /// One line per transaction whose stored status a poll or recheck changed.
    pub(crate) async fn record_status_changes(
        &self,
        changes: &[crate::store::TransactionStatusChange],
    ) {
        use crate::store::wallet_domain::CoreTransactionStatus as Status;
        for change in changes.iter().filter(|c| c.status_changed) {
            let (level, message) = match change.new_status {
                Status::Confirmed => (DiagnosticLogLevel::Info, "Transaction confirmed on-chain."),
                Status::Failed => (DiagnosticLogLevel::Error, "Transaction failed."),
                Status::Pending => (DiagnosticLogLevel::Info, "Transaction is pending again."),
            };
            self.record_event(
                level,
                "Transaction Status",
                message.into(),
                Some(change.chain_id.clone()),
                change.transaction_hash.clone(),
            )
            .await;
        }
    }

    /// Record something that happened on a chain. A test fixture.
    #[cfg(test)]
    pub(crate) async fn append_chain_operational_event(
        &self,
        chain_id: String,
        level: DiagnosticLogLevel,
        message: String,
        transaction_hash: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        self.apply_diagnostic_command(DiagnosticCommand::Append {
            input: DiagnosticLogInput {
                level,
                category: "Chain Operations".into(),
                message,
                chain_id: Some(chain_id),
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
        chain_id: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        self.apply_diagnostic_command(DiagnosticCommand::ClearLogs { chain_id })
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
    pub async fn operational_events(&self, chain_id: String) -> Vec<DiagnosticLog> {
        self.diagnostic_state()
            .await
            .logs
            .into_iter()
            .filter(|l| l.input.chain_id.as_deref() == Some(&chain_id))
            .take(200)
            .collect()
    }
}

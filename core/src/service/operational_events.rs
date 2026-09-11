//! Persisted per-chain operational events.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Record something that happened on a chain — a broadcast accepted, a
    /// verification warning, a send failure.
    ///
    /// Core stamps the id and the time and applies the cap. The planner this
    /// replaces took the existing list in and handed a capped one back, so the
    /// list, its ordering and its bound were only as correct as whichever
    /// caller wrote the answer down.
    pub async fn append_chain_operational_event(
        &self,
        chain_name: String,
        level: crate::store::ChainOperationalEventLevel,
        message: String,
        transaction_hash: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let event = crate::store::ChainOperationalEventRecord {
                id: crate::store::new_event_id(),
                timestamp_unix: crate::store::now_unix(),
                chain_name: chain_name.clone(),
                level,
                message,
                transaction_hash: transaction_hash.filter(|hash| !hash.trim().is_empty()),
            };
            let snapshot = {
                let mut table = service.operational_events.read().await.clone();
                let existing = table.remove(&chain_name).unwrap_or_default();
                table.insert(
                    chain_name,
                    crate::store::plan_append_chain_operational_event(existing, event),
                );
                (serde_json::to_string(&table)?, table)
            };
            service.persist_operational_events(snapshot.0).await?;
            *service.operational_events.write().await = snapshot.1;
            Ok(())
        })
        .await
    }

    /// This chain's events, newest first.
    pub async fn operational_events(
        &self,
        chain_name: String,
    ) -> Vec<crate::store::ChainOperationalEventRecord> {
        self.operational_events
            .read()
            .await
            .get(&chain_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Drop one chain's events, or every chain's when `chain_name` is absent.
    pub async fn clear_operational_events(
        &self,
        chain_name: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let snapshot = {
                let mut table = service.operational_events.read().await.clone();
                match chain_name {
                    Some(chain) => {
                        table.remove(&chain);
                    }
                    None => table.clear(),
                }
                (serde_json::to_string(&table)?, table)
            };
            service.persist_operational_events(snapshot.0).await?;
            *service.operational_events.write().await = snapshot.1;
            Ok(())
        })
        .await
    }
}

impl WalletService {
    pub(super) async fn persist_operational_events(
        &self,
        snapshot: String,
    ) -> Result<(), SpectraBridgeError> {
        let Some(db_path) = self.state_db_path.read().await.clone() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || {
            sqlite_save(&db_path, OPERATIONAL_EVENTS_KEY, &snapshot)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }
}

pub(super) const OPERATIONAL_EVENTS_KEY: &str = "operationalEvents.byChain.v1";

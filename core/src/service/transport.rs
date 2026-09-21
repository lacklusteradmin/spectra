//! Transport lifecycle follows committed service settings, not UI projections.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn configure_network_runtime(
        &self,
        cache_dir: String,
    ) -> Result<crate::tor::TorStatus, SpectraBridgeError> {
        let _writer = self.state_writer.lock().await;
        self.bound_database().await?;
        let settings = self.wallet_state.read().await.settings.clone();
        *self.transport_cache_dir.lock() = Some(cache_dir);
        self.reconcile_transport(&settings, false);
        Ok(crate::tor::tor_status())
    }

    pub async fn reconnect_tor(&self) -> crate::tor::TorStatus {
        let _writer = self.state_writer.lock().await;
        let settings = self.wallet_state.read().await.settings.clone();
        self.reconcile_transport(&settings, true);
        crate::tor::tor_status()
    }

    /// Short-lived clients must await bootstrap before their first network request.
    pub async fn await_network_ready(&self) -> Result<(), SpectraBridgeError> {
        tokio::time::timeout(std::time::Duration::from_secs(120), async {
            loop {
                match crate::tor::tor_status() {
                    crate::tor::TorStatus::Bootstrapping { .. } => {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await
                    }
                    crate::tor::TorStatus::Error { message } => return Err(message.into()),
                    _ => return Ok(()),
                }
            }
        })
        .await
        .map_err(|_| SpectraBridgeError::from("Tor bootstrap timed out"))?
    }
}

impl WalletService {
    pub(super) fn reconcile_transport(
        &self,
        settings: &crate::store::state::AppSettings,
        restart: bool,
    ) {
        if let Some(dir) = self.transport_cache_dir.lock().as_ref() {
            crate::tor::reconcile(settings, dir, restart);
        } else {
            crate::tor::apply_policy(settings.tor_enabled, settings.tor_kill_switch);
        }
    }
}

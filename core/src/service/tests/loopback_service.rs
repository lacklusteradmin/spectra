//! A wallet service opened as a front end opens one, over a throwaway
//! database and an in-memory secret store, whose networks read only the
//! loopback nodes a test names.

use super::*;
use crate::derivation::import::WalletImportCommit;
use crate::store::secret_backends::InMemorySecretStore;

/// The service and the directory its database lives in, removed on drop.
pub(crate) struct OpenService {
    pub service: Arc<WalletService>,
    directory: std::path::PathBuf,
}

impl Drop for OpenService {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl std::ops::Deref for OpenService {
    type Target = WalletService;
    fn deref(&self) -> &WalletService {
        &self.service
    }
}

/// A catalog service, as the CLI and the app construct one, with its state
/// opened.
pub(crate) async fn open() -> OpenService {
    let directory = std::env::temp_dir().join(crate::store::new_event_id());
    std::fs::create_dir_all(&directory).unwrap();
    let service = WalletService::new_catalog().unwrap();
    service.set_secret_store(Arc::new(InMemorySecretStore::new()));
    service
        .open_state(directory.join("state.db").to_string_lossy().into_owned())
        .await
        .unwrap();
    OpenService { service, directory }
}

impl OpenService {
    /// Import `commit`'s one wallet; its id.
    pub(crate) async fn import(&self, commit: WalletImportCommit) -> String {
        self.import_wallets(commit).await.unwrap().wallets[0]
            .id
            .clone()
    }

    /// The wallet's address on `chain`.
    pub(crate) async fn address(&self, wallet: &str, chain: Chain) -> String {
        self.app_state()
            .await
            .wallets
            .iter()
            .find(|w| w.id == wallet)
            .and_then(|w| w.address_on(chain))
            .unwrap()
            .to_string()
    }

    /// `url`, speaking `api` with `capabilities`, among `chain`'s endpoints,
    /// which are the user's own alone: no catalog endpoint is contacted.
    pub(crate) async fn use_endpoint(
        &self,
        chain: Chain,
        api: crate::EndpointApi,
        capabilities: &[EndpointCapability],
        url: &str,
    ) {
        for update in [
            crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                capabilities: capabilities.to_vec(),
                chain_id: chain,
                api: api.as_str().into(),
                endpoint: url.into(),
            },
            crate::store::state::AppSettingUpdate::CustomEndpointsOnly {
                chain_id: chain,
                value: true,
            },
        ] {
            self.apply_state_command(StateCommand::SetAppSetting { update })
                .await
                .unwrap();
        }
    }
}

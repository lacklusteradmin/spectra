use serde::{Deserialize, Serialize};

use super::state::CoreAppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretMaterialDescriptor {
    pub wallet_id: String,
    pub has_seed_phrase: bool,
    pub has_password: bool,
    pub secure_store_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAppSnapshot {
    pub schema_version: u32,
    pub app_state: CoreAppState,
    pub secrets: Vec<SecretMaterialDescriptor>,
}

pub trait SecretStore: Send + Sync {
    fn store_seed_phrase(&self, wallet_id: &str, seed_phrase: &str) -> Result<(), String>;
    fn load_seed_phrase(&self, wallet_id: &str) -> Result<Option<String>, String>;
    fn delete_wallet_secret(&self, wallet_id: &str) -> Result<(), String>;
}

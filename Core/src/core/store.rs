use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::state::CoreAppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretMaterialDescriptor {
    pub wallet_id: String,
    pub secret_kind: String,
    pub has_seed_phrase: bool,
    pub has_private_key: bool,
    pub has_password: bool,
    pub has_signing_material: bool,
    pub seed_phrase_store_key: String,
    pub password_store_key: String,
    pub private_key_store_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAppSnapshot {
    pub schema_version: u32,
    pub app_state: CoreAppState,
    pub secrets: Vec<SecretMaterialDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletSecretObservation {
    pub wallet_id: String,
    pub secret_kind: Option<String>,
    pub has_seed_phrase: bool,
    pub has_private_key: bool,
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAppSnapshotRequest {
    pub app_state_json: String,
    pub secret_observations: Vec<WalletSecretObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletSecretIndex {
    pub descriptors: Vec<SecretMaterialDescriptor>,
    pub signing_material_wallet_ids: Vec<String>,
    pub private_key_backed_wallet_ids: Vec<String>,
    pub password_protected_wallet_ids: Vec<String>,
}

pub trait SecretStore: Send + Sync {
    fn store_seed_phrase(&self, wallet_id: &str, seed_phrase: &str) -> Result<(), String>;
    fn load_seed_phrase(&self, wallet_id: &str) -> Result<Option<String>, String>;
    fn delete_wallet_secret(&self, wallet_id: &str) -> Result<(), String>;
}

pub fn build_persisted_snapshot(
    request: PersistedAppSnapshotRequest,
) -> Result<PersistedAppSnapshot, String> {
    let app_state =
        serde_json::from_str::<CoreAppState>(&request.app_state_json).map_err(display_error)?;
    let observations_by_wallet_id = request
        .secret_observations
        .into_iter()
        .map(|observation| (observation.wallet_id.clone(), observation))
        .collect::<BTreeMap<_, _>>();

    let secrets = app_state
        .wallets
        .iter()
        .map(|wallet| {
            secret_descriptor_for_wallet(
                wallet.id.as_str(),
                observations_by_wallet_id.get(&wallet.id),
            )
        })
        .collect::<Vec<_>>();

    Ok(PersistedAppSnapshot {
        schema_version: 1,
        app_state,
        secrets,
    })
}

pub fn persisted_snapshot_from_json(json: &str) -> Result<PersistedAppSnapshot, String> {
    if let Ok(snapshot) = serde_json::from_str::<PersistedAppSnapshot>(json) {
        return Ok(snapshot);
    }

    let app_state = serde_json::from_str::<CoreAppState>(json).map_err(display_error)?;
    Ok(PersistedAppSnapshot {
        schema_version: 1,
        app_state,
        secrets: Vec::new(),
    })
}

pub fn wallet_secret_index(snapshot: &PersistedAppSnapshot) -> WalletSecretIndex {
    WalletSecretIndex {
        descriptors: snapshot.secrets.clone(),
        signing_material_wallet_ids: snapshot
            .secrets
            .iter()
            .filter(|descriptor| descriptor.has_signing_material)
            .map(|descriptor| descriptor.wallet_id.clone())
            .collect(),
        private_key_backed_wallet_ids: snapshot
            .secrets
            .iter()
            .filter(|descriptor| descriptor.has_private_key)
            .map(|descriptor| descriptor.wallet_id.clone())
            .collect(),
        password_protected_wallet_ids: snapshot
            .secrets
            .iter()
            .filter(|descriptor| descriptor.has_password)
            .map(|descriptor| descriptor.wallet_id.clone())
            .collect(),
    }
}

fn secret_descriptor_for_wallet(
    wallet_id: &str,
    observation: Option<&WalletSecretObservation>,
) -> SecretMaterialDescriptor {
    let has_seed_phrase = observation
        .map(|observation| observation.has_seed_phrase)
        .unwrap_or(false);
    let has_private_key = observation
        .map(|observation| observation.has_private_key)
        .unwrap_or(false);
    let has_password = observation
        .map(|observation| observation.has_password)
        .unwrap_or(false);
    let secret_kind = observation
        .and_then(|observation| observation.secret_kind.clone())
        .unwrap_or_else(|| {
            if has_private_key {
                "privateKey".to_string()
            } else if has_seed_phrase {
                "seedPhrase".to_string()
            } else {
                "watchOnly".to_string()
            }
        });

    SecretMaterialDescriptor {
        wallet_id: wallet_id.to_string(),
        secret_kind,
        has_seed_phrase,
        has_private_key,
        has_password,
        has_signing_material: has_seed_phrase || has_private_key,
        seed_phrase_store_key: format!("wallet.seed.{wallet_id}"),
        password_store_key: format!("wallet.seed.password.{wallet_id}"),
        private_key_store_key: format!("wallet.privatekey.{wallet_id}"),
    }
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        build_persisted_snapshot, persisted_snapshot_from_json, wallet_secret_index,
        PersistedAppSnapshot, PersistedAppSnapshotRequest, WalletSecretObservation,
    };
    use crate::core::state::CoreAppState;
    use std::collections::BTreeMap;

    #[test]
    fn builds_secret_catalog_for_persisted_snapshot() {
        let request = PersistedAppSnapshotRequest {
            app_state_json: serde_json::to_string(&CoreAppState::default()).unwrap(),
            secret_observations: vec![WalletSecretObservation {
                wallet_id: "wallet-1".to_string(),
                secret_kind: Some("seedPhrase".to_string()),
                has_seed_phrase: true,
                has_private_key: false,
                has_password: true,
            }],
        };

        let mut app_state = CoreAppState::default();
        app_state.wallets.push(crate::core::state::WalletSummary {
            id: "wallet-1".to_string(),
            name: "Main".to_string(),
            is_watch_only: false,
            selected_chain: Some("Bitcoin".to_string()),
            include_in_portfolio_total: true,
            bitcoin_network_mode: "mainnet".to_string(),
            dogecoin_network_mode: "mainnet".to_string(),
            bitcoin_xpub: None,
            derivation_preset: "standard".to_string(),
            derivation_paths: BTreeMap::new(),
            holdings: Vec::new(),
            addresses: Vec::new(),
        });

        let request = PersistedAppSnapshotRequest {
            app_state_json: serde_json::to_string(&app_state).unwrap(),
            secret_observations: request.secret_observations,
        };
        let snapshot = build_persisted_snapshot(request).unwrap();

        assert_eq!(snapshot.secrets.len(), 1);
        assert_eq!(snapshot.secrets[0].wallet_id, "wallet-1");
        assert!(snapshot.secrets[0].has_signing_material);
        assert_eq!(
            snapshot.secrets[0].password_store_key,
            "wallet.seed.password.wallet-1"
        );
    }

    #[test]
    fn computes_wallet_secret_index_from_snapshot() {
        let snapshot = PersistedAppSnapshot {
            schema_version: 1,
            app_state: CoreAppState::default(),
            secrets: vec![
                super::SecretMaterialDescriptor {
                    wallet_id: "seed-wallet".to_string(),
                    secret_kind: "seedPhrase".to_string(),
                    has_seed_phrase: true,
                    has_private_key: false,
                    has_password: true,
                    has_signing_material: true,
                    seed_phrase_store_key: "wallet.seed.seed-wallet".to_string(),
                    password_store_key: "wallet.seed.password.seed-wallet".to_string(),
                    private_key_store_key: "wallet.privatekey.seed-wallet".to_string(),
                },
                super::SecretMaterialDescriptor {
                    wallet_id: "watch-wallet".to_string(),
                    secret_kind: "watchOnly".to_string(),
                    has_seed_phrase: false,
                    has_private_key: false,
                    has_password: false,
                    has_signing_material: false,
                    seed_phrase_store_key: "wallet.seed.watch-wallet".to_string(),
                    password_store_key: "wallet.seed.password.watch-wallet".to_string(),
                    private_key_store_key: "wallet.privatekey.watch-wallet".to_string(),
                },
            ],
        };

        let index = wallet_secret_index(&snapshot);
        assert_eq!(
            index.signing_material_wallet_ids,
            vec!["seed-wallet".to_string()]
        );
        assert_eq!(
            index.password_protected_wallet_ids,
            vec!["seed-wallet".to_string()]
        );
        assert!(index.private_key_backed_wallet_ids.is_empty());
    }

    #[test]
    fn upgrades_core_state_payload_into_empty_secret_snapshot() {
        let json = serde_json::to_string(&CoreAppState::default()).unwrap();
        let snapshot = persisted_snapshot_from_json(&json).unwrap();
        assert_eq!(snapshot.schema_version, 1);
        assert!(snapshot.secrets.is_empty());
    }
}

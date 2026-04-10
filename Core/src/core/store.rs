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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedHoldingInput {
    pub holding_index: usize,
    pub asset_identity_key: String,
    pub symbol_upper: String,
    pub amount: String,
    pub is_priced_asset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedWalletInput {
    pub wallet_id: String,
    pub include_in_portfolio_total: bool,
    pub has_signing_material: bool,
    pub is_private_key_backed: bool,
    pub holdings: Vec<StoreDerivedHoldingInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedStateRequest {
    pub wallets: Vec<StoreDerivedWalletInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletHoldingRef {
    pub wallet_id: String,
    pub holding_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupedPortfolioHolding {
    pub asset_identity_key: String,
    pub wallet_id: String,
    pub holding_index: usize,
    pub total_amount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedStatePlan {
    pub included_portfolio_holding_refs: Vec<WalletHoldingRef>,
    pub unique_price_request_holding_refs: Vec<WalletHoldingRef>,
    pub grouped_portfolio: Vec<GroupedPortfolioHolding>,
    pub signing_material_wallet_ids: Vec<String>,
    pub private_key_backed_wallet_ids: Vec<String>,
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

pub fn plan_store_derived_state(request: StoreDerivedStateRequest) -> StoreDerivedStatePlan {
    let mut included_portfolio_holding_refs = Vec::new();
    let mut unique_price_request_holding_refs = Vec::new();
    let mut signing_material_wallet_ids = Vec::new();
    let mut private_key_backed_wallet_ids = Vec::new();

    let mut seen_price_request_keys = std::collections::BTreeSet::<String>::new();
    let mut grouped_portfolio_totals = BTreeMap::<String, f64>::new();
    let mut grouped_portfolio_order = Vec::<String>::new();
    let mut grouped_portfolio_representatives = BTreeMap::<String, WalletHoldingRef>::new();

    for wallet in request.wallets {
        if wallet.has_signing_material {
            signing_material_wallet_ids.push(wallet.wallet_id.clone());
        }
        if wallet.is_private_key_backed {
            private_key_backed_wallet_ids.push(wallet.wallet_id.clone());
        }

        for holding in wallet.holdings {
            let holding_ref = WalletHoldingRef {
                wallet_id: wallet.wallet_id.clone(),
                holding_index: holding.holding_index,
            };

            if holding.is_priced_asset
                && seen_price_request_keys.insert(holding.asset_identity_key.clone())
            {
                unique_price_request_holding_refs.push(holding_ref.clone());
            }

            if wallet.include_in_portfolio_total {
                included_portfolio_holding_refs.push(holding_ref.clone());

                let amount = holding.amount.parse::<f64>().unwrap_or(0.0);
                if !grouped_portfolio_totals.contains_key(&holding.asset_identity_key) {
                    grouped_portfolio_order.push(holding.asset_identity_key.clone());
                    grouped_portfolio_representatives
                        .insert(holding.asset_identity_key.clone(), holding_ref);
                }
                *grouped_portfolio_totals
                    .entry(holding.asset_identity_key)
                    .or_default() += amount;
            }
        }
    }

    let grouped_portfolio = grouped_portfolio_order
        .into_iter()
        .filter_map(|asset_identity_key| {
            let representative = grouped_portfolio_representatives.get(&asset_identity_key)?;
            Some(GroupedPortfolioHolding {
                total_amount: grouped_portfolio_totals
                    .get(&asset_identity_key)
                    .copied()
                    .unwrap_or_default()
                    .to_string(),
                asset_identity_key,
                wallet_id: representative.wallet_id.clone(),
                holding_index: representative.holding_index,
            })
        })
        .collect();

    StoreDerivedStatePlan {
        included_portfolio_holding_refs,
        unique_price_request_holding_refs,
        grouped_portfolio,
        signing_material_wallet_ids,
        private_key_backed_wallet_ids,
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
        build_persisted_snapshot, persisted_snapshot_from_json, plan_store_derived_state,
        wallet_secret_index, PersistedAppSnapshot, PersistedAppSnapshotRequest,
        StoreDerivedHoldingInput, StoreDerivedStateRequest, StoreDerivedWalletInput,
        WalletSecretObservation,
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

    #[test]
    fn plans_store_derived_state_with_stable_grouping() {
        let plan = plan_store_derived_state(StoreDerivedStateRequest {
            wallets: vec![
                StoreDerivedWalletInput {
                    wallet_id: "wallet-1".to_string(),
                    include_in_portfolio_total: true,
                    has_signing_material: true,
                    is_private_key_backed: false,
                    holdings: vec![
                        StoreDerivedHoldingInput {
                            holding_index: 0,
                            asset_identity_key: "Bitcoin|BTC".to_string(),
                            symbol_upper: "BTC".to_string(),
                            amount: "1.25".to_string(),
                            is_priced_asset: true,
                        },
                        StoreDerivedHoldingInput {
                            holding_index: 1,
                            asset_identity_key: "Ethereum|USDC".to_string(),
                            symbol_upper: "USDC".to_string(),
                            amount: "50".to_string(),
                            is_priced_asset: true,
                        },
                    ],
                },
                StoreDerivedWalletInput {
                    wallet_id: "wallet-2".to_string(),
                    include_in_portfolio_total: true,
                    has_signing_material: false,
                    is_private_key_backed: true,
                    holdings: vec![StoreDerivedHoldingInput {
                        holding_index: 0,
                        asset_identity_key: "Bitcoin|BTC".to_string(),
                        symbol_upper: "BTC".to_string(),
                        amount: "0.75".to_string(),
                        is_priced_asset: true,
                    }],
                },
            ],
        });

        assert_eq!(plan.included_portfolio_holding_refs.len(), 3);
        assert_eq!(plan.unique_price_request_holding_refs.len(), 2);
        assert_eq!(
            plan.signing_material_wallet_ids,
            vec!["wallet-1".to_string()]
        );
        assert_eq!(
            plan.private_key_backed_wallet_ids,
            vec!["wallet-2".to_string()]
        );
        assert_eq!(plan.grouped_portfolio.len(), 2);
        assert_eq!(plan.grouped_portfolio[0].asset_identity_key, "Bitcoin|BTC");
        assert_eq!(plan.grouped_portfolio[0].total_amount, "2");
    }
}

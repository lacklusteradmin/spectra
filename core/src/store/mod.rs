pub mod chain_aliases;
pub mod password_verifier;
pub mod persistence_models;
pub mod secret_store;
pub mod seed_envelope;
pub mod state;
// token_helpers moved to root tokens.rs
// wallet_core moved to send/preview_types.rs
pub mod wallet_db;
pub mod wallet_domain;

pub use chain_aliases::{
    plan_canonical_chain_component, plan_icon_identifier, plan_normalized_icon_identifier,
};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use self::state::CoreAppState;
use crate::derivation::addressing::{validate_address, AddressValidationRequest};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAppSnapshot {
    pub schema_version: u32,
    pub app_state: CoreAppState,
    pub secrets: Vec<SecretMaterialDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSecretIndex {
    pub descriptors: Vec<SecretMaterialDescriptor>,
    pub signing_material_wallet_ids: Vec<String>,
    pub private_key_backed_wallet_ids: Vec<String>,
    pub password_protected_wallet_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedHoldingInput {
    pub holding_index: u64,
    pub asset_identity_key: String,
    pub symbol_upper: String,
    pub amount: String,
    pub is_priced_asset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedWalletInput {
    pub wallet_id: String,
    pub include_in_portfolio_total: bool,
    pub has_signing_material: bool,
    pub is_private_key_backed: bool,
    pub holdings: Vec<StoreDerivedHoldingInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedStateRequest {
    pub wallets: Vec<StoreDerivedWalletInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletHoldingRef {
    pub wallet_id: String,
    pub holding_index: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct GroupedPortfolioHolding {
    pub asset_identity_key: String,
    pub wallet_id: String,
    pub holding_index: u64,
    pub total_amount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StoreDerivedStatePlan {
    pub included_portfolio_holding_refs: Vec<WalletHoldingRef>,
    pub unique_price_request_holding_refs: Vec<WalletHoldingRef>,
    pub grouped_portfolio: Vec<GroupedPortfolioHolding>,
    pub signing_material_wallet_ids: Vec<String>,
    pub private_key_backed_wallet_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct OwnedAddressAggregationRequest {
    pub candidate_addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveSelectionHoldingInput {
    pub holding_index: u64,
    pub chain_name: String,
    pub has_contract_address: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveSelectionRequest {
    pub receive_chain_name: String,
    pub available_receive_chains: Vec<String>,
    pub available_receive_holdings: Vec<ReceiveSelectionHoldingInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveSelectionPlan {
    pub resolved_chain_name: String,
    pub selected_receive_holding_index: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PendingSelfSendConfirmationInput {
    pub wallet_id: String,
    pub chain_name: String,
    pub symbol: String,
    pub destination_address_lowercased: String,
    pub amount: f64,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SelfSendConfirmationRequest {
    pub pending_confirmation: Option<PendingSelfSendConfirmationInput>,
    pub wallet_id: String,
    pub chain_name: String,
    pub symbol: String,
    pub destination_address: String,
    pub amount: f64,
    pub now_unix: f64,
    pub window_seconds: f64,
    pub owned_addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SelfSendConfirmationPlan {
    pub requires_confirmation: bool,
    pub consume_existing_confirmation: bool,
    pub clear_pending_confirmation: bool,
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

pub fn build_persisted_snapshot_typed(
    app_state: CoreAppState,
    secret_observations: Vec<WalletSecretObservation>,
) -> PersistedAppSnapshot {
    let observations_by_wallet_id = secret_observations
        .into_iter()
        .map(|observation| (observation.wallet_id.clone(), observation))
        .collect::<HashMap<_, _>>();

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

    PersistedAppSnapshot {
        schema_version: 1,
        app_state,
        secrets,
    }
}

pub fn wallet_secret_index_from_observations(
    app_state: CoreAppState,
    secret_observations: Vec<WalletSecretObservation>,
) -> WalletSecretIndex {
    wallet_secret_index(&build_persisted_snapshot_typed(app_state, secret_observations))
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

pub fn aggregate_owned_addresses(request: OwnedAddressAggregationRequest) -> Vec<String> {
    let mut ordered = Vec::new();
    let mut seen = std::collections::BTreeSet::<String>::new();

    for candidate in request.candidate_addresses {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_lowercase();
        if seen.insert(normalized) {
            ordered.push(trimmed.to_string());
        }
    }

    ordered
}

pub fn plan_receive_selection(request: ReceiveSelectionRequest) -> ReceiveSelectionPlan {
    let resolved_chain_name = if request
        .available_receive_chains
        .iter()
        .any(|chain| chain == &request.receive_chain_name)
    {
        request.receive_chain_name
    } else {
        request
            .available_receive_chains
            .first()
            .cloned()
            .unwrap_or_default()
    };

    let mut first_matching = None;
    let mut selected_receive_holding_index = None;
    for holding in request.available_receive_holdings {
        if holding.chain_name != resolved_chain_name {
            continue;
        }
        if first_matching.is_none() {
            first_matching = Some(holding.holding_index);
        }
        if !holding.has_contract_address {
            selected_receive_holding_index = Some(holding.holding_index);
            break;
        }
    }

    ReceiveSelectionPlan {
        resolved_chain_name,
        selected_receive_holding_index: selected_receive_holding_index.or(first_matching),
    }
}

pub fn plan_self_send_confirmation(
    request: SelfSendConfirmationRequest,
) -> SelfSendConfirmationPlan {
    let destination = request.destination_address.trim().to_lowercase();
    let owned_addresses = request
        .owned_addresses
        .into_iter()
        .map(|address| address.trim().to_lowercase())
        .collect::<std::collections::BTreeSet<_>>();

    if !owned_addresses.contains(&destination) {
        return SelfSendConfirmationPlan {
            requires_confirmation: false,
            consume_existing_confirmation: false,
            clear_pending_confirmation: false,
        };
    }

    let Some(pending) = request.pending_confirmation else {
        return SelfSendConfirmationPlan {
            requires_confirmation: true,
            consume_existing_confirmation: false,
            clear_pending_confirmation: false,
        };
    };

    let is_expired = request.now_unix - pending.created_at_unix > request.window_seconds;
    if is_expired {
        return SelfSendConfirmationPlan {
            requires_confirmation: true,
            consume_existing_confirmation: false,
            clear_pending_confirmation: true,
        };
    }

    let same_wallet = pending.wallet_id == request.wallet_id;
    let same_chain = pending.chain_name == request.chain_name;
    let same_symbol = pending.symbol == request.symbol;
    let same_destination = pending.destination_address_lowercased == destination;
    let same_amount = (pending.amount - request.amount).abs() < 0.00000001;

    if same_wallet && same_chain && same_symbol && same_destination && same_amount {
        return SelfSendConfirmationPlan {
            requires_confirmation: false,
            consume_existing_confirmation: true,
            clear_pending_confirmation: true,
        };
    }

    SelfSendConfirmationPlan {
        requires_confirmation: true,
        consume_existing_confirmation: false,
        clear_pending_confirmation: true,
    }
}

pub fn plan_dashboard_supported_token_entries(
    entries: Vec<wallet_domain::CoreTokenPreferenceEntry>,
) -> Vec<wallet_domain::CoreTokenPreferenceEntry> {
    fn chain_name(chain: wallet_domain::CoreTokenTrackingChain) -> &'static str {
        use wallet_domain::CoreTokenTrackingChain::*;
        match chain {
            Ethereum => "Ethereum",
            Arbitrum => "Arbitrum",
            Optimism => "Optimism",
            Bnb => "BNB Chain",
            Avalanche => "Avalanche",
            Hyperliquid => "Hyperliquid",
            Polygon => "Polygon",
            Base => "Base",
            Linea => "Linea",
            Scroll => "Scroll",
            Blast => "Blast",
            Mantle => "Mantle",
            Solana => "Solana",
            Sui => "Sui",
            Aptos => "Aptos",
            Ton => "TON",
            Near => "NEAR",
            Tron => "Tron",
        }
    }

    let mut filtered: Vec<wallet_domain::CoreTokenPreferenceEntry> = entries
        .into_iter()
        .filter(|entry| !entry.contract_address.is_empty())
        .collect();
    filtered.sort_by(|lhs, rhs| {
        chain_name(lhs.chain)
            .to_lowercase()
            .cmp(&chain_name(rhs.chain).to_lowercase())
    });
    let mut seen_keys = std::collections::BTreeSet::<String>::new();
    filtered
        .into_iter()
        .filter(|entry| {
            let key = format!(
                "{}|{}",
                chain_name(entry.chain).to_lowercase(),
                entry.contract_address.to_lowercase()
            );
            seen_keys.insert(key)
        })
        .collect()
}

/// Normalize a token contract address for identity matching. Mirrors the
/// Swift `normalizedTrackedTokenIdentifier(for:contractAddress:)` dispatch.
fn normalize_tracked_token_identifier(
    chain: wallet_domain::CoreTokenTrackingChain,
    contract_address: &str,
) -> String {
    use wallet_domain::CoreTokenTrackingChain::*;
    let trimmed = contract_address.trim();
    match chain {
        Ethereum | Arbitrum | Optimism | Bnb | Avalanche | Hyperliquid | Polygon | Base | Linea
        | Scroll | Blast | Mantle => trimmed.to_lowercase(),
        Aptos => crate::tokens::normalize_aptos_token_identifier(
            trimmed.to_string(),
        ),
        Sui => crate::tokens::normalize_sui_token_identifier(
            trimmed.to_string(),
        ),
        Ton => trimmed.to_string(),
        _ => trimmed.to_lowercase(),
    }
}

/// Merge built-in token registry entries with persisted user preferences:
/// copies `is_enabled` + `display_decimals` from matching persisted built-ins,
/// appends all non-built-in (custom) persisted entries, and returns the list
/// sorted by (chain-label, built-in first, symbol).
pub fn plan_merge_built_in_token_preferences(
    built_ins: Vec<wallet_domain::CoreTokenPreferenceEntry>,
    persisted: Vec<wallet_domain::CoreTokenPreferenceEntry>,
) -> Vec<wallet_domain::CoreTokenPreferenceEntry> {
    fn chain_label(chain: wallet_domain::CoreTokenTrackingChain) -> &'static str {
        use wallet_domain::CoreTokenTrackingChain::*;
        match chain {
            Ethereum => "Ethereum",
            Arbitrum => "Arbitrum",
            Optimism => "Optimism",
            Bnb => "BNB Chain",
            Avalanche => "Avalanche",
            Hyperliquid => "Hyperliquid",
            Polygon => "Polygon",
            Base => "Base",
            Linea => "Linea",
            Scroll => "Scroll",
            Blast => "Blast",
            Mantle => "Mantle",
            Solana => "Solana",
            Sui => "Sui",
            Aptos => "Aptos",
            Ton => "TON",
            Near => "NEAR",
            Tron => "Tron",
        }
    }
    let mut merged: Vec<wallet_domain::CoreTokenPreferenceEntry> = Vec::new();
    for built_in in built_ins.into_iter() {
        let built_in_key =
            normalize_tracked_token_identifier(built_in.chain, &built_in.contract_address);
        let existing = persisted.iter().find(|entry| {
            entry.is_built_in
                && entry.chain == built_in.chain
                && normalize_tracked_token_identifier(entry.chain, &entry.contract_address)
                    == built_in_key
        });
        let mut updated = built_in;
        if let Some(existing) = existing {
            updated.is_enabled = existing.is_enabled;
            updated.display_decimals = existing.display_decimals;
        }
        merged.push(updated);
    }
    merged.extend(persisted.into_iter().filter(|entry| !entry.is_built_in));
    merged.sort_by(|lhs, rhs| {
        let lhs_chain = chain_label(lhs.chain);
        let rhs_chain = chain_label(rhs.chain);
        lhs_chain
            .cmp(rhs_chain)
            .then_with(|| rhs.is_built_in.cmp(&lhs.is_built_in))
            .then_with(|| lhs.symbol.cmp(&rhs.symbol))
    });
    merged
}

pub fn plan_priced_chain(
    chain_name: String,
    bitcoin_network_mode_raw: String,
    ethereum_network_mode_raw: String,
) -> bool {
    match chain_name.as_str() {
        "Bitcoin" => bitcoin_network_mode_raw == "mainnet",
        "Ethereum" => ethereum_network_mode_raw == "mainnet",
        _ => true,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletChainInput {
    pub wallet_id: String,
    pub selected_chain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionActivityInput {
    pub id: String,
    pub wallet_id: Option<String>,
    pub chain_name: String,
}

pub fn plan_active_wallet_transaction_ids(
    transactions: Vec<TransactionActivityInput>,
    wallets: Vec<WalletChainInput>,
) -> Vec<String> {
    let wallet_chain: HashMap<String, String> = wallets
        .into_iter()
        .map(|wallet| (wallet.wallet_id, wallet.selected_chain))
        .collect();
    transactions
        .into_iter()
        .filter_map(|transaction| {
            let wallet_id = transaction.wallet_id.as_ref()?;
            let chain = wallet_chain.get(wallet_id)?;
            if chain == &transaction.chain_name {
                Some(transaction.id)
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedHistorySignatureTransaction {
    pub id: String,
    pub wallet_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub chain_name: String,
    pub symbol: String,
    pub transaction_hash: Option<String>,
    pub created_at_unix: f64,
}

pub fn plan_normalized_history_signature(
    transactions: Vec<NormalizedHistorySignatureTransaction>,
    wallets: Vec<WalletChainInput>,
) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (transactions.len() as u64).hash(&mut hasher);
    for transaction in &transactions {
        transaction.id.hash(&mut hasher);
        transaction.wallet_id.hash(&mut hasher);
        transaction.kind.hash(&mut hasher);
        transaction.status.hash(&mut hasher);
        transaction.chain_name.hash(&mut hasher);
        transaction.symbol.hash(&mut hasher);
        transaction
            .transaction_hash
            .as_deref()
            .unwrap_or("")
            .hash(&mut hasher);
        transaction.created_at_unix.to_bits().hash(&mut hasher);
    }
    let wallet_chain: BTreeMap<String, String> = wallets
        .into_iter()
        .map(|wallet| (wallet.wallet_id, wallet.selected_chain))
        .collect();
    for (wallet_id, selected_chain) in &wallet_chain {
        wallet_id.hash(&mut hasher);
        selected_chain.hash(&mut hasher);
    }
    hasher.finish() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletEarliestTransactionDate {
    pub wallet_id: String,
    pub earliest_created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionEarliestInput {
    pub wallet_id: Option<String>,
    pub created_at_unix: f64,
}

pub fn plan_earliest_transaction_dates(
    transactions: Vec<TransactionEarliestInput>,
) -> Vec<WalletEarliestTransactionDate> {
    let mut earliest: BTreeMap<String, f64> = BTreeMap::new();
    for transaction in transactions {
        let Some(wallet_id) = transaction.wallet_id else {
            continue;
        };
        earliest
            .entry(wallet_id)
            .and_modify(|current| {
                if transaction.created_at_unix < *current {
                    *current = transaction.created_at_unix;
                }
            })
            .or_insert(transaction.created_at_unix);
    }
    earliest
        .into_iter()
        .map(|(wallet_id, earliest_created_at_unix)| WalletEarliestTransactionDate {
            wallet_id,
            earliest_created_at_unix,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletChainEligibilityInput {
    pub wallet_id: String,
    pub selected_chain: String,
    pub has_seed_phrase: bool,
    pub bitcoin_address: Option<String>,
    pub bitcoin_address_is_valid: bool,
    pub bitcoin_xpub: Option<String>,
    pub resolved_address_for_chain: Option<String>,
}

pub fn plan_has_wallet_for_chain(
    chain_name: String,
    wallets: Vec<WalletChainEligibilityInput>,
) -> bool {
    wallets.into_iter().any(|wallet| {
        if wallet.selected_chain != chain_name {
            return false;
        }
        if chain_name == "Bitcoin" {
            if wallet.has_seed_phrase {
                return true;
            }
            if wallet.bitcoin_address.is_some() && wallet.bitcoin_address_is_valid {
                return true;
            }
            if let Some(xpub) = wallet.bitcoin_xpub.as_ref() {
                if xpub.starts_with("xpub") || xpub.starts_with("ypub") || xpub.starts_with("zpub")
                {
                    return true;
                }
            }
            return false;
        }
        wallet.resolved_address_for_chain.is_some()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DerivedAddressPostProcess {
    None,
    Lowercase,
    Trim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreResetPlan {
    pub reset_wallets_and_secrets: bool,
    pub reset_history_and_cache: bool,
    pub reset_alerts_and_contacts: bool,
    pub reset_settings_and_endpoints: bool,
    pub reset_dashboard_customization: bool,
    pub reset_provider_state: bool,
    pub clear_network_and_transport_caches: bool,
}

pub fn plan_reset_dispatch(scopes: Vec<String>) -> CoreResetPlan {
    let has = |s: &str| scopes.iter().any(|x| x == s);
    let wallets_and_secrets = has("walletsAndSecrets");
    let history_and_cache_direct = has("historyAndCache");
    let history_and_cache = wallets_and_secrets || history_and_cache_direct;
    CoreResetPlan {
        reset_wallets_and_secrets: wallets_and_secrets,
        reset_history_and_cache: history_and_cache,
        reset_alerts_and_contacts: has("alertsAndContacts"),
        reset_settings_and_endpoints: has("settingsAndEndpoints"),
        reset_dashboard_customization: has("dashboardCustomization"),
        reset_provider_state: has("providerState"),
        clear_network_and_transport_caches: wallets_and_secrets || history_and_cache_direct,
    }
}

/// Input per price alert — ids/metadata needed to produce notifications;
/// Swift formats the user-facing text itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertEvaluationAlert {
    pub id: String,
    pub holding_key: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub target_price: f64,
    pub condition: wallet_domain::CorePriceAlertCondition,
    pub is_enabled: bool,
    pub has_triggered: bool,
}

/// Live price lookup for one holding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertEvaluationPrice {
    pub holding_key: String,
    pub live_price: f64,
}

/// Alert `has_triggered` state changes produced by the evaluator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertTriggerUpdate {
    pub id: String,
    pub has_triggered: bool,
}

/// A single firing — Swift formats the notification body using this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertNotification {
    pub id: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub target_price: f64,
    pub live_price: f64,
    pub condition: wallet_domain::CorePriceAlertCondition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertEvaluationPlan {
    pub updates: Vec<PriceAlertTriggerUpdate>,
    pub notifications: Vec<PriceAlertNotification>,
}

pub fn plan_price_alert_evaluation(
    alerts: Vec<PriceAlertEvaluationAlert>,
    prices: Vec<PriceAlertEvaluationPrice>,
) -> PriceAlertEvaluationPlan {
    let price_by_key: HashMap<String, f64> = prices
        .into_iter()
        .map(|p| (p.holding_key, p.live_price))
        .collect();
    let mut updates = Vec::new();
    let mut notifications = Vec::new();
    for alert in alerts.into_iter() {
        if !alert.is_enabled {
            continue;
        }
        let Some(live_price) = price_by_key.get(&alert.holding_key).copied() else {
            continue;
        };
        let meets_target = match alert.condition {
            wallet_domain::CorePriceAlertCondition::Above => live_price >= alert.target_price,
            wallet_domain::CorePriceAlertCondition::Below => live_price <= alert.target_price,
        };
        if meets_target && !alert.has_triggered {
            updates.push(PriceAlertTriggerUpdate {
                id: alert.id.clone(),
                has_triggered: true,
            });
            notifications.push(PriceAlertNotification {
                id: alert.id,
                asset_name: alert.asset_name,
                symbol: alert.symbol,
                chain_name: alert.chain_name,
                target_price: alert.target_price,
                live_price,
                condition: alert.condition,
            });
        } else if !meets_target && alert.has_triggered {
            updates.push(PriceAlertTriggerUpdate {
                id: alert.id,
                has_triggered: false,
            });
        }
    }
    PriceAlertEvaluationPlan { updates, notifications }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DashboardRebuildDecisionRequest {
    pub old_prices: Vec<PriceAlertEvaluationPrice>,
    pub new_prices: Vec<PriceAlertEvaluationPrice>,
    pub cached_relevant_price_keys: Vec<String>,
    pub pinned_prototype_keys: Vec<String>,
    pub selected_main_tab_is_home: bool,
}

/// Decide whether a live-price update should trigger a dashboard rebuild.
/// Mirrors Swift `shouldRebuildDashboardForLivePriceChange(from:to:)`.
pub fn plan_dashboard_rebuild_for_live_price_change(
    request: DashboardRebuildDecisionRequest,
) -> bool {
    let old: HashMap<String, f64> = request
        .old_prices
        .into_iter()
        .map(|p| (p.holding_key, p.live_price))
        .collect();
    let new: HashMap<String, f64> = request
        .new_prices
        .into_iter()
        .map(|p| (p.holding_key, p.live_price))
        .collect();
    if old == new {
        return false;
    }
    if request.cached_relevant_price_keys.is_empty() {
        return true;
    }
    let changed_relevant = request
        .cached_relevant_price_keys
        .iter()
        .any(|key| old.get(key) != new.get(key));
    if changed_relevant {
        return true;
    }
    if request.selected_main_tab_is_home {
        return request
            .pinned_prototype_keys
            .iter()
            .any(|key| old.get(key) != new.get(key));
    }
    false
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
pub enum EthereumCustomFeeValidationCode {
    InvalidMaxFee,
    InvalidPriorityFee,
    MaxBelowPriority,
}

pub fn plan_ethereum_custom_fee_validation(
    use_custom_fees: bool,
    is_ethereum_chain: bool,
    max_fee_gwei_raw: String,
    priority_fee_gwei_raw: String,
) -> Option<EthereumCustomFeeValidationCode> {
    if !use_custom_fees || !is_ethereum_chain {
        return None;
    }
    let max_trimmed = max_fee_gwei_raw.trim();
    let priority_trimmed = priority_fee_gwei_raw.trim();
    let Some(max_fee) = max_trimmed.parse::<f64>().ok().filter(|v| *v > 0.0) else {
        return Some(EthereumCustomFeeValidationCode::InvalidMaxFee);
    };
    let Some(priority_fee) = priority_trimmed.parse::<f64>().ok().filter(|v| *v > 0.0) else {
        return Some(EthereumCustomFeeValidationCode::InvalidPriorityFee);
    };
    if max_fee < priority_fee {
        return Some(EthereumCustomFeeValidationCode::MaxBelowPriority);
    }
    None
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
pub enum EthereumManualNonceValidationCode {
    Empty,
    NotNonNegativeInteger,
    TooLarge,
}

pub fn plan_ethereum_manual_nonce_validation(
    manual_nonce_enabled: bool,
    nonce_raw: String,
) -> Option<EthereumManualNonceValidationCode> {
    if !manual_nonce_enabled {
        return None;
    }
    let trimmed = nonce_raw.trim();
    if trimmed.is_empty() {
        return Some(EthereumManualNonceValidationCode::Empty);
    }
    let Ok(parsed) = trimmed.parse::<i64>() else {
        return Some(EthereumManualNonceValidationCode::NotNonNegativeInteger);
    };
    if parsed < 0 {
        return Some(EthereumManualNonceValidationCode::NotNonNegativeInteger);
    }
    if parsed > i32::MAX as i64 {
        return Some(EthereumManualNonceValidationCode::TooLarge);
    }
    None
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ChainOperationalEventLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainOperationalEventRecord {
    pub id: String,
    pub timestamp_unix: f64,
    pub chain_name: String,
    pub level: ChainOperationalEventLevel,
    pub message: String,
    pub transaction_hash: Option<String>,
}

/// Prepend `new_event` to `existing_events` and cap the list to 200. Matches
/// the Swift ring-buffer semantics inside `appendChainOperationalEvent`.
pub fn plan_append_chain_operational_event(
    existing_events: Vec<ChainOperationalEventRecord>,
    new_event: ChainOperationalEventRecord,
) -> Vec<ChainOperationalEventRecord> {
    const RING_BUFFER_CAP: usize = 200;
    let mut events = Vec::with_capacity((existing_events.len() + 1).min(RING_BUFFER_CAP));
    events.push(new_event);
    events.extend(
        existing_events
            .into_iter()
            .take(RING_BUFFER_CAP.saturating_sub(1)),
    );
    events
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRecipientPreflightRequest {
    pub chain_name: String,
    pub holding_symbol: String,
    pub token_symbol: Option<String>,
    pub recipient_has_code: Option<bool>,
    pub token_has_code: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRecipientPreflightWarning {
    pub code: String,
    pub chain_name: Option<String>,
    pub symbol: Option<String>,
    pub token_symbol: Option<String>,
}

/// Build warning codes for an EVM send's recipient + token contract checks.
/// Swift localizes the codes into user-facing strings.
pub fn plan_evm_recipient_preflight_warnings(
    request: EvmRecipientPreflightRequest,
) -> Vec<EvmRecipientPreflightWarning> {
    let mut warnings = Vec::new();
    match request.recipient_has_code {
        Some(true) => warnings.push(EvmRecipientPreflightWarning {
            code: "recipient_is_contract".to_string(),
            chain_name: Some(request.chain_name.clone()),
            symbol: Some(request.holding_symbol.clone()),
            token_symbol: None,
        }),
        Some(false) => {}
        None => warnings.push(EvmRecipientPreflightWarning {
            code: "recipient_code_unknown".to_string(),
            chain_name: Some(request.chain_name.clone()),
            symbol: None,
            token_symbol: None,
        }),
    }
    if let Some(token_symbol) = request.token_symbol {
        match request.token_has_code {
            Some(false) => warnings.push(EvmRecipientPreflightWarning {
                code: "token_contract_missing".to_string(),
                chain_name: Some(request.chain_name.clone()),
                symbol: None,
                token_symbol: Some(token_symbol),
            }),
            None => warnings.push(EvmRecipientPreflightWarning {
                code: "token_code_unknown".to_string(),
                chain_name: Some(request.chain_name.clone()),
                symbol: None,
                token_symbol: Some(token_symbol),
            }),
            Some(true) => {}
        }
    }
    warnings
}

// ─── Transaction status polling state machine (J+K+L) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatusTrackerState {
    pub last_checked_at_unix: Option<f64>,
    pub next_check_at_unix: f64,
    pub consecutive_failures: u32,
    pub reached_finality: bool,
}

impl TransactionStatusTrackerState {
    fn initial(now_unix: f64) -> Self {
        Self {
            last_checked_at_unix: None,
            next_check_at_unix: now_unix,
            consecutive_failures: 0,
            reached_finality: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatusPollConfig {
    pub pending_poll_seconds: f64,
    pub confirmed_poll_seconds: f64,
    pub backoff_max_seconds: f64,
    pub finality_confirmations: u32,
    pub pending_failure_timeout_seconds: f64,
    pub pending_failure_min_failures: u32,
}

/// Matches Swift `shouldPollTransactionStatus`.
pub fn plan_transaction_status_should_poll(
    tracker: Option<TransactionStatusTrackerState>,
    now_unix: f64,
) -> bool {
    let tracker = tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    if tracker.reached_finality {
        return false;
    }
    now_unix >= tracker.next_check_at_unix
}

/// Matches Swift `markTransactionStatusPollSuccess`.
pub fn plan_transaction_status_poll_success(
    tracker: Option<TransactionStatusTrackerState>,
    resolved_status_confirmed: bool,
    resolved_status_pending: bool,
    reported_confirmations: Option<u32>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> TransactionStatusTrackerState {
    let mut tracker =
        tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    tracker.last_checked_at_unix = Some(now_unix);
    tracker.consecutive_failures = 0;
    let reached_finality = if resolved_status_pending {
        false
    } else {
        reported_confirmations.unwrap_or(config.finality_confirmations)
            >= config.finality_confirmations
    };
    if reached_finality {
        tracker.reached_finality = true;
        tracker.next_check_at_unix = now_unix + config.backoff_max_seconds;
    } else if resolved_status_confirmed {
        tracker.next_check_at_unix = now_unix + config.confirmed_poll_seconds;
    } else {
        tracker.next_check_at_unix = now_unix + config.pending_poll_seconds;
    }
    tracker
}

/// Matches Swift `markTransactionStatusPollFailure`. Exponential backoff capped at
/// `config.backoff_max_seconds`.
pub fn plan_transaction_status_poll_failure(
    tracker: Option<TransactionStatusTrackerState>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> TransactionStatusTrackerState {
    let mut tracker =
        tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    tracker.last_checked_at_unix = Some(now_unix);
    tracker.consecutive_failures = tracker.consecutive_failures.saturating_add(1);
    let exponent = tracker.consecutive_failures.saturating_sub(1) as i32;
    let backoff =
        (config.pending_poll_seconds * 2f64.powi(exponent)).min(config.backoff_max_seconds);
    tracker.next_check_at_unix = now_unix + backoff;
    tracker
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StalePendingFailureTransactionInput {
    pub id: String,
    pub created_at_unix: f64,
    pub status_is_pending: bool,
    pub tracker_consecutive_failures: u32,
}

/// Matches Swift `stalePendingFailureIDs`.
pub fn plan_stale_pending_failure_ids(
    transactions: Vec<StalePendingFailureTransactionInput>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> Vec<String> {
    transactions
        .into_iter()
        .filter(|transaction| {
            if !transaction.status_is_pending {
                return false;
            }
            let age = now_unix - transaction.created_at_unix;
            if age < config.pending_failure_timeout_seconds {
                return false;
            }
            transaction.tracker_consecutive_failures >= config.pending_failure_min_failures
        })
        .map(|transaction| transaction.id)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPendingStatusInput {
    pub status: String,
    pub confirmations: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPendingTransactionInput {
    pub id: String,
    pub old_status: String,
    pub old_failure_reason: Option<String>,
    pub old_confirmations: Option<u32>,
    pub resolution: Option<ResolvedPendingStatusInput>,
    pub is_stale_failure: bool,
    pub current_tracker: Option<TransactionStatusTrackerState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum FailureReasonDisposition {
    None,
    Preserve,
    LocalizedFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPendingTransactionDecision {
    pub id: String,
    pub new_status: String,
    pub status_changed: bool,
    pub failure_reason_disposition: FailureReasonDisposition,
    pub updated_tracker: Option<TransactionStatusTrackerState>,
    pub emit_event_code: Option<String>,
    /// When set, emit a chain-event indicating the transaction newly reached the
    /// finality threshold this poll cycle. Independent of `status_changed` so it
    /// fires when `confirmed→confirmed` but confirmations crossed the threshold.
    pub reached_finality_confirmations: Option<u32>,
    pub send_status_notification: bool,
}

/// Matches Swift `applyResolvedPendingTransactionStatuses` decision logic. Swift keeps
/// the `setTransactions` mutation and notification/event emission; Rust returns a
/// per-transaction decision describing what changed.
pub fn plan_apply_resolved_pending_transaction_statuses(
    inputs: Vec<ResolvedPendingTransactionInput>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> Vec<ResolvedPendingTransactionDecision> {
    inputs
        .into_iter()
        .filter_map(|input| {
            if let Some(resolution) = input.resolution {
                let new_status = resolution.status.clone();
                let status_changed = input.old_status != new_status;
                let new_confirmations = resolution.confirmations;
                let updated_tracker = if new_status != "pending" {
                    let mut tracker = input
                        .current_tracker
                        .clone()
                        .unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
                    tracker.reached_finality = new_confirmations
                        .unwrap_or(config.finality_confirmations)
                        >= config.finality_confirmations;
                    tracker.next_check_at_unix = now_unix + config.backoff_max_seconds;
                    Some(tracker)
                } else {
                    None
                };
                let failure_reason_disposition = if new_status == "failed" {
                    if input.old_failure_reason.is_some() {
                        FailureReasonDisposition::Preserve
                    } else {
                        FailureReasonDisposition::LocalizedFallback
                    }
                } else {
                    FailureReasonDisposition::None
                };
                let emit_event_code = if status_changed && new_status == "confirmed" {
                    Some("confirmed".to_string())
                } else if status_changed && new_status == "failed" {
                    Some("failed".to_string())
                } else {
                    None
                };
                let reached_finality_confirmations = match (new_confirmations, input.old_confirmations) {
                    (Some(new_count), old) if new_status == "confirmed"
                        && new_count >= config.finality_confirmations
                        && old.unwrap_or(0) < config.finality_confirmations =>
                    {
                        Some(new_count)
                    }
                    _ => None,
                };
                Some(ResolvedPendingTransactionDecision {
                    id: input.id,
                    new_status,
                    status_changed,
                    failure_reason_disposition,
                    updated_tracker,
                    emit_event_code,
                    reached_finality_confirmations,
                    send_status_notification: status_changed,
                })
            } else if input.is_stale_failure {
                let new_status = "failed".to_string();
                let status_changed = input.old_status != new_status;
                let failure_reason_disposition = if input.old_failure_reason.is_some() {
                    FailureReasonDisposition::Preserve
                } else {
                    FailureReasonDisposition::LocalizedFallback
                };
                let emit_event_code = if status_changed {
                    Some("failed".to_string())
                } else {
                    None
                };
                Some(ResolvedPendingTransactionDecision {
                    id: input.id,
                    new_status,
                    status_changed,
                    failure_reason_disposition,
                    updated_tracker: None,
                    emit_event_code,
                    reached_finality_confirmations: None,
                    send_status_notification: status_changed,
                })
            } else {
                None
            }
        })
        .collect()
}

// ─── M: Ethereum send error classification ────────────────────────────────────
//
// Matches Swift `mapEthereumSendError`. Swift passes the lowercased error
// message; Rust returns a code. Swift materializes the localized string.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum EthereumSendErrorCode {
    NonceTooLow,
    ReplacementUnderpriced,
    AlreadyKnown,
    InsufficientFunds,
    MaxFeeBelowBaseFee,
    IntrinsicGasLow,
    Unknown,
}

pub fn plan_ethereum_send_error_code(message: String) -> EthereumSendErrorCode {
    let lower = message.to_lowercase();
    if lower.contains("nonce too low") {
        EthereumSendErrorCode::NonceTooLow
    } else if lower.contains("replacement transaction underpriced") {
        EthereumSendErrorCode::ReplacementUnderpriced
    } else if lower.contains("already known") {
        EthereumSendErrorCode::AlreadyKnown
    } else if lower.contains("insufficient funds") {
        EthereumSendErrorCode::InsufficientFunds
    } else if lower.contains("max fee per gas less than block base fee") {
        EthereumSendErrorCode::MaxFeeBelowBaseFee
    } else if lower.contains("intrinsic gas too low") {
        EthereumSendErrorCode::IntrinsicGasLow
    } else {
        EthereumSendErrorCode::Unknown
    }
}

// ─── N: Chain keypool state (baseline + merge with existing) ──────────────────
//
// Matches Swift `baselineChainKeypoolState` + `keypoolState`. Swift filters
// transactions and owned addresses against its in-memory dictionaries and
// supplies the max-index inputs; Rust owns the `+1`, `max(...)`, and
// reserved-receive merge policy.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainKeypoolStateRecord {
    pub next_external_index: i32,
    pub next_change_index: i32,
    pub reserved_receive_index: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainKeypoolBaselineInput {
    pub supports_deep_utxo_discovery: bool,
    pub max_transaction_external_index: Option<i32>,
    pub max_transaction_change_index: Option<i32>,
    pub max_owned_external_index: Option<i32>,
    pub max_owned_change_index: Option<i32>,
    pub has_resolved_address: bool,
}

pub fn plan_baseline_chain_keypool_state(
    input: ChainKeypoolBaselineInput,
) -> ChainKeypoolStateRecord {
    if input.supports_deep_utxo_discovery {
        let max_external = input.max_transaction_external_index.unwrap_or(-1);
        let max_change = input.max_transaction_change_index.unwrap_or(-1);
        let max_owned_external = input.max_owned_external_index.unwrap_or(0);
        let max_owned_change = input.max_owned_change_index.unwrap_or(-1);
        return ChainKeypoolStateRecord {
            next_external_index: std::cmp::max(
                std::cmp::max(max_external, max_owned_external) + 1,
                1,
            ),
            next_change_index: std::cmp::max(
                std::cmp::max(max_change, max_owned_change) + 1,
                0,
            ),
            reserved_receive_index: None,
        };
    }
    let next_external_index = if input.has_resolved_address { 1 } else { 0 };
    ChainKeypoolStateRecord {
        next_external_index,
        next_change_index: 0,
        reserved_receive_index: if input.has_resolved_address {
            Some(0)
        } else {
            None
        },
    }
}

pub fn plan_chain_keypool_state(
    baseline: ChainKeypoolStateRecord,
    existing: Option<ChainKeypoolStateRecord>,
) -> ChainKeypoolStateRecord {
    let Some(mut state) = existing else {
        return baseline;
    };
    state.next_external_index = std::cmp::max(state.next_external_index, baseline.next_external_index);
    state.next_change_index = std::cmp::max(state.next_change_index, baseline.next_change_index);
    if state.reserved_receive_index.is_none() {
        state.reserved_receive_index = baseline.reserved_receive_index;
    }
    if let Some(reserved) = state.reserved_receive_index {
        state.next_external_index = std::cmp::max(state.next_external_index, reserved + 1);
    }
    state
}

// ─── O: Wallet holdings merge from balance summary ────────────────────────────
//
// Matches Swift `holdingsAppliedFromSummary`. Rust owns the match-by-key
// policy; Swift applies the actions to its `CoreCoin` array, preserving
// visual properties (id, priceUsd) on updates and providing defaults on
// inserts.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HoldingMergeExistingInput {
    pub symbol: String,
    pub chain_name: String,
    pub contract_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HoldingMergeIncomingInput {
    pub name: String,
    pub symbol: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HoldingMergeAppendPayload {
    pub name: String,
    pub symbol: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum HoldingMergeAction {
    UpdateAmount {
        existing_index: u32,
        amount: f64,
    },
    Append {
        coin: HoldingMergeAppendPayload,
    },
}

fn holding_lookup_key(symbol: &str, chain_name: &str, contract: Option<&str>) -> String {
    if let Some(raw) = contract {
        format!("{}:{}", chain_name, raw.to_lowercase())
    } else {
        format!("{}:{}", chain_name, symbol)
    }
}

pub fn plan_apply_holdings_from_summary(
    existing: Vec<HoldingMergeExistingInput>,
    incoming: Vec<HoldingMergeIncomingInput>,
) -> Vec<HoldingMergeAction> {
    if incoming.is_empty() {
        return Vec::new();
    }
    let keys: Vec<String> = existing
        .iter()
        .map(|h| holding_lookup_key(&h.symbol, &h.chain_name, h.contract_address.as_deref()))
        .collect();
    let mut actions: Vec<HoldingMergeAction> = Vec::new();
    for holding in incoming {
        let key = holding_lookup_key(
            &holding.symbol,
            &holding.chain_name,
            holding.contract_address.as_deref(),
        );
        if let Some(index) = keys.iter().position(|existing_key| existing_key == &key) {
            actions.push(HoldingMergeAction::UpdateAmount {
                existing_index: index as u32,
                amount: holding.amount,
            });
        } else if holding.amount > 0.0 {
            actions.push(HoldingMergeAction::Append {
                coin: HoldingMergeAppendPayload {
                    name: holding.name,
                    symbol: holding.symbol,
                    coin_gecko_id: holding.coin_gecko_id,
                    chain_name: holding.chain_name,
                    token_standard: holding.token_standard,
                    contract_address: holding.contract_address,
                    amount: holding.amount,
                },
            });
        }
    }
    actions
}

pub fn plan_resolve_derived_or_stored_address(
    derived: Option<String>,
    stored: Option<String>,
    validation_kind: String,
    validation_network_mode: Option<String>,
    derived_post_process: DerivedAddressPostProcess,
    normalize_stored: bool,
) -> Option<String> {
    if let Some(raw) = derived {
        let processed = match derived_post_process {
            DerivedAddressPostProcess::Lowercase => raw.to_lowercase(),
            DerivedAddressPostProcess::Trim => raw.trim().to_string(),
            DerivedAddressPostProcess::None => raw,
        };
        let result = validate_address(AddressValidationRequest {
            kind: validation_kind.clone(),
            value: processed.clone(),
            network_mode: validation_network_mode.clone(),
        });
        if result.is_valid {
            return Some(processed);
        }
    }
    let stored = stored?;
    if normalize_stored {
        let result = validate_address(AddressValidationRequest {
            kind: validation_kind.clone(),
            value: stored.clone(),
            network_mode: validation_network_mode.clone(),
        });
        if let Some(normalized) = result.normalized_value {
            return Some(normalized);
        }
    }
    let trimmed = stored.trim().to_string();
    let result = validate_address(AddressValidationRequest {
        kind: validation_kind,
        value: trimmed.clone(),
        network_mode: validation_network_mode,
    });
    if result.is_valid {
        Some(trimmed)
    } else {
        None
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
mod tests;

// ── FFI surface (relocated from ffi.rs) ──────────────────────────────────

#[uniffi::export]
pub fn core_build_persisted_snapshot(
    app_state: crate::store::state::CoreAppState,
    secret_observations: Vec<WalletSecretObservation>,
) -> PersistedAppSnapshot {
    build_persisted_snapshot_typed(app_state, secret_observations)
}

#[uniffi::export]
pub fn core_wallet_secret_index(
    app_state: crate::store::state::CoreAppState,
    secret_observations: Vec<WalletSecretObservation>,
) -> WalletSecretIndex {
    wallet_secret_index_from_observations(app_state, secret_observations)
}

#[uniffi::export]
pub fn core_plan_store_derived_state(
    request: StoreDerivedStateRequest,
) -> StoreDerivedStatePlan {
    plan_store_derived_state(request)
}

#[uniffi::export]
pub fn core_aggregate_owned_addresses(request: OwnedAddressAggregationRequest) -> Vec<String> {
    aggregate_owned_addresses(request)
}

#[uniffi::export]
pub fn core_plan_receive_selection(request: ReceiveSelectionRequest) -> ReceiveSelectionPlan {
    plan_receive_selection(request)
}

#[uniffi::export]
pub fn core_plan_self_send_confirmation(
    request: SelfSendConfirmationRequest,
) -> SelfSendConfirmationPlan {
    plan_self_send_confirmation(request)
}

#[uniffi::export]
pub fn core_plan_dashboard_supported_token_entries(
    entries: Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry>,
) -> Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry> {
    plan_dashboard_supported_token_entries(entries)
}

#[uniffi::export]
pub fn core_plan_merge_built_in_token_preferences(
    built_ins: Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry>,
    persisted: Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry>,
) -> Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry> {
    plan_merge_built_in_token_preferences(built_ins, persisted)
}

#[uniffi::export]
pub fn core_plan_price_alert_evaluation(
    alerts: Vec<PriceAlertEvaluationAlert>,
    prices: Vec<PriceAlertEvaluationPrice>,
) -> PriceAlertEvaluationPlan {
    plan_price_alert_evaluation(alerts, prices)
}

#[uniffi::export]
pub fn core_plan_dashboard_rebuild_for_live_price_change(
    request: DashboardRebuildDecisionRequest,
) -> bool {
    plan_dashboard_rebuild_for_live_price_change(request)
}

#[uniffi::export]
pub fn core_plan_ethereum_custom_fee_validation(
    use_custom_fees: bool,
    is_ethereum_chain: bool,
    max_fee_gwei_raw: String,
    priority_fee_gwei_raw: String,
) -> Option<EthereumCustomFeeValidationCode> {
    plan_ethereum_custom_fee_validation(
        use_custom_fees,
        is_ethereum_chain,
        max_fee_gwei_raw,
        priority_fee_gwei_raw,
    )
}

#[uniffi::export]
pub fn core_plan_ethereum_manual_nonce_validation(
    manual_nonce_enabled: bool,
    nonce_raw: String,
) -> Option<EthereumManualNonceValidationCode> {
    plan_ethereum_manual_nonce_validation(manual_nonce_enabled, nonce_raw)
}

#[uniffi::export]
pub fn core_plan_append_chain_operational_event(
    existing_events: Vec<ChainOperationalEventRecord>,
    new_event: ChainOperationalEventRecord,
) -> Vec<ChainOperationalEventRecord> {
    plan_append_chain_operational_event(existing_events, new_event)
}

#[uniffi::export]
pub fn core_plan_transaction_status_should_poll(
    tracker: Option<TransactionStatusTrackerState>,
    now_unix: f64,
) -> bool {
    plan_transaction_status_should_poll(tracker, now_unix)
}

#[uniffi::export]
pub fn core_plan_transaction_status_poll_success(
    tracker: Option<TransactionStatusTrackerState>,
    resolved_status_confirmed: bool,
    resolved_status_pending: bool,
    reported_confirmations: Option<u32>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> TransactionStatusTrackerState {
    plan_transaction_status_poll_success(
        tracker,
        resolved_status_confirmed,
        resolved_status_pending,
        reported_confirmations,
        now_unix,
        config,
    )
}

#[uniffi::export]
pub fn core_plan_transaction_status_poll_failure(
    tracker: Option<TransactionStatusTrackerState>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> TransactionStatusTrackerState {
    plan_transaction_status_poll_failure(tracker, now_unix, config)
}

#[uniffi::export]
pub fn core_plan_stale_pending_failure_ids(
    transactions: Vec<StalePendingFailureTransactionInput>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> Vec<String> {
    plan_stale_pending_failure_ids(transactions, now_unix, config)
}

#[uniffi::export]
pub fn core_plan_apply_resolved_pending_transaction_statuses(
    inputs: Vec<ResolvedPendingTransactionInput>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> Vec<ResolvedPendingTransactionDecision> {
    plan_apply_resolved_pending_transaction_statuses(inputs, now_unix, config)
}

#[uniffi::export]
pub fn core_plan_ethereum_send_error_code(message: String) -> EthereumSendErrorCode {
    plan_ethereum_send_error_code(message)
}

#[uniffi::export]
pub fn core_plan_baseline_chain_keypool_state(
    input: ChainKeypoolBaselineInput,
) -> ChainKeypoolStateRecord {
    plan_baseline_chain_keypool_state(input)
}

#[uniffi::export]
pub fn core_plan_chain_keypool_state(
    baseline: ChainKeypoolStateRecord,
    existing: Option<ChainKeypoolStateRecord>,
) -> ChainKeypoolStateRecord {
    plan_chain_keypool_state(baseline, existing)
}

#[uniffi::export]
pub fn core_plan_apply_holdings_from_summary(
    existing: Vec<HoldingMergeExistingInput>,
    incoming: Vec<HoldingMergeIncomingInput>,
) -> Vec<HoldingMergeAction> {
    plan_apply_holdings_from_summary(existing, incoming)
}

#[uniffi::export]
pub fn core_plan_evm_recipient_preflight_warnings(
    request: EvmRecipientPreflightRequest,
) -> Vec<EvmRecipientPreflightWarning> {
    plan_evm_recipient_preflight_warnings(request)
}

#[uniffi::export]
pub fn core_plan_priced_chain(
    chain_name: String,
    bitcoin_network_mode_raw: String,
    ethereum_network_mode_raw: String,
) -> bool {
    plan_priced_chain(chain_name, bitcoin_network_mode_raw, ethereum_network_mode_raw)
}

#[uniffi::export]
pub fn core_plan_active_wallet_transaction_ids(
    transactions: Vec<TransactionActivityInput>,
    wallets: Vec<WalletChainInput>,
) -> Vec<String> {
    plan_active_wallet_transaction_ids(transactions, wallets)
}

#[uniffi::export]
pub fn core_plan_normalized_history_signature(
    transactions: Vec<NormalizedHistorySignatureTransaction>,
    wallets: Vec<WalletChainInput>,
) -> i64 {
    plan_normalized_history_signature(transactions, wallets)
}

#[uniffi::export]
pub fn core_plan_earliest_transaction_dates(
    transactions: Vec<TransactionEarliestInput>,
) -> Vec<WalletEarliestTransactionDate> {
    plan_earliest_transaction_dates(transactions)
}

#[uniffi::export]
pub fn core_plan_has_wallet_for_chain(
    chain_name: String,
    wallets: Vec<WalletChainEligibilityInput>,
) -> bool {
    plan_has_wallet_for_chain(chain_name, wallets)
}

#[uniffi::export]
pub fn core_plan_canonical_chain_component(chain_name: String, symbol: String) -> String {
    plan_canonical_chain_component(chain_name, symbol)
}

#[uniffi::export]
pub fn core_plan_icon_identifier(
    symbol: String,
    chain_name: String,
    contract_address: Option<String>,
    token_standard: String,
) -> String {
    plan_icon_identifier(symbol, chain_name, contract_address, token_standard)
}

#[uniffi::export]
pub fn core_plan_normalized_icon_identifier(identifier: String) -> String {
    plan_normalized_icon_identifier(identifier)
}

#[uniffi::export]
pub fn core_plan_reset_dispatch(scopes: Vec<String>) -> CoreResetPlan {
    plan_reset_dispatch(scopes)
}

#[uniffi::export]
pub fn core_plan_resolve_derived_or_stored_address(
    derived: Option<String>,
    stored: Option<String>,
    validation_kind: String,
    validation_network_mode: Option<String>,
    derived_post_process: DerivedAddressPostProcess,
    normalize_stored: bool,
) -> Option<String> {
    plan_resolve_derived_or_stored_address(
        derived,
        stored,
        validation_kind,
        validation_network_mode,
        derived_post_process,
        normalize_stored,
    )
}

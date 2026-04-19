pub mod app_state;
pub mod password_verifier;
pub mod persistence;
pub mod secret_store;
pub mod seed_envelope;
pub mod state;
pub mod wallet_core;
pub mod wallet_db;
pub mod wallet_domain;

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

fn known_chain_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("bitcoin", "bitcoin"),
        ("bitcoin cash", "bitcoin-cash"),
        ("bitcoin sv", "bitcoin-sv"),
        ("litecoin", "litecoin"),
        ("dogecoin", "dogecoin"),
        ("ethereum", "ethereum"),
        ("ethereum classic", "ethereum-classic"),
        ("arbitrum", "arbitrum"),
        ("optimism", "optimism"),
        ("bnb chain", "bnb"),
        ("avalanche", "avalanche"),
        ("hyperliquid", "hyperliquid"),
        ("tron", "tron"),
        ("solana", "solana"),
        ("stellar", "stellar"),
        ("cardano", "cardano"),
        ("xrp ledger", "xrp"),
        ("monero", "monero"),
        ("sui", "sui"),
        ("aptos", "aptos"),
        ("ton", "ton"),
        ("internet computer", "internet-computer"),
        ("near", "near"),
        ("polkadot", "polkadot"),
    ]
}

fn native_symbol_chain_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("BTC", "bitcoin"),
        ("BCH", "bitcoin-cash"),
        ("BSV", "bitcoin-sv"),
        ("LTC", "litecoin"),
        ("DOGE", "dogecoin"),
        ("ETH", "ethereum"),
        ("ETC", "ethereum-classic"),
        ("ARB", "arbitrum"),
        ("OP", "optimism"),
        ("BNB", "bnb"),
        ("AVAX", "avalanche"),
        ("HYPE", "hyperliquid"),
        ("TRX", "tron"),
        ("SOL", "solana"),
        ("XLM", "stellar"),
        ("ADA", "cardano"),
        ("XRP", "xrp"),
        ("XMR", "monero"),
        ("SUI", "sui"),
        ("APT", "aptos"),
        ("TON", "ton"),
        ("ICP", "internet-computer"),
        ("NEAR", "near"),
        ("DOT", "polkadot"),
    ]
}

fn chain_id_by_chain_name() -> &'static HashMap<String, String> {
    use std::sync::OnceLock;
    static LOOKUP: OnceLock<HashMap<String, String>> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        let raw = include_str!("../../../resources/strings/base/ChainWikiEntries.json");
        let mut map = HashMap::new();
        if let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(raw)
        {
            for entry in entries {
                let id = entry.get("id").and_then(|v| v.as_str());
                let name = entry.get("name").and_then(|v| v.as_str());
                if let (Some(id), Some(name)) = (id, name) {
                    map.insert(name.trim().to_lowercase(), id.to_string());
                }
            }
        }
        map
    })
}

fn canonical_chain_component_inner(chain_name: &str, symbol: &str) -> String {
    let normalized_chain = chain_name.trim().to_lowercase();
    let normalized_symbol = symbol.trim().to_uppercase();
    if let Some((_, alias)) = known_chain_aliases()
        .iter()
        .find(|(name, _)| *name == normalized_chain)
    {
        return (*alias).to_string();
    }
    if let Some(id) = chain_id_by_chain_name().get(&normalized_chain) {
        return id.clone();
    }
    if let Some((_, alias)) = native_symbol_chain_aliases()
        .iter()
        .find(|(sym, _)| *sym == normalized_symbol)
    {
        return (*alias).to_string();
    }
    normalized_chain.replace(' ', "-")
}

pub fn plan_canonical_chain_component(chain_name: String, symbol: String) -> String {
    canonical_chain_component_inner(&chain_name, &symbol)
}

pub fn plan_icon_identifier(
    symbol: String,
    chain_name: String,
    contract_address: Option<String>,
    token_standard: String,
) -> String {
    let normalized_symbol = symbol.to_lowercase();
    let trimmed_contract = contract_address
        .map(|c| c.trim().to_string())
        .unwrap_or_default();
    let normalized_chain = canonical_chain_component_inner(&chain_name, &symbol);
    if !trimmed_contract.is_empty() {
        return format!(
            "token:{}:{}:{}",
            normalized_chain,
            normalized_symbol,
            trimmed_contract.to_lowercase()
        );
    }
    let is_native_token =
        token_standard.eq_ignore_ascii_case("Native") || token_standard.is_empty();
    let namespace = if is_native_token { "native" } else { "asset" };
    format!("{namespace}:{normalized_chain}:{normalized_symbol}")
}

pub fn plan_normalized_icon_identifier(identifier: String) -> String {
    let trimmed_identifier = identifier.trim().to_string();
    let components: Vec<String> = trimmed_identifier.split(':').map(String::from).collect();
    if components.len() < 3 {
        return trimmed_identifier;
    }
    let namespace = &components[0];
    let chain_component = &components[1];
    let symbol_component = &components[2];
    match namespace.as_str() {
        "native" | "asset" | "token" => {
            let canonical_chain =
                canonical_chain_component_inner(chain_component, symbol_component);
            let mut normalized = components.clone();
            normalized[0] = namespace.clone();
            normalized[1] = canonical_chain;
            normalized[2] = symbol_component.to_lowercase();
            if normalized.len() >= 4 {
                normalized[3] = normalized[3].to_lowercase();
            }
            normalized.join(":")
        }
        _ => trimmed_identifier,
    }
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
mod tests {
    use super::{
        aggregate_owned_addresses, build_persisted_snapshot, persisted_snapshot_from_json,
        plan_receive_selection, plan_self_send_confirmation, plan_store_derived_state,
        wallet_secret_index, OwnedAddressAggregationRequest, PendingSelfSendConfirmationInput,
        PersistedAppSnapshot, PersistedAppSnapshotRequest, ReceiveSelectionHoldingInput,
        ReceiveSelectionRequest, SelfSendConfirmationRequest, StoreDerivedHoldingInput,
        StoreDerivedStateRequest, StoreDerivedWalletInput, WalletSecretObservation,
    };
    use crate::state::CoreAppState;
    use std::collections::HashMap;

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
        app_state.wallets.push(crate::state::WalletSummary {
            id: "wallet-1".to_string(),
            name: "Main".to_string(),
            is_watch_only: false,
            chain_name: "Bitcoin".to_string(),
            include_in_portfolio_total: true,
            network_mode: Some("mainnet".to_string()),
            xpub: None,
            derivation_preset: "standard".to_string(),
            derivation_path: None,
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

    #[test]
    fn aggregates_owned_addresses_in_order_without_duplicates() {
        let addresses = aggregate_owned_addresses(OwnedAddressAggregationRequest {
            candidate_addresses: vec![
                " 0xAbc ".to_string(),
                "".to_string(),
                "0xabc".to_string(),
                "bc1example".to_string(),
            ],
        });

        assert_eq!(
            addresses,
            vec!["0xAbc".to_string(), "bc1example".to_string()]
        );
    }

    #[test]
    fn prefers_native_receive_holding_for_resolved_chain() {
        let plan = plan_receive_selection(ReceiveSelectionRequest {
            receive_chain_name: "Ethereum".to_string(),
            available_receive_chains: vec!["Ethereum".to_string()],
            available_receive_holdings: vec![
                ReceiveSelectionHoldingInput {
                    holding_index: 0,
                    chain_name: "Ethereum".to_string(),
                    has_contract_address: true,
                },
                ReceiveSelectionHoldingInput {
                    holding_index: 1,
                    chain_name: "Ethereum".to_string(),
                    has_contract_address: false,
                },
            ],
        });

        assert_eq!(plan.resolved_chain_name, "Ethereum");
        assert_eq!(plan.selected_receive_holding_index, Some(1));
    }

    #[test]
    fn consumes_matching_pending_self_send_confirmation() {
        let plan = plan_self_send_confirmation(SelfSendConfirmationRequest {
            pending_confirmation: Some(PendingSelfSendConfirmationInput {
                wallet_id: "wallet-1".to_string(),
                chain_name: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                destination_address_lowercased: "bc1self".to_string(),
                amount: 1.5,
                created_at_unix: 100.0,
            }),
            wallet_id: "wallet-1".to_string(),
            chain_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            destination_address: "BC1SELF".to_string(),
            amount: 1.5,
            now_unix: 110.0,
            window_seconds: 30.0,
            owned_addresses: vec!["bc1self".to_string()],
        });

        assert!(!plan.requires_confirmation);
        assert!(plan.consume_existing_confirmation);
        assert!(plan.clear_pending_confirmation);
    }
}

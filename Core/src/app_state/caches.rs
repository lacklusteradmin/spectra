// Rust-owned derived caches for Swift's AppState.
//
// Previously these were stored `cached*` dictionaries/arrays/sets on AppState.
// Each cache is a pure derivation over `wallets` / `derivedStatePlan`; moving
// the storage here shrinks AppState and aligns with the Swift->Rust migration.
//
// Pattern (matches `diagnostics/registry.rs`): one typed field per cache on a
// `CachesRegistry`, a single global `Mutex<CachesRegistry>`, and a UniFFI
// `get/set/clear` trio that returns / accepts the full container. Swift then
// exposes computed vars (same names as before) delegating here, bumping a
// `cachesRevision` so SwiftUI invalidates.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use crate::wallet_domain::{
    CoreCoin, CoreDashboardAssetGroup, CoreDashboardPinOption, CoreImportedWallet,
    CoreTokenPreferenceEntry, CoreWalletRustSecretMaterialDescriptor,
};

#[derive(Default)]
struct CachesRegistry {
    wallet_by_id: HashMap<String, CoreImportedWallet>,
    wallet_by_id_string: HashMap<String, CoreImportedWallet>,
    included_portfolio_wallets: Vec<CoreImportedWallet>,
    included_portfolio_holdings: Vec<CoreCoin>,
    included_portfolio_holdings_by_symbol: HashMap<String, Vec<CoreCoin>>,
    unique_wallet_price_request_coins: Vec<CoreCoin>,
    portfolio: Vec<CoreCoin>,
    available_send_coins_by_wallet_id: HashMap<String, Vec<CoreCoin>>,
    available_receive_coins_by_wallet_id: HashMap<String, Vec<CoreCoin>>,
    available_receive_chains_by_wallet_id: HashMap<String, Vec<String>>,
    send_enabled_wallets: Vec<CoreImportedWallet>,
    receive_enabled_wallets: Vec<CoreImportedWallet>,
    refreshable_chain_names: HashSet<String>,
    signing_material_wallet_ids: HashSet<String>,
    private_key_backed_wallet_ids: HashSet<String>,
    password_protected_wallet_ids: HashSet<String>,
    resolved_ens_addresses: HashMap<String, String>,
    // Token-preference caches (previously blocked on TokenPreferenceEntry port).
    resolved_token_preferences: Vec<CoreTokenPreferenceEntry>,
    // Keyed by TokenTrackingChain rawValue (display name); Swift converts on read.
    token_preferences_by_chain: HashMap<String, Vec<CoreTokenPreferenceEntry>>,
    resolved_token_preferences_by_symbol: HashMap<String, Vec<CoreTokenPreferenceEntry>>,
    enabled_tracked_token_preferences: Vec<CoreTokenPreferenceEntry>,
    token_preference_by_chain_and_symbol: HashMap<String, CoreTokenPreferenceEntry>,
    // Dashboard caches (previously blocked on DashboardPinOption / AssetGroup port).
    pinned_dashboard_asset_symbols: Vec<String>,
    dashboard_pin_option_by_symbol: HashMap<String, CoreDashboardPinOption>,
    available_dashboard_pin_options: Vec<CoreDashboardPinOption>,
    dashboard_asset_groups: Vec<CoreDashboardAssetGroup>,
    dashboard_relevant_price_keys: HashSet<String>,
    dashboard_supported_token_entries_by_symbol:
        HashMap<String, Vec<CoreTokenPreferenceEntry>>,
    // Secret-descriptor cache (previously blocked on WalletRustSecretMaterialDescriptor port).
    secret_descriptors_by_wallet_id: HashMap<String, CoreWalletRustSecretMaterialDescriptor>,
}

impl CachesRegistry {
    fn clear(&mut self) {
        *self = Self::default();
    }
}

fn registry() -> &'static Mutex<CachesRegistry> {
    static REG: OnceLock<Mutex<CachesRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(CachesRegistry::default()))
}

macro_rules! map_cache {
    ($field:ident, $val:ty, $get_all:ident, $replace:ident, $get_one:ident, $set_one:ident, $remove_one:ident) => {
        #[uniffi::export]
        pub fn $get_all() -> HashMap<String, $val> {
            registry().lock().unwrap().$field.clone()
        }
        #[uniffi::export]
        pub fn $replace(entries: HashMap<String, $val>) {
            registry().lock().unwrap().$field = entries;
        }
        #[uniffi::export]
        pub fn $get_one(key: String) -> Option<$val> {
            registry().lock().unwrap().$field.get(&key).cloned()
        }
        #[uniffi::export]
        pub fn $set_one(key: String, value: $val) {
            registry().lock().unwrap().$field.insert(key, value);
        }
        #[uniffi::export]
        pub fn $remove_one(key: String) {
            registry().lock().unwrap().$field.remove(&key);
        }
    };
}

macro_rules! vec_cache {
    ($field:ident, $val:ty, $get_all:ident, $replace:ident) => {
        #[uniffi::export]
        pub fn $get_all() -> Vec<$val> {
            registry().lock().unwrap().$field.clone()
        }
        #[uniffi::export]
        pub fn $replace(entries: Vec<$val>) {
            registry().lock().unwrap().$field = entries;
        }
    };
}

macro_rules! set_cache {
    ($field:ident, $get_all:ident, $replace:ident, $contains:ident, $insert:ident, $remove_one:ident) => {
        #[uniffi::export]
        pub fn $get_all() -> Vec<String> {
            let g = registry().lock().unwrap();
            let mut v: Vec<String> = g.$field.iter().cloned().collect();
            v.sort();
            v
        }
        #[uniffi::export]
        pub fn $replace(entries: Vec<String>) {
            registry().lock().unwrap().$field = entries.into_iter().collect();
        }
        #[uniffi::export]
        pub fn $contains(key: String) -> bool {
            registry().lock().unwrap().$field.contains(&key)
        }
        #[uniffi::export]
        pub fn $insert(key: String) {
            registry().lock().unwrap().$field.insert(key);
        }
        #[uniffi::export]
        pub fn $remove_one(key: String) {
            registry().lock().unwrap().$field.remove(&key);
        }
    };
}

map_cache!(
    wallet_by_id,
    CoreImportedWallet,
    caches_get_wallet_by_id,
    caches_replace_wallet_by_id,
    caches_get_wallet_by_id_entry,
    caches_set_wallet_by_id_entry,
    caches_remove_wallet_by_id_entry
);
map_cache!(
    wallet_by_id_string,
    CoreImportedWallet,
    caches_get_wallet_by_id_string,
    caches_replace_wallet_by_id_string,
    caches_get_wallet_by_id_string_entry,
    caches_set_wallet_by_id_string_entry,
    caches_remove_wallet_by_id_string_entry
);
map_cache!(
    included_portfolio_holdings_by_symbol,
    Vec<CoreCoin>,
    caches_get_included_portfolio_holdings_by_symbol,
    caches_replace_included_portfolio_holdings_by_symbol,
    caches_get_included_portfolio_holdings_for_symbol,
    caches_set_included_portfolio_holdings_for_symbol,
    caches_remove_included_portfolio_holdings_for_symbol
);
map_cache!(
    available_send_coins_by_wallet_id,
    Vec<CoreCoin>,
    caches_get_available_send_coins_by_wallet_id,
    caches_replace_available_send_coins_by_wallet_id,
    caches_get_available_send_coins_for_wallet,
    caches_set_available_send_coins_for_wallet,
    caches_remove_available_send_coins_for_wallet
);
map_cache!(
    available_receive_coins_by_wallet_id,
    Vec<CoreCoin>,
    caches_get_available_receive_coins_by_wallet_id,
    caches_replace_available_receive_coins_by_wallet_id,
    caches_get_available_receive_coins_for_wallet,
    caches_set_available_receive_coins_for_wallet,
    caches_remove_available_receive_coins_for_wallet
);
map_cache!(
    available_receive_chains_by_wallet_id,
    Vec<String>,
    caches_get_available_receive_chains_by_wallet_id,
    caches_replace_available_receive_chains_by_wallet_id,
    caches_get_available_receive_chains_for_wallet,
    caches_set_available_receive_chains_for_wallet,
    caches_remove_available_receive_chains_for_wallet
);
map_cache!(
    resolved_ens_addresses,
    String,
    caches_get_resolved_ens_addresses,
    caches_replace_resolved_ens_addresses,
    caches_get_resolved_ens_address,
    caches_set_resolved_ens_address,
    caches_remove_resolved_ens_address
);

vec_cache!(
    included_portfolio_wallets,
    CoreImportedWallet,
    caches_get_included_portfolio_wallets,
    caches_replace_included_portfolio_wallets
);
vec_cache!(
    included_portfolio_holdings,
    CoreCoin,
    caches_get_included_portfolio_holdings,
    caches_replace_included_portfolio_holdings
);
vec_cache!(
    unique_wallet_price_request_coins,
    CoreCoin,
    caches_get_unique_wallet_price_request_coins,
    caches_replace_unique_wallet_price_request_coins
);
vec_cache!(portfolio, CoreCoin, caches_get_portfolio, caches_replace_portfolio);
vec_cache!(
    send_enabled_wallets,
    CoreImportedWallet,
    caches_get_send_enabled_wallets,
    caches_replace_send_enabled_wallets
);
vec_cache!(
    receive_enabled_wallets,
    CoreImportedWallet,
    caches_get_receive_enabled_wallets,
    caches_replace_receive_enabled_wallets
);

set_cache!(
    refreshable_chain_names,
    caches_get_refreshable_chain_names,
    caches_replace_refreshable_chain_names,
    caches_refreshable_chain_names_contains,
    caches_insert_refreshable_chain_name,
    caches_remove_refreshable_chain_name
);
set_cache!(
    signing_material_wallet_ids,
    caches_get_signing_material_wallet_ids,
    caches_replace_signing_material_wallet_ids,
    caches_signing_material_wallet_ids_contains,
    caches_insert_signing_material_wallet_id,
    caches_remove_signing_material_wallet_id
);
set_cache!(
    private_key_backed_wallet_ids,
    caches_get_private_key_backed_wallet_ids,
    caches_replace_private_key_backed_wallet_ids,
    caches_private_key_backed_wallet_ids_contains,
    caches_insert_private_key_backed_wallet_id,
    caches_remove_private_key_backed_wallet_id
);
set_cache!(
    password_protected_wallet_ids,
    caches_get_password_protected_wallet_ids,
    caches_replace_password_protected_wallet_ids,
    caches_password_protected_wallet_ids_contains,
    caches_insert_password_protected_wallet_id,
    caches_remove_password_protected_wallet_id
);

// ── Token-preference caches ─────────────────────────────────────────────
vec_cache!(
    resolved_token_preferences,
    CoreTokenPreferenceEntry,
    caches_get_resolved_token_preferences,
    caches_replace_resolved_token_preferences
);
vec_cache!(
    enabled_tracked_token_preferences,
    CoreTokenPreferenceEntry,
    caches_get_enabled_tracked_token_preferences,
    caches_replace_enabled_tracked_token_preferences
);
map_cache!(
    token_preferences_by_chain,
    Vec<CoreTokenPreferenceEntry>,
    caches_get_token_preferences_by_chain,
    caches_replace_token_preferences_by_chain,
    caches_get_token_preferences_for_chain,
    caches_set_token_preferences_for_chain,
    caches_remove_token_preferences_for_chain
);
map_cache!(
    resolved_token_preferences_by_symbol,
    Vec<CoreTokenPreferenceEntry>,
    caches_get_resolved_token_preferences_by_symbol,
    caches_replace_resolved_token_preferences_by_symbol,
    caches_get_resolved_token_preferences_for_symbol,
    caches_set_resolved_token_preferences_for_symbol,
    caches_remove_resolved_token_preferences_for_symbol
);
map_cache!(
    token_preference_by_chain_and_symbol,
    CoreTokenPreferenceEntry,
    caches_get_token_preference_by_chain_and_symbol,
    caches_replace_token_preference_by_chain_and_symbol,
    caches_get_token_preference_for_chain_and_symbol,
    caches_set_token_preference_for_chain_and_symbol,
    caches_remove_token_preference_for_chain_and_symbol
);

// ── Dashboard caches ────────────────────────────────────────────────────
#[uniffi::export]
pub fn caches_get_pinned_dashboard_asset_symbols() -> Vec<String> {
    registry().lock().unwrap().pinned_dashboard_asset_symbols.clone()
}
#[uniffi::export]
pub fn caches_replace_pinned_dashboard_asset_symbols(entries: Vec<String>) {
    registry().lock().unwrap().pinned_dashboard_asset_symbols = entries;
}
map_cache!(
    dashboard_pin_option_by_symbol,
    CoreDashboardPinOption,
    caches_get_dashboard_pin_option_by_symbol,
    caches_replace_dashboard_pin_option_by_symbol,
    caches_get_dashboard_pin_option_for_symbol,
    caches_set_dashboard_pin_option_for_symbol,
    caches_remove_dashboard_pin_option_for_symbol
);
vec_cache!(
    available_dashboard_pin_options,
    CoreDashboardPinOption,
    caches_get_available_dashboard_pin_options,
    caches_replace_available_dashboard_pin_options
);
vec_cache!(
    dashboard_asset_groups,
    CoreDashboardAssetGroup,
    caches_get_dashboard_asset_groups,
    caches_replace_dashboard_asset_groups
);
set_cache!(
    dashboard_relevant_price_keys,
    caches_get_dashboard_relevant_price_keys,
    caches_replace_dashboard_relevant_price_keys,
    caches_dashboard_relevant_price_keys_contains,
    caches_insert_dashboard_relevant_price_key,
    caches_remove_dashboard_relevant_price_key
);
map_cache!(
    dashboard_supported_token_entries_by_symbol,
    Vec<CoreTokenPreferenceEntry>,
    caches_get_dashboard_supported_token_entries_by_symbol,
    caches_replace_dashboard_supported_token_entries_by_symbol,
    caches_get_dashboard_supported_token_entries_for_symbol,
    caches_set_dashboard_supported_token_entries_for_symbol,
    caches_remove_dashboard_supported_token_entries_for_symbol
);

// ── Secret descriptors ──────────────────────────────────────────────────
map_cache!(
    secret_descriptors_by_wallet_id,
    CoreWalletRustSecretMaterialDescriptor,
    caches_get_secret_descriptors_by_wallet_id,
    caches_replace_secret_descriptors_by_wallet_id,
    caches_get_secret_descriptor_for_wallet,
    caches_set_secret_descriptor_for_wallet,
    caches_remove_secret_descriptor_for_wallet
);

#[uniffi::export]
pub fn caches_clear_all() {
    registry().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Single shared global; serialize tests to avoid cross-test interleaving.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn sample_wallet(id: &str) -> CoreImportedWallet {
        let mut w = CoreImportedWallet::default();
        w.id = id.to_string();
        w
    }

    #[test]
    fn map_cache_roundtrip() {
        let _g = test_lock();
        caches_clear_all();
        assert!(caches_get_wallet_by_id().is_empty());
        caches_set_wallet_by_id_entry("w1".into(), sample_wallet("w1"));
        caches_set_wallet_by_id_entry("w2".into(), sample_wallet("w2"));
        assert_eq!(caches_get_wallet_by_id().len(), 2);
        assert_eq!(
            caches_get_wallet_by_id_entry("w1".into()).unwrap().id,
            "w1"
        );
        caches_remove_wallet_by_id_entry("w1".into());
        assert!(caches_get_wallet_by_id_entry("w1".into()).is_none());

        let mut replacement = HashMap::new();
        replacement.insert("w9".into(), sample_wallet("w9"));
        caches_replace_wallet_by_id(replacement);
        assert_eq!(caches_get_wallet_by_id().len(), 1);
        assert!(caches_get_wallet_by_id_entry("w9".into()).is_some());
        caches_clear_all();
    }

    #[test]
    fn vec_cache_roundtrip() {
        let _g = test_lock();
        caches_clear_all();
        caches_replace_portfolio(vec![CoreCoin::default()]);
        assert_eq!(caches_get_portfolio().len(), 1);
        caches_replace_portfolio(Vec::new());
        assert!(caches_get_portfolio().is_empty());
    }

    #[test]
    fn set_cache_roundtrip() {
        let _g = test_lock();
        caches_clear_all();
        caches_insert_refreshable_chain_name("Bitcoin".into());
        caches_insert_refreshable_chain_name("Ethereum".into());
        assert!(caches_refreshable_chain_names_contains("Bitcoin".into()));
        assert_eq!(caches_get_refreshable_chain_names().len(), 2);
        caches_remove_refreshable_chain_name("Bitcoin".into());
        assert!(!caches_refreshable_chain_names_contains("Bitcoin".into()));
        caches_replace_refreshable_chain_names(vec!["Solana".into(), "Solana".into()]);
        assert_eq!(caches_get_refreshable_chain_names(), vec!["Solana"]);
        caches_clear_all();
    }

    #[test]
    fn clear_all_resets_every_field() {
        let _g = test_lock();
        caches_set_wallet_by_id_entry("w".into(), sample_wallet("w"));
        caches_replace_portfolio(vec![CoreCoin::default()]);
        caches_insert_signing_material_wallet_id("w".into());
        caches_set_resolved_ens_address("k".into(), "v".into());
        caches_clear_all();
        assert!(caches_get_wallet_by_id().is_empty());
        assert!(caches_get_portfolio().is_empty());
        assert!(caches_get_signing_material_wallet_ids().is_empty());
        assert!(caches_get_resolved_ens_addresses().is_empty());
    }
}

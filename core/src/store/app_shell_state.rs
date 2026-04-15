// Rust-owned mirror of Swift AppState's @Published scalar fields.
//
// Swift AppState keeps its `var x: T` interface via computed properties that
// delegate here. Setters call `objectWillChange.send()` so SwiftUI observes
// changes. Complex types (wallets, transactions, previews) migrate in later
// stages; Stage 1 covers only Bool/Int/Double/String/Option<String> scalars
// whose @Published declarations in Swift have no `didSet` side effects.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Default)]
struct Inner {
    // Bools (side-effect-free)
    is_importing_wallet: bool,
    is_showing_wallet_importer: bool,
    is_showing_send_sheet: bool,
    is_showing_receive_sheet: bool,
    is_resolving_receive_address: bool,
    is_checking_send_destination_balance: bool,
    is_showing_high_risk_send_confirmation: bool,
    send_verification_notice_is_warning: bool,
    is_app_locked: bool,
    is_user_initiated_refresh_in_progress: bool,
    is_loading_more_on_chain_history: bool,
    is_running_bitcoin_rescan: bool,
    is_running_bitcoin_cash_rescan: bool,
    is_running_bitcoin_sv_rescan: bool,
    is_running_litecoin_rescan: bool,
    is_running_dogecoin_rescan: bool,
    is_preparing_ethereum_replacement_context: bool,
    use_custom_ethereum_fees: bool,
    send_advanced_mode: bool,
    send_enable_rbf: bool,
    send_enable_cpfp: bool,
    ethereum_manual_nonce_enabled: bool,
    is_sending_bitcoin: bool,
    is_sending_bitcoin_cash: bool,
    is_sending_bitcoin_sv: bool,
    is_sending_litecoin: bool,
    is_sending_dogecoin: bool,
    is_sending_ethereum: bool,
    is_sending_tron: bool,
    is_sending_solana: bool,
    is_sending_xrp: bool,
    is_sending_stellar: bool,
    is_sending_monero: bool,
    is_sending_cardano: bool,
    is_sending_sui: bool,
    is_sending_aptos: bool,
    is_sending_ton: bool,
    is_sending_icp: bool,
    is_sending_near: bool,
    is_sending_polkadot: bool,

    // Ints
    send_utxo_max_input_count: i64,
    bitcoin_stop_gap: i64,
    automatic_refresh_frequency_minutes: i64,

    // Doubles
    large_movement_alert_percent_threshold: f64,
    large_movement_alert_usd_threshold: f64,

    // Stage 2 bools
    hide_balances: bool,
    use_face_id: bool,
    use_auto_lock: bool,
    use_strict_rpc_only: bool,
    require_biometric_for_send_actions: bool,
    use_price_alerts: bool,
    use_transaction_status_notifications: bool,
    use_large_movement_notifications: bool,

    // Stage 2 enums (stored as rawValue String)
    pricing_provider: String,
    selected_fiat_currency: String,
    fiat_rate_provider: String,
    ethereum_network_mode: String,
    bitcoin_network_mode: String,
    dogecoin_network_mode: String,
    bitcoin_fee_priority: String,
    dogecoin_fee_priority: String,
    background_sync_profile: String,
    send_litecoin_change_strategy: String,

    // Stage 2 strings with didSet
    coin_gecko_api_key: String,
    ethereum_rpc_endpoint: String,
    etherscan_api_key: String,
    monero_backend_base_url: String,
    monero_backend_api_key: String,
    bitcoin_esplora_endpoints: String,

    // Stage 3 primitive dictionaries
    live_prices: HashMap<String, f64>,
    fiat_rates_from_usd: HashMap<String, f64>,
    asset_display_decimals_by_chain: HashMap<String, i64>,
    selected_fee_priority_option_raw_by_chain: HashMap<String, String>,

    // Strings
    send_wallet_id: String,
    send_holding_key: String,
    send_amount: String,
    send_address: String,
    receive_wallet_id: String,
    receive_chain_name: String,
    receive_holding_key: String,
    receive_resolved_address: String,
    custom_ethereum_max_fee_gwei: String,
    custom_ethereum_priority_fee_gwei: String,
    ethereum_manual_nonce: String,

    // Stage 4 isPreparing* sends
    is_preparing_ethereum_send: bool,
    is_preparing_dogecoin_send: bool,
    is_preparing_tron_send: bool,
    is_preparing_solana_send: bool,
    is_preparing_xrp_send: bool,
    is_preparing_stellar_send: bool,
    is_preparing_monero_send: bool,
    is_preparing_cardano_send: bool,
    is_preparing_sui_send: bool,
    is_preparing_aptos_send: bool,
    is_preparing_ton_send: bool,
    is_preparing_icp_send: bool,
    is_preparing_near_send: bool,
    is_preparing_polkadot_send: bool,

    // Stage 4 Date? as Option<f64> (seconds since 1970)
    bitcoin_rescan_last_run_at: Option<f64>,
    bitcoin_cash_rescan_last_run_at: Option<f64>,
    bitcoin_sv_rescan_last_run_at: Option<f64>,
    litecoin_rescan_last_run_at: Option<f64>,
    dogecoin_rescan_last_run_at: Option<f64>,
    last_pending_transaction_refresh_at: Option<f64>,
    tron_last_send_error_at: Option<f64>,

    // Stage 4 Option<String> / Vec<String>
    editing_wallet_id: Option<String>,
    pending_high_risk_send_reasons: Vec<String>,

    // Option<String>
    import_error: Option<String>,
    send_error: Option<String>,
    send_destination_risk_warning: Option<String>,
    send_destination_info_message: Option<String>,
    send_verification_notice: Option<String>,
    app_lock_error: Option<String>,
    tron_last_send_error_details: Option<String>,
    fiat_rates_refresh_error: Option<String>,
    quote_refresh_error: Option<String>,
}

impl Inner {
    fn seeded() -> Self {
        Self {
            send_enable_rbf: true,
            use_face_id: true,
            require_biometric_for_send_actions: true,
            use_price_alerts: true,
            use_transaction_status_notifications: true,
            use_large_movement_notifications: true,
            bitcoin_stop_gap: 10,
            automatic_refresh_frequency_minutes: 5,
            large_movement_alert_percent_threshold: 10.0,
            large_movement_alert_usd_threshold: 50.0,
            ..Default::default()
        }
    }
}

#[derive(uniffi::Object)]
pub struct AppShellState {
    inner: Mutex<Inner>,
}

#[uniffi::export]
impl AppShellState {
    #[uniffi::constructor]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(Inner::seeded()),
        })
    }
}

macro_rules! scalar_getset {
    ($ty:ty, $field:ident, $get:ident, $set:ident) => {
        #[uniffi::export]
        impl AppShellState {
            pub fn $get(&self) -> $ty {
                self.inner.lock().unwrap().$field.clone()
            }
            pub fn $set(&self, value: $ty) {
                self.inner.lock().unwrap().$field = value;
            }
        }
    };
}

scalar_getset!(bool, is_importing_wallet, get_is_importing_wallet, set_is_importing_wallet);
scalar_getset!(bool, is_showing_wallet_importer, get_is_showing_wallet_importer, set_is_showing_wallet_importer);
scalar_getset!(bool, is_showing_send_sheet, get_is_showing_send_sheet, set_is_showing_send_sheet);
scalar_getset!(bool, is_showing_receive_sheet, get_is_showing_receive_sheet, set_is_showing_receive_sheet);
scalar_getset!(bool, is_resolving_receive_address, get_is_resolving_receive_address, set_is_resolving_receive_address);
scalar_getset!(bool, is_checking_send_destination_balance, get_is_checking_send_destination_balance, set_is_checking_send_destination_balance);
scalar_getset!(bool, is_showing_high_risk_send_confirmation, get_is_showing_high_risk_send_confirmation, set_is_showing_high_risk_send_confirmation);
scalar_getset!(bool, send_verification_notice_is_warning, get_send_verification_notice_is_warning, set_send_verification_notice_is_warning);
scalar_getset!(bool, is_app_locked, get_is_app_locked, set_is_app_locked);
scalar_getset!(bool, is_user_initiated_refresh_in_progress, get_is_user_initiated_refresh_in_progress, set_is_user_initiated_refresh_in_progress);
scalar_getset!(bool, is_loading_more_on_chain_history, get_is_loading_more_on_chain_history, set_is_loading_more_on_chain_history);
scalar_getset!(bool, is_running_bitcoin_rescan, get_is_running_bitcoin_rescan, set_is_running_bitcoin_rescan);
scalar_getset!(bool, is_running_bitcoin_cash_rescan, get_is_running_bitcoin_cash_rescan, set_is_running_bitcoin_cash_rescan);
scalar_getset!(bool, is_running_bitcoin_sv_rescan, get_is_running_bitcoin_sv_rescan, set_is_running_bitcoin_sv_rescan);
scalar_getset!(bool, is_running_litecoin_rescan, get_is_running_litecoin_rescan, set_is_running_litecoin_rescan);
scalar_getset!(bool, is_running_dogecoin_rescan, get_is_running_dogecoin_rescan, set_is_running_dogecoin_rescan);
scalar_getset!(bool, is_preparing_ethereum_replacement_context, get_is_preparing_ethereum_replacement_context, set_is_preparing_ethereum_replacement_context);
scalar_getset!(bool, use_custom_ethereum_fees, get_use_custom_ethereum_fees, set_use_custom_ethereum_fees);
scalar_getset!(bool, send_advanced_mode, get_send_advanced_mode, set_send_advanced_mode);
scalar_getset!(bool, send_enable_rbf, get_send_enable_rbf, set_send_enable_rbf);
scalar_getset!(bool, send_enable_cpfp, get_send_enable_cpfp, set_send_enable_cpfp);
scalar_getset!(bool, ethereum_manual_nonce_enabled, get_ethereum_manual_nonce_enabled, set_ethereum_manual_nonce_enabled);
scalar_getset!(bool, is_sending_bitcoin, get_is_sending_bitcoin, set_is_sending_bitcoin);
scalar_getset!(bool, is_sending_bitcoin_cash, get_is_sending_bitcoin_cash, set_is_sending_bitcoin_cash);
scalar_getset!(bool, is_sending_bitcoin_sv, get_is_sending_bitcoin_sv, set_is_sending_bitcoin_sv);
scalar_getset!(bool, is_sending_litecoin, get_is_sending_litecoin, set_is_sending_litecoin);
scalar_getset!(bool, is_sending_dogecoin, get_is_sending_dogecoin, set_is_sending_dogecoin);
scalar_getset!(bool, is_sending_ethereum, get_is_sending_ethereum, set_is_sending_ethereum);
scalar_getset!(bool, is_sending_tron, get_is_sending_tron, set_is_sending_tron);
scalar_getset!(bool, is_sending_solana, get_is_sending_solana, set_is_sending_solana);
scalar_getset!(bool, is_sending_xrp, get_is_sending_xrp, set_is_sending_xrp);
scalar_getset!(bool, is_sending_stellar, get_is_sending_stellar, set_is_sending_stellar);
scalar_getset!(bool, is_sending_monero, get_is_sending_monero, set_is_sending_monero);
scalar_getset!(bool, is_sending_cardano, get_is_sending_cardano, set_is_sending_cardano);
scalar_getset!(bool, is_sending_sui, get_is_sending_sui, set_is_sending_sui);
scalar_getset!(bool, is_sending_aptos, get_is_sending_aptos, set_is_sending_aptos);
scalar_getset!(bool, is_sending_ton, get_is_sending_ton, set_is_sending_ton);
scalar_getset!(bool, is_sending_icp, get_is_sending_icp, set_is_sending_icp);
scalar_getset!(bool, is_sending_near, get_is_sending_near, set_is_sending_near);
scalar_getset!(bool, is_sending_polkadot, get_is_sending_polkadot, set_is_sending_polkadot);

scalar_getset!(i64, send_utxo_max_input_count, get_send_utxo_max_input_count, set_send_utxo_max_input_count);
scalar_getset!(i64, bitcoin_stop_gap, get_bitcoin_stop_gap, set_bitcoin_stop_gap);
scalar_getset!(i64, automatic_refresh_frequency_minutes, get_automatic_refresh_frequency_minutes, set_automatic_refresh_frequency_minutes);

scalar_getset!(f64, large_movement_alert_percent_threshold, get_large_movement_alert_percent_threshold, set_large_movement_alert_percent_threshold);
scalar_getset!(f64, large_movement_alert_usd_threshold, get_large_movement_alert_usd_threshold, set_large_movement_alert_usd_threshold);

scalar_getset!(bool, hide_balances, get_hide_balances, set_hide_balances);
scalar_getset!(bool, use_face_id, get_use_face_id, set_use_face_id);
scalar_getset!(bool, use_auto_lock, get_use_auto_lock, set_use_auto_lock);
scalar_getset!(bool, use_strict_rpc_only, get_use_strict_rpc_only, set_use_strict_rpc_only);
scalar_getset!(bool, require_biometric_for_send_actions, get_require_biometric_for_send_actions, set_require_biometric_for_send_actions);
scalar_getset!(bool, use_price_alerts, get_use_price_alerts, set_use_price_alerts);
scalar_getset!(bool, use_transaction_status_notifications, get_use_transaction_status_notifications, set_use_transaction_status_notifications);
scalar_getset!(bool, use_large_movement_notifications, get_use_large_movement_notifications, set_use_large_movement_notifications);

scalar_getset!(String, pricing_provider, get_pricing_provider, set_pricing_provider);
scalar_getset!(String, selected_fiat_currency, get_selected_fiat_currency, set_selected_fiat_currency);
scalar_getset!(String, fiat_rate_provider, get_fiat_rate_provider, set_fiat_rate_provider);
scalar_getset!(String, ethereum_network_mode, get_ethereum_network_mode, set_ethereum_network_mode);
scalar_getset!(String, bitcoin_network_mode, get_bitcoin_network_mode, set_bitcoin_network_mode);
scalar_getset!(String, dogecoin_network_mode, get_dogecoin_network_mode, set_dogecoin_network_mode);
scalar_getset!(String, bitcoin_fee_priority, get_bitcoin_fee_priority, set_bitcoin_fee_priority);
scalar_getset!(String, dogecoin_fee_priority, get_dogecoin_fee_priority, set_dogecoin_fee_priority);
scalar_getset!(String, background_sync_profile, get_background_sync_profile, set_background_sync_profile);
scalar_getset!(String, send_litecoin_change_strategy, get_send_litecoin_change_strategy, set_send_litecoin_change_strategy);

scalar_getset!(String, coin_gecko_api_key, get_coin_gecko_api_key, set_coin_gecko_api_key);
scalar_getset!(String, ethereum_rpc_endpoint, get_ethereum_rpc_endpoint, set_ethereum_rpc_endpoint);
scalar_getset!(String, etherscan_api_key, get_etherscan_api_key, set_etherscan_api_key);
scalar_getset!(String, monero_backend_base_url, get_monero_backend_base_url, set_monero_backend_base_url);
scalar_getset!(String, monero_backend_api_key, get_monero_backend_api_key, set_monero_backend_api_key);
scalar_getset!(String, bitcoin_esplora_endpoints, get_bitcoin_esplora_endpoints, set_bitcoin_esplora_endpoints);

scalar_getset!(HashMap<String, f64>, live_prices, get_live_prices, set_live_prices);
scalar_getset!(HashMap<String, f64>, fiat_rates_from_usd, get_fiat_rates_from_usd, set_fiat_rates_from_usd);
scalar_getset!(HashMap<String, i64>, asset_display_decimals_by_chain, get_asset_display_decimals_by_chain, set_asset_display_decimals_by_chain);
scalar_getset!(HashMap<String, String>, selected_fee_priority_option_raw_by_chain, get_selected_fee_priority_option_raw_by_chain, set_selected_fee_priority_option_raw_by_chain);

scalar_getset!(bool, is_preparing_ethereum_send, get_is_preparing_ethereum_send, set_is_preparing_ethereum_send);
scalar_getset!(bool, is_preparing_dogecoin_send, get_is_preparing_dogecoin_send, set_is_preparing_dogecoin_send);
scalar_getset!(bool, is_preparing_tron_send, get_is_preparing_tron_send, set_is_preparing_tron_send);
scalar_getset!(bool, is_preparing_solana_send, get_is_preparing_solana_send, set_is_preparing_solana_send);
scalar_getset!(bool, is_preparing_xrp_send, get_is_preparing_xrp_send, set_is_preparing_xrp_send);
scalar_getset!(bool, is_preparing_stellar_send, get_is_preparing_stellar_send, set_is_preparing_stellar_send);
scalar_getset!(bool, is_preparing_monero_send, get_is_preparing_monero_send, set_is_preparing_monero_send);
scalar_getset!(bool, is_preparing_cardano_send, get_is_preparing_cardano_send, set_is_preparing_cardano_send);
scalar_getset!(bool, is_preparing_sui_send, get_is_preparing_sui_send, set_is_preparing_sui_send);
scalar_getset!(bool, is_preparing_aptos_send, get_is_preparing_aptos_send, set_is_preparing_aptos_send);
scalar_getset!(bool, is_preparing_ton_send, get_is_preparing_ton_send, set_is_preparing_ton_send);
scalar_getset!(bool, is_preparing_icp_send, get_is_preparing_icp_send, set_is_preparing_icp_send);
scalar_getset!(bool, is_preparing_near_send, get_is_preparing_near_send, set_is_preparing_near_send);
scalar_getset!(bool, is_preparing_polkadot_send, get_is_preparing_polkadot_send, set_is_preparing_polkadot_send);

scalar_getset!(Option<f64>, bitcoin_rescan_last_run_at, get_bitcoin_rescan_last_run_at, set_bitcoin_rescan_last_run_at);
scalar_getset!(Option<f64>, bitcoin_cash_rescan_last_run_at, get_bitcoin_cash_rescan_last_run_at, set_bitcoin_cash_rescan_last_run_at);
scalar_getset!(Option<f64>, bitcoin_sv_rescan_last_run_at, get_bitcoin_sv_rescan_last_run_at, set_bitcoin_sv_rescan_last_run_at);
scalar_getset!(Option<f64>, litecoin_rescan_last_run_at, get_litecoin_rescan_last_run_at, set_litecoin_rescan_last_run_at);
scalar_getset!(Option<f64>, dogecoin_rescan_last_run_at, get_dogecoin_rescan_last_run_at, set_dogecoin_rescan_last_run_at);
scalar_getset!(Option<f64>, last_pending_transaction_refresh_at, get_last_pending_transaction_refresh_at, set_last_pending_transaction_refresh_at);
scalar_getset!(Option<f64>, tron_last_send_error_at, get_tron_last_send_error_at, set_tron_last_send_error_at);

scalar_getset!(Option<String>, editing_wallet_id, get_editing_wallet_id, set_editing_wallet_id);
scalar_getset!(Vec<String>, pending_high_risk_send_reasons, get_pending_high_risk_send_reasons, set_pending_high_risk_send_reasons);

scalar_getset!(String, send_wallet_id, get_send_wallet_id, set_send_wallet_id);
scalar_getset!(String, send_holding_key, get_send_holding_key, set_send_holding_key);
scalar_getset!(String, send_amount, get_send_amount, set_send_amount);
scalar_getset!(String, send_address, get_send_address, set_send_address);
scalar_getset!(String, receive_wallet_id, get_receive_wallet_id, set_receive_wallet_id);
scalar_getset!(String, receive_chain_name, get_receive_chain_name, set_receive_chain_name);
scalar_getset!(String, receive_holding_key, get_receive_holding_key, set_receive_holding_key);
scalar_getset!(String, receive_resolved_address, get_receive_resolved_address, set_receive_resolved_address);
scalar_getset!(String, custom_ethereum_max_fee_gwei, get_custom_ethereum_max_fee_gwei, set_custom_ethereum_max_fee_gwei);
scalar_getset!(String, custom_ethereum_priority_fee_gwei, get_custom_ethereum_priority_fee_gwei, set_custom_ethereum_priority_fee_gwei);
scalar_getset!(String, ethereum_manual_nonce, get_ethereum_manual_nonce, set_ethereum_manual_nonce);

scalar_getset!(Option<String>, import_error, get_import_error, set_import_error);
scalar_getset!(Option<String>, send_error, get_send_error, set_send_error);
scalar_getset!(Option<String>, send_destination_risk_warning, get_send_destination_risk_warning, set_send_destination_risk_warning);
scalar_getset!(Option<String>, send_destination_info_message, get_send_destination_info_message, set_send_destination_info_message);
scalar_getset!(Option<String>, send_verification_notice, get_send_verification_notice, set_send_verification_notice);
scalar_getset!(Option<String>, app_lock_error, get_app_lock_error, set_app_lock_error);
scalar_getset!(Option<String>, tron_last_send_error_details, get_tron_last_send_error_details, set_tron_last_send_error_details);
scalar_getset!(Option<String>, fiat_rates_refresh_error, get_fiat_rates_refresh_error, set_fiat_rates_refresh_error);
scalar_getset!(Option<String>, quote_refresh_error, get_quote_refresh_error, set_quote_refresh_error);

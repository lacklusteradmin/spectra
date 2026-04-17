uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SpectraBridgeError {
    #[error("{message}")]
    Failure { message: String },
}

impl From<String> for SpectraBridgeError {
    fn from(message: String) -> Self {
        Self::Failure { message }
    }
}

impl From<&str> for SpectraBridgeError {
    fn from(message: &str) -> Self {
        Self::Failure {
            message: message.to_string(),
        }
    }
}

impl From<serde_json::Error> for SpectraBridgeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Failure {
            message: error.to_string(),
        }
    }
}

impl From<hex::FromHexError> for SpectraBridgeError {
    fn from(error: hex::FromHexError) -> Self {
        Self::Failure {
            message: error.to_string(),
        }
    }
}

mod app_core;
pub use app_core::*;

pub mod chains;
pub mod derivation;
pub mod diagnostics;
pub mod diagnostics_sanitizer;
pub mod fetch;
pub mod ffi;
pub mod formatting;
pub mod localization;
pub mod receive;
pub mod resources;
pub mod self_tests;
pub mod send;
pub mod service;
pub mod store;
pub mod tokens;
pub mod types;
pub mod catalog;

// Re-export every submodule at crate root so pre-reorganization paths
// (e.g. `crate::http::HttpClient`, `crate::wallet_domain::...`) continue to resolve.
// New code should prefer the folder-qualified path (`crate::fetch::http::...`).
pub use derivation::*;
pub use derivation::{addressing, import};
pub use fetch::{
    balance_cache, balance_decoder, balance_observer, endpoint_reliability, history,
    history_cache, history_decode, history_store, http, http_ffi, price, refresh, refresh_engine,
    transactions,
};
pub use send::{
    amount_input, ethereum as ethereum_send, flow as send_flow, flow_helpers as send_flow_helpers,
    machine as send_machine, payload as send_payload, preview_decode as send_preview_decode,
    transfer, utxo, verification as send_verification,
};
pub use store::{
    app_shell_state, app_state, password_verifier, persistence, secret_store,
    seed_envelope, state, wallet_core, wallet_db, wallet_domain,
};

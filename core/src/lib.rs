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

pub mod derivation;
pub mod diagnostics;
pub mod fetch;
pub mod ffi;
pub mod platform;
pub mod receive;
pub mod registry;
pub mod send;
pub mod service;
pub mod store;

// Compat re-exports so pre-reorganization paths (e.g. `crate::http::HttpClient`,
// `crate::catalog::...`, `crate::tokens::...`) continue to resolve without
// touching call sites. New code should prefer folder-qualified paths.
pub use derivation::*;
pub use derivation::{addressing, import};
pub use fetch::{
    balance_observer, history, history_decode,
    history_store, http, http_ffi, price, refresh, refresh_engine, transactions,
};
pub use send::{
    amount_input, ethereum as ethereum_send, flow as send_flow, flow_helpers as send_flow_helpers,
    payload as send_payload, preview_decode as send_preview_decode,
    transfer, utxo, verification as send_verification,
};
pub use store::{
    app_state, password_verifier, persistence, secret_store,
    seed_envelope, state, wallet_core, wallet_db, wallet_domain,
};
// Platform helpers (catalog/formatting/localization/resources/types) were at
// crate root before reorg — keep them reachable via their old names.
pub use platform::{catalog, formatting, localization, resources, types};
// tokens.rs moved under registry/; diagnostics_sanitizer.rs and self_tests.rs
// moved under diagnostics/. Expose at crate root for existing callers.
pub use registry::tokens;
pub use diagnostics::{sanitizer as diagnostics_sanitizer, self_tests};

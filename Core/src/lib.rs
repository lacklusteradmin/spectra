use serde::{Deserialize, Serialize};

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

// Preset-focused API surface for Rust-side derivation request compilation data.
// Swift currently owns runtime preset selection, but this keeps Rust ready to
// own or validate preset payloads without mixing them with execution internals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivationRequestPreset {
    pub chain: String,
    pub derivation_algorithm: String,
    pub address_algorithm: String,
    pub public_key_format: String,
    pub script_policy: String,
    pub fixed_script_type: Option<String>,
    pub bitcoin_purpose_script_map: Option<std::collections::BTreeMap<String, String>>,
}

pub fn parse_derivation_request_presets_json(
    json: &str,
) -> Result<Vec<DerivationRequestPreset>, serde_json::Error> {
    serde_json::from_str(json)
}

#[path = "main.rs"]
mod derivation_runtime;

pub use derivation_runtime::*;

mod app_core;

pub use app_core::*;

pub mod addressing;
pub mod balance_cache;
pub mod balance_observer;
pub mod catalog;
pub mod chains;
pub mod diagnostics;
pub mod diagnostics_sanitizer;
pub mod endpoint_reliability;
pub mod fetch;
pub mod ffi;
pub mod formatting;
pub mod history;
pub mod history_cache;
pub mod history_decode;
pub mod history_store;
pub mod http;
pub mod import;
pub mod localization;
pub mod migration;
pub mod persistence;
pub mod price;
pub mod refresh;
pub mod refresh_engine;
pub mod resources;
pub mod secret_store;
pub mod self_tests;
pub mod send;
pub mod send_machine;
pub mod service;
pub mod state;
pub mod store;
pub mod tokens;
pub mod transactions;
pub mod transfer;
pub mod types;
pub mod utxo;
pub mod ethereum_send;
pub mod send_preview_decode;
pub mod send_payload;
pub mod send_flow;
pub mod send_flow_helpers;
pub mod send_verification;
pub mod amount_input;
pub mod app_shell_state;
pub mod app_state;
pub mod http_ffi;
pub mod wallet_core;
pub mod wallet_db;
pub mod wallet_domain;
pub mod receive;
pub mod balance_decoder;

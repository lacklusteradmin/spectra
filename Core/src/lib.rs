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

pub mod core;

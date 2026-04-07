use serde::{Deserialize, Serialize};

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

//! Chain derivation presets, loaded from `core/derivation_presets.toml` at
//! compile time. Single source of truth for per-chain defaults: curve,
//! derivation algorithm, address algorithm, public key format, script type,
//! and which networks each chain supports.
//!
//! Replaces three hardcoded match statements that used to live in
//! `runtime.rs` (`chain_defaults_from_name`, `is_secp_chain`,
//! `is_network_supported`). Adding a new chain preset is now a TOML edit.

use super::runtime::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

static PRESETS_TOML: &str = include_str!("../../derivation_presets.toml");

#[derive(Debug, Deserialize)]
struct PresetsFile {
    chains: Vec<RawPreset>,
}

#[derive(Debug, Deserialize)]
struct RawPreset {
    name: String,
    chain_id: u32,
    #[serde(default)]
    aliases: Vec<String>,
    curve: String,
    derivation_algorithm: String,
    address_algorithm: String,
    public_key_format: String,
    #[serde(default)]
    script_type: Option<String>,
    networks: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ChainPreset {
    pub chain_id: u32,
    pub curve: u32,
    pub derivation_algorithm: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    /// `None` means the caller must infer the script type from the path's
    /// purpose level (only Bitcoin does this today).
    pub script_type: Option<u32>,
    pub networks: Vec<u32>,
}

static PRESETS_BY_NAME: LazyLock<HashMap<String, ChainPreset>> = LazyLock::new(|| {
    let file = load_presets_file();
    let mut map = HashMap::new();
    for raw in &file.chains {
        let preset = parse_preset(raw);
        map.insert(raw.name.clone(), preset.clone());
        for alias in &raw.aliases {
            map.insert(alias.clone(), preset.clone());
        }
    }
    map
});

static PRESETS_BY_CHAIN_ID: LazyLock<HashMap<u32, ChainPreset>> = LazyLock::new(|| {
    let file = load_presets_file();
    let mut map = HashMap::new();
    for raw in &file.chains {
        // Aliases share chain_id with their canonical entry; keep the first
        // one encountered so EVM aliases don't overwrite the Ethereum entry.
        map.entry(raw.chain_id).or_insert_with(|| parse_preset(raw));
    }
    map
});

fn load_presets_file() -> PresetsFile {
    toml::from_str(PRESETS_TOML)
        .expect("derivation_presets.toml is embedded at compile time and must be valid TOML")
}

fn parse_preset(raw: &RawPreset) -> ChainPreset {
    let script_type = raw
        .script_type
        .as_deref()
        .map(|s| parse_script_type_name(s).unwrap_or_else(|e| panic!("{}: {}", raw.name, e)));
    let networks = raw
        .networks
        .iter()
        .map(|n| parse_network_name(n).unwrap_or_else(|e| panic!("{}: {}", raw.name, e)))
        .collect();
    ChainPreset {
        chain_id: raw.chain_id,
        curve: parse_curve_name(&raw.curve).unwrap_or_else(|e| panic!("{}: {}", raw.name, e)),
        derivation_algorithm: parse_derivation_algorithm_name(&raw.derivation_algorithm)
            .unwrap_or_else(|e| panic!("{}: {}", raw.name, e)),
        address_algorithm: parse_address_algorithm_name(&raw.address_algorithm)
            .unwrap_or_else(|e| panic!("{}: {}", raw.name, e)),
        public_key_format: parse_public_key_format_name(&raw.public_key_format)
            .unwrap_or_else(|e| panic!("{}: {}", raw.name, e)),
        script_type,
        networks,
    }
}

fn parse_curve_name(s: &str) -> Result<u32, String> {
    match s {
        "secp256k1" => Ok(CURVE_SECP256K1),
        "ed25519" => Ok(CURVE_ED25519),
        "sr25519" => Ok(CURVE_SR25519),
        other => Err(format!("unknown curve: {other}")),
    }
}

fn parse_derivation_algorithm_name(s: &str) -> Result<u32, String> {
    match s {
        "bip32_secp256k1" => Ok(DERIVATION_BIP32_SECP256K1),
        "slip10_ed25519" => Ok(DERIVATION_SLIP10_ED25519),
        "direct_seed_ed25519" => Ok(DERIVATION_DIRECT_SEED_ED25519),
        "ton_mnemonic" => Ok(DERIVATION_TON_MNEMONIC),
        "bip32_ed25519_icarus" => Ok(DERIVATION_BIP32_ED25519_ICARUS),
        "substrate_bip39" => Ok(DERIVATION_SUBSTRATE_BIP39),
        "monero_bip39" => Ok(DERIVATION_MONERO_BIP39),
        other => Err(format!("unknown derivation algorithm: {other}")),
    }
}

fn parse_address_algorithm_name(s: &str) -> Result<u32, String> {
    match s {
        "bitcoin" => Ok(ADDRESS_BITCOIN),
        "evm" => Ok(ADDRESS_EVM),
        "solana" => Ok(ADDRESS_SOLANA),
        "near_hex" => Ok(ADDRESS_NEAR_HEX),
        "ton_raw_account_id" => Ok(ADDRESS_TON_RAW_ACCOUNT_ID),
        "cardano_shelley_enterprise" => Ok(ADDRESS_CARDANO_SHELLEY_ENTERPRISE),
        "ss58" => Ok(ADDRESS_SS58),
        "monero_main" => Ok(ADDRESS_MONERO_MAIN),
        "ton_v4r2" => Ok(ADDRESS_TON_V4R2),
        "litecoin" => Ok(ADDRESS_LITECOIN),
        "dogecoin" => Ok(ADDRESS_DOGECOIN),
        "bitcoin_cash_legacy" => Ok(ADDRESS_BITCOIN_CASH_LEGACY),
        "bitcoin_sv_legacy" => Ok(ADDRESS_BITCOIN_SV_LEGACY),
        "tron_base58_check" => Ok(ADDRESS_TRON_BASE58_CHECK),
        "xrp_base58_check" => Ok(ADDRESS_XRP_BASE58_CHECK),
        "stellar_strkey" => Ok(ADDRESS_STELLAR_STRKEY),
        "sui_keccak" => Ok(ADDRESS_SUI_KECCAK),
        "aptos_keccak" => Ok(ADDRESS_APTOS_KECCAK),
        "icp_principal" => Ok(ADDRESS_ICP_PRINCIPAL),
        other => Err(format!("unknown address algorithm: {other}")),
    }
}

fn parse_public_key_format_name(s: &str) -> Result<u32, String> {
    match s {
        "compressed" => Ok(PUBLIC_KEY_COMPRESSED),
        "uncompressed" => Ok(PUBLIC_KEY_UNCOMPRESSED),
        "raw" => Ok(PUBLIC_KEY_RAW),
        other => Err(format!("unknown public key format: {other}")),
    }
}

fn parse_script_type_name(s: &str) -> Result<u32, String> {
    match s {
        "p2pkh" => Ok(SCRIPT_P2PKH),
        "p2sh_p2wpkh" => Ok(SCRIPT_P2SH_P2WPKH),
        "p2wpkh" => Ok(SCRIPT_P2WPKH),
        "p2tr" => Ok(SCRIPT_P2TR),
        "account" => Ok(SCRIPT_ACCOUNT),
        other => Err(format!("unknown script type: {other}")),
    }
}

fn parse_network_name(s: &str) -> Result<u32, String> {
    match s {
        "mainnet" => Ok(NETWORK_MAINNET),
        "testnet" => Ok(NETWORK_TESTNET),
        "testnet4" => Ok(NETWORK_TESTNET4),
        "signet" => Ok(NETWORK_SIGNET),
        other => Err(format!("unknown network: {other}")),
    }
}

pub(super) fn preset_by_name(name: &str) -> Option<&'static ChainPreset> {
    PRESETS_BY_NAME.get(name)
}

pub(super) fn preset_by_chain_id(chain_id: u32) -> Option<&'static ChainPreset> {
    PRESETS_BY_CHAIN_ID.get(&chain_id)
}

// ──────────────────────────────────────────────────────────────────────────
// Public wire-value parsers, used by override handling in
// `derive_key_material_for_chain_with_overrides`. Accept the same string
// names as `derivation_presets.toml` and produce the u32 wire values the
// derivation request expects.
// ──────────────────────────────────────────────────────────────────────────

pub(crate) fn curve_wire_value(name: &str) -> Result<u32, String> {
    parse_curve_name(name)
}

pub(crate) fn derivation_algorithm_wire_value(name: &str) -> Result<u32, String> {
    parse_derivation_algorithm_name(name)
}

pub(crate) fn address_algorithm_wire_value(name: &str) -> Result<u32, String> {
    parse_address_algorithm_name(name)
}

pub(crate) fn public_key_format_wire_value(name: &str) -> Result<u32, String> {
    parse_public_key_format_name(name)
}

pub(crate) fn script_type_wire_value(name: &str) -> Result<u32, String> {
    parse_script_type_name(name)
}

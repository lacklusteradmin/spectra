// Pure logic lifts from Swift AppState+SendFlow.swift.
// No IO, no SwiftUI, no Keychain — just mappings, validators, and small parsers.

use crate::SpectraBridgeError;

// ─── EVM chain context string mapping ────────────────────────────────────────
// Returns a tag like "ethereum", "ethereum_sepolia", "ethereum_hoodi",
// "ethereum_classic", "arbitrum", "optimism", "bnb", "avalanche", "hyperliquid",
// or empty string for non-EVM.

#[uniffi::export]
pub fn core_evm_chain_context_tag(chain_name: String, ethereum_network_mode: String) -> String {
    match chain_name.as_str() {
        "Ethereum" => match ethereum_network_mode.as_str() {
            "sepolia" => "ethereum_sepolia".to_string(),
            "hoodi" => "ethereum_hoodi".to_string(),
            _ => "ethereum".to_string(),
        },
        "Ethereum Classic" => "ethereum_classic".to_string(),
        "Arbitrum" => "arbitrum".to_string(),
        "Optimism" => "optimism".to_string(),
        "BNB Chain" => "bnb".to_string(),
        "Avalanche" => "avalanche".to_string(),
        "Hyperliquid" => "hyperliquid".to_string(),
        _ => String::new(),
    }
}

#[uniffi::export]
pub fn core_is_evm_chain(chain_name: String) -> bool {
    !core_evm_chain_context_tag(chain_name, "mainnet".to_string()).is_empty()
}

// ─── Dogecoin derivation index parser ─────────────────────────────────────────

#[uniffi::export]
pub fn core_parse_dogecoin_derivation_index(path: Option<String>, expected_prefix: String) -> Option<i32> {
    let path = path?;
    if !path.starts_with(&expected_prefix) {
        return None;
    }
    let suffix = &path[expected_prefix.len()..];
    suffix.parse::<i32>().ok()
}

// ─── Simple chain risk probe config ──────────────────────────────────────────
// Per-chain static config for the Litecoin/Dogecoin/Solana/XRP/Monero/Sui/Aptos
// branch of Swift's destination-risk probe: display chain name and balance
// label for messages.

#[derive(Debug, Clone, uniffi::Record)]
pub struct SimpleChainRiskProbeConfig {
    pub display_chain_name: String,
    pub balance_label: String,
}

#[uniffi::export]
pub fn core_simple_chain_risk_probe_config(
    chain_name: String,
    symbol: String,
) -> Option<SimpleChainRiskProbeConfig> {
    let (display_chain_name, balance_label) = match chain_name.as_str() {
        "Litecoin" => ("Litecoin", "balance"),
        "Dogecoin" if symbol == "DOGE" => ("Dogecoin", "balance"),
        "Solana" => ("Solana", "SOL balance"),
        "XRP Ledger" => ("XRP", "XRP balance"),
        "Monero" => ("Monero", "XMR balance"),
        "Sui" => ("Sui", "SUI balance"),
        "Aptos" => ("Aptos", "APT balance"),
        _ => return None,
    };
    Some(SimpleChainRiskProbeConfig {
        display_chain_name: display_chain_name.to_string(),
        balance_label: balance_label.to_string(),
    })
}

// ─── Broadcast rebroadcast dispatch table ─────────────────────────────────────
// Maps Swift's BroadcastEntry payload format → (chain_id, result_field, wrap_key,
// extract_field). Returns an error for unknown formats.

#[derive(Debug, Clone, uniffi::Record)]
pub struct RebroadcastDispatch {
    pub chain_id: u32,
    pub result_field: String,
    pub wrap_key: Option<String>,
    pub extract_field: Option<String>,
}

#[uniffi::export]
pub fn core_rebroadcast_dispatch_for_format(
    format: String,
) -> Result<RebroadcastDispatch, SpectraBridgeError> {
    // Keep chain IDs aligned with SpectraChainID in Swift.
    // 0 bitcoin, 1 bitcoin_cash, 2 bitcoin_sv, 3 litecoin, 4 dogecoin,
    // 5 ethereum, 6 tron, 7 solana, 8 xrp, 9 stellar, 10 monero,
    // 11 cardano, 12 sui, 13 aptos, 14 ton, 15 icp, 16 near, 17 polkadot
    let entry: Option<RebroadcastDispatch> = match format.as_str() {
        "bitcoin.raw_hex" => Some(RebroadcastDispatch { chain_id: 0, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "bitcoin_cash.raw_hex" => Some(RebroadcastDispatch { chain_id: 1, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "bitcoin_sv.raw_hex" => Some(RebroadcastDispatch { chain_id: 2, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "litecoin.raw_hex" => Some(RebroadcastDispatch { chain_id: 3, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "dogecoin.raw_hex" => Some(RebroadcastDispatch { chain_id: 4, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "tron.signed_json" => Some(RebroadcastDispatch { chain_id: 6, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "solana.base64" => Some(RebroadcastDispatch { chain_id: 7, result_field: "signature".into(), wrap_key: None, extract_field: None }),
        "xrp.blob_hex" => Some(RebroadcastDispatch { chain_id: 8, result_field: "txid".into(), wrap_key: Some("tx_blob_hex".into()), extract_field: None }),
        "stellar.xdr" => Some(RebroadcastDispatch { chain_id: 9, result_field: "txid".into(), wrap_key: Some("signed_xdr_b64".into()), extract_field: None }),
        "cardano.cbor_hex" => Some(RebroadcastDispatch { chain_id: 11, result_field: "txid".into(), wrap_key: Some("cbor_hex".into()), extract_field: None }),
        "near.base64" => Some(RebroadcastDispatch { chain_id: 16, result_field: "txid".into(), wrap_key: Some("signed_tx_b64".into()), extract_field: None }),
        "polkadot.extrinsic_hex" => Some(RebroadcastDispatch { chain_id: 17, result_field: "txid".into(), wrap_key: Some("extrinsic_hex".into()), extract_field: None }),
        "aptos.signed_json" => Some(RebroadcastDispatch { chain_id: 13, result_field: "txid".into(), wrap_key: Some("signed_body_json".into()), extract_field: None }),
        "ton.boc" => Some(RebroadcastDispatch { chain_id: 14, result_field: "message_hash".into(), wrap_key: Some("boc_b64".into()), extract_field: None }),
        "bitcoin.rust_json" => Some(RebroadcastDispatch { chain_id: 0, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "bitcoin_cash.rust_json" => Some(RebroadcastDispatch { chain_id: 1, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "bitcoin_sv.rust_json" => Some(RebroadcastDispatch { chain_id: 2, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "litecoin.rust_json" => Some(RebroadcastDispatch { chain_id: 3, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "dogecoin.rust_json" => Some(RebroadcastDispatch { chain_id: 4, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "solana.rust_json" => Some(RebroadcastDispatch { chain_id: 7, result_field: "signature".into(), wrap_key: None, extract_field: Some("signed_tx_base64".into()) }),
        "tron.rust_json" => Some(RebroadcastDispatch { chain_id: 6, result_field: "txid".into(), wrap_key: None, extract_field: Some("signed_tx_json".into()) }),
        "xrp.rust_json" => Some(RebroadcastDispatch { chain_id: 8, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "stellar.rust_json" => Some(RebroadcastDispatch { chain_id: 9, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "cardano.rust_json" => Some(RebroadcastDispatch { chain_id: 11, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "polkadot.rust_json" => Some(RebroadcastDispatch { chain_id: 17, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "sui.rust_json" => Some(RebroadcastDispatch { chain_id: 12, result_field: "digest".into(), wrap_key: None, extract_field: None }),
        "aptos.rust_json" => Some(RebroadcastDispatch { chain_id: 13, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "ton.rust_json" => Some(RebroadcastDispatch { chain_id: 14, result_field: "message_hash".into(), wrap_key: None, extract_field: None }),
        "near.rust_json" => Some(RebroadcastDispatch { chain_id: 16, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        _ => None,
    };
    entry.ok_or_else(|| SpectraBridgeError::from("Rebroadcast is not supported for this transaction format yet."))
}

// ─── Rebroadcast prepared payload ────────────────────────────────────────────
// Fuses the dispatch-table lookup with the payload shape transformation so Swift
// never has to build JSON objects or scrape fields for rebroadcast. Handles:
//   • sui.signed_json — remap {txBytesBase64, signatureBase64} → {tx_bytes_b64, sig_b64}
//   • extract_field branch — pull named field value out of a wallet-produced JSON
//   • wrap_key branch — wrap raw payload string under a single JSON key
//   • otherwise — pass payload through unchanged

#[derive(Debug, Clone, uniffi::Record)]
pub struct PreparedBroadcastPayload {
    pub chain_id: u32,
    pub broadcast_payload: String,
    pub result_field: String,
}

#[uniffi::export]
pub fn core_rebroadcast_prepare_payload(
    format: String,
    raw_payload: String,
) -> Result<PreparedBroadcastPayload, SpectraBridgeError> {
    if format == "sui.signed_json" {
        let remapped = sui_signed_json_remap(&raw_payload).unwrap_or_else(|| raw_payload.clone());
        return Ok(PreparedBroadcastPayload {
            chain_id: 12,
            broadcast_payload: remapped,
            result_field: "digest".to_string(),
        });
    }
    let dispatch = core_rebroadcast_dispatch_for_format(format)?;
    let broadcast_payload = if let Some(extract_field) = dispatch.extract_field.as_ref() {
        crate::send::preview_decode::extract_json_string_field(raw_payload.clone(), extract_field.clone())
    } else if let Some(wrap_key) = dispatch.wrap_key.as_ref() {
        let mut map = serde_json::Map::new();
        map.insert(wrap_key.clone(), serde_json::Value::String(raw_payload.clone()));
        serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or(raw_payload)
    } else {
        raw_payload
    };
    Ok(PreparedBroadcastPayload {
        chain_id: dispatch.chain_id,
        broadcast_payload,
        result_field: dispatch.result_field,
    })
}

fn sui_signed_json_remap(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = v.as_object()?;
    let tx = obj.get("txBytesBase64")?.as_str()?;
    let sig = obj.get("signatureBase64")?.as_str()?;
    let remapped = serde_json::json!({ "tx_bytes_b64": tx, "sig_b64": sig });
    serde_json::to_string(&remapped).ok()
}

// ─── Seed derivation chain raw lookup ────────────────────────────────────────

#[uniffi::export]
pub fn core_seed_derivation_chain_raw(chain_name: String) -> Option<String> {
    let raw = match chain_name.as_str() {
        "Bitcoin" => "Bitcoin",
        "Bitcoin Cash" => "Bitcoin Cash",
        "Bitcoin SV" => "Bitcoin SV",
        "Litecoin" => "Litecoin",
        "Dogecoin" => "Dogecoin",
        "Ethereum" | "BNB Chain" => "Ethereum",
        "Ethereum Classic" => "Ethereum Classic",
        "Arbitrum" => "Arbitrum",
        "Optimism" => "Optimism",
        "Avalanche" => "Avalanche",
        "Hyperliquid" => "Hyperliquid",
        "Tron" => "Tron",
        "Solana" => "Solana",
        "Stellar" => "Stellar",
        "XRP Ledger" => "XRP Ledger",
        "Cardano" => "Cardano",
        "Sui" => "Sui",
        "Aptos" => "Aptos",
        "TON" => "TON",
        "Internet Computer" => "Internet Computer",
        "NEAR" => "NEAR",
        "Polkadot" => "Polkadot",
        _ => return None,
    };
    Some(raw.to_string())
}

#[uniffi::export]
pub fn core_supports_deep_utxo_discovery(chain_name: String) -> bool {
    crate::registry::Chain::from_display_name(&chain_name)
        .is_some_and(|c| c.supports_deep_utxo_discovery())
}

// ─── Receive address resolver dispatch ───────────────────────────────────────
// Centralizes the `(symbol, chain_name, is_evm_chain)` → resolver routing that
// `receiveAddress()` in Swift previously encoded as nested switches.

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ReceiveAddressResolverKind {
    BitcoinLegacy,
    BitcoinCash,
    BitcoinSv,
    Litecoin,
    DogecoinNone,
    Evm,
    Tron,
    Solana,
    Cardano,
    Xrp,
    Stellar,
    Monero,
    Sui,
    Aptos,
    Ton,
    Icp,
    Near,
    Polkadot,
    Zcash,
    BitcoinGold,
    Decred,
    Kaspa,
    Dash,
    Bittensor,
    None,
}

#[uniffi::export]
pub fn core_plan_receive_address_resolver(
    symbol: String,
    chain_name: String,
    is_evm_chain: bool,
) -> ReceiveAddressResolverKind {
    match (symbol.as_str(), chain_name.as_str()) {
        ("BTC", _) => ReceiveAddressResolverKind::BitcoinLegacy,
        ("BCH", "Bitcoin Cash") => ReceiveAddressResolverKind::BitcoinCash,
        ("BSV", "Bitcoin SV") => ReceiveAddressResolverKind::BitcoinSv,
        ("LTC", "Litecoin") => ReceiveAddressResolverKind::Litecoin,
        ("DOGE", "Dogecoin") => ReceiveAddressResolverKind::DogecoinNone,
        _ if is_evm_chain => ReceiveAddressResolverKind::Evm,
        (_, "Tron") => ReceiveAddressResolverKind::Tron,
        (_, "Solana") => ReceiveAddressResolverKind::Solana,
        (_, "Cardano") => ReceiveAddressResolverKind::Cardano,
        (_, "XRP Ledger") => ReceiveAddressResolverKind::Xrp,
        (_, "Stellar") => ReceiveAddressResolverKind::Stellar,
        (_, "Monero") => ReceiveAddressResolverKind::Monero,
        (_, "Sui") => ReceiveAddressResolverKind::Sui,
        (_, "Aptos") => ReceiveAddressResolverKind::Aptos,
        (_, "TON") => ReceiveAddressResolverKind::Ton,
        (_, "Internet Computer") => ReceiveAddressResolverKind::Icp,
        (_, "NEAR") => ReceiveAddressResolverKind::Near,
        (_, "Polkadot") => ReceiveAddressResolverKind::Polkadot,
        ("ZEC", "Zcash") => ReceiveAddressResolverKind::Zcash,
        ("BTG", "Bitcoin Gold") => ReceiveAddressResolverKind::BitcoinGold,
        ("DCR", "Decred") => ReceiveAddressResolverKind::Decred,
        ("KAS", "Kaspa") => ReceiveAddressResolverKind::Kaspa,
        ("DASH", "Dash") => ReceiveAddressResolverKind::Dash,
        ("TAO", "Bittensor") => ReceiveAddressResolverKind::Bittensor,
        _ => ReceiveAddressResolverKind::None,
    }
}

// ─── EVM contract-code detection ─────────────────────────────────────────────
// Lifted from Swift `evmHasContractCode`: a nonempty `eth_getCode` result
// (anything other than "0x" or "0x0") indicates deployed bytecode.

pub fn core_evm_has_contract_code(code: String) -> bool {
    let trimmed = code.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("0x")
        && !trimmed.eq_ignore_ascii_case("0x0")
}

// ─── EVM replacement fee bump calculator ─────────────────────────────────────
// When preparing a speed-up / cancel replacement, Swift bumps existing custom
// fees by 20% with a 0.1 gwei floor (or falls back to defaults if either input
// is missing / blank). Returns formatted strings (3 decimals) the way Swift
// renders them into the composer fields.

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmReplacementFeeBump {
    pub max_fee_gwei: String,
    pub priority_fee_gwei: String,
}

#[uniffi::export]
pub fn core_evm_replacement_fee_bump(
    existing_max_fee_gwei: Option<String>,
    existing_priority_fee_gwei: Option<String>,
    default_max_fee_gwei: f64,
    default_priority_fee_gwei: f64,
) -> EvmReplacementFeeBump {
    let parse = |s: Option<&str>| -> Option<f64> {
        s.and_then(|v| {
            let trimmed = v.trim();
            if trimmed.is_empty() { None } else { trimmed.parse::<f64>().ok() }
        })
    };
    let have_max = parse(existing_max_fee_gwei.as_deref());
    let have_pri = parse(existing_priority_fee_gwei.as_deref());
    if have_max.is_none() || have_pri.is_none() {
        return EvmReplacementFeeBump {
            max_fee_gwei: format!("{:.1}", default_max_fee_gwei),
            priority_fee_gwei: format!("{:.1}", default_priority_fee_gwei),
        };
    }
    let bumped_max = (have_max.unwrap() * 1.2).max(0.1);
    let bumped_pri = (have_pri.unwrap() * 1.2).max(0.1);
    EvmReplacementFeeBump {
        max_fee_gwei: format!("{:.3}", bumped_max),
        priority_fee_gwei: format!("{:.3}", bumped_pri),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_chain_context_ethereum_sepolia() {
        assert_eq!(
            core_evm_chain_context_tag("Ethereum".to_string(), "sepolia".to_string()),
            "ethereum_sepolia"
        );
    }

    #[test]
    fn evm_chain_context_non_evm() {
        assert_eq!(
            core_evm_chain_context_tag("Bitcoin".to_string(), "mainnet".to_string()),
            ""
        );
    }

    #[test]
    fn parse_dogecoin_index() {
        assert_eq!(
            core_parse_dogecoin_derivation_index(Some("m/44'/3'/0'/0/7".to_string()), "m/44'/3'/0'/0/".to_string()),
            Some(7)
        );
        assert_eq!(
            core_parse_dogecoin_derivation_index(Some("other".to_string()), "m/44'/3'/0'/0/".to_string()),
            None
        );
    }

    #[test]
    fn rebroadcast_dispatch_btc() {
        let d = core_rebroadcast_dispatch_for_format("bitcoin.raw_hex".to_string()).unwrap();
        assert_eq!(d.chain_id, 0);
        assert_eq!(d.result_field, "txid");
    }

    #[test]
    fn rebroadcast_dispatch_unknown_errors() {
        assert!(core_rebroadcast_dispatch_for_format("nope".to_string()).is_err());
    }

    #[test]
    fn evm_has_contract_code_variants() {
        assert!(!core_evm_has_contract_code("0x".to_string()));
        assert!(!core_evm_has_contract_code("0X0".to_string()));
        assert!(!core_evm_has_contract_code("   0x ".to_string()));
        assert!(!core_evm_has_contract_code(String::new()));
        assert!(core_evm_has_contract_code("0x60806040".to_string()));
    }

    #[test]
    fn evm_bump_defaults_when_blank() {
        let r = core_evm_replacement_fee_bump(None, Some(" ".to_string()), 4.0, 2.0);
        assert_eq!(r.max_fee_gwei, "4.0");
        assert_eq!(r.priority_fee_gwei, "2.0");
    }

    #[test]
    fn evm_bump_scales_existing() {
        let r = core_evm_replacement_fee_bump(
            Some("5.0".to_string()), Some("2.5".to_string()), 4.0, 2.0,
        );
        assert_eq!(r.max_fee_gwei, "6.000");
        assert_eq!(r.priority_fee_gwei, "3.000");
    }

    #[test]
    fn prepare_payload_sui_signed_json_remap() {
        let raw = r#"{"txBytesBase64":"AAAA","signatureBase64":"BBBB"}"#;
        let p = core_rebroadcast_prepare_payload("sui.signed_json".into(), raw.into()).unwrap();
        assert_eq!(p.chain_id, 12);
        assert_eq!(p.result_field, "digest");
        let parsed: serde_json::Value = serde_json::from_str(&p.broadcast_payload).unwrap();
        assert_eq!(parsed["tx_bytes_b64"], "AAAA");
        assert_eq!(parsed["sig_b64"], "BBBB");
    }

    #[test]
    fn prepare_payload_sui_malformed_passthrough() {
        let raw = "not json";
        let p = core_rebroadcast_prepare_payload("sui.signed_json".into(), raw.into()).unwrap();
        assert_eq!(p.broadcast_payload, raw);
    }

    #[test]
    fn prepare_payload_wrap_key() {
        let p = core_rebroadcast_prepare_payload("xrp.blob_hex".into(), "deadbeef".into()).unwrap();
        assert_eq!(p.chain_id, 8);
        assert_eq!(p.result_field, "txid");
        let parsed: serde_json::Value = serde_json::from_str(&p.broadcast_payload).unwrap();
        assert_eq!(parsed["tx_blob_hex"], "deadbeef");
    }

    #[test]
    fn prepare_payload_extract_field() {
        let raw = r#"{"raw_tx_hex":"ff00","other":"x"}"#;
        let p = core_rebroadcast_prepare_payload("bitcoin.rust_json".into(), raw.into()).unwrap();
        assert_eq!(p.chain_id, 0);
        assert_eq!(p.broadcast_payload, "ff00");
    }

    #[test]
    fn prepare_payload_passthrough() {
        let p = core_rebroadcast_prepare_payload("bitcoin.raw_hex".into(), "abcd".into()).unwrap();
        assert_eq!(p.broadcast_payload, "abcd");
    }

    #[test]
    fn prepare_payload_unknown_errors() {
        assert!(core_rebroadcast_prepare_payload("nope".into(), "x".into()).is_err());
    }

    #[test]
    fn evm_bump_respects_floor() {
        let r = core_evm_replacement_fee_bump(
            Some("0.01".to_string()), Some("0.01".to_string()), 4.0, 2.0,
        );
        assert_eq!(r.max_fee_gwei, "0.100");
        assert_eq!(r.priority_fee_gwei, "0.100");
    }

}

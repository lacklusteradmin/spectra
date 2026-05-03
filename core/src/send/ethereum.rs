// Ethereum / EVM send — pure-logic core.
//
// HTTP (fetch preview JSON), ENS resolution, Keychain (seed access), and
// broadcast stay in Swift by architectural decision. This module owns:
// 1. Input validation (address format, amount parsing, native-vs-token choice)
// 2. Request assembly (value_wei, to, data_hex for native vs ERC-20 transfer)
// 3. Preview JSON decoding with optional custom fee / nonce overrides
//
// The goal: Swift becomes a thin caller — validate + assemble in Rust,
// HTTP in Swift, decode in Rust, state write via WalletCore (already Rust).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmCustomFeeConfiguration {
    pub max_fee_per_gas_gwei: f64,
    pub max_priority_fee_per_gas_gwei: f64,
}

/// Typed EVM overrides crossing the FFI from Swift. Internal-only on the
/// Rust side: `build_execute_send_payload` projects this into the comma-prefix
/// JSON fragment that `build_evm_*_send_payload` expects.
#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendOverridesInput {
    pub nonce: Option<i64>,
    pub custom_fees: Option<EvmCustomFeeConfiguration>,
    /// Pin the gas limit. Defaults: 21_000 for plain ETH, node-estimated for
    /// ERC-20 / contract calls. Must be set for arbitrary calldata sends.
    pub gas_limit: Option<i64>,
    /// Hex-encoded calldata (with or without 0x prefix). For native ETH sends
    /// this appends arbitrary data (e.g. a memo). For ERC-20 sends, this
    /// overrides the auto-encoded `transfer(to, amount)` calldata entirely,
    /// enabling approvals, swaps, multicall, or any ABI-encoded function call.
    pub calldata_hex: Option<String>,
    /// Sign the transaction without broadcasting. The signed raw transaction
    /// hex is returned in `SendExecutionResult.evm.raw_tx_hex`; `txid` is
    /// left empty. Useful for offline signing or pre-flight inspection.
    pub sign_only: Option<bool>,
    /// EIP-2930 access list as a flat JSON string (array of
    /// `{address, storageKeys}` objects). Pre-warms storage slots to reduce
    /// gas cost for contracts with known read patterns.
    pub access_list_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSupportedToken {
    pub symbol: String,
    pub contract_address: String,
    pub decimals: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendAssemblyInput {
    pub chain_name: String,
    pub symbol: String,
    pub from_address: String,
    // Caller passes the already-resolved destination (ENS resolved in Swift).
    pub resolved_destination: String,
    pub amount: f64,
    // If set, this is an ERC-20 transfer (symbol is the token symbol).
    pub token: Option<EvmSupportedToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendAssembly {
    pub value_wei: String,
    pub to_address: String,
    pub data_hex: String,
    pub is_native: bool,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EvmSendError {
    #[error("Invalid destination address")]
    InvalidDestination,
    #[error("Invalid from address")]
    InvalidFromAddress,
    #[error("Unsupported chain: {0}")]
    UnsupportedChain(String),
    #[error("Unsupported asset for chain")]
    UnsupportedAsset,
    #[error("Invalid amount")]
    InvalidAmount,
}

fn normalize_evm_address(address: &str) -> String {
    address.trim().to_lowercase()
}

fn is_valid_evm_address(address: &str) -> bool {
    let a = normalize_evm_address(address);
    a.len() == 42 && a.starts_with("0x") && a[2..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn is_native_evm_asset(chain_name: &str, symbol: &str) -> bool {
    matches!(
        (chain_name, symbol),
        ("Ethereum", "ETH")
            | ("Ethereum Classic", "ETC")
            | ("Arbitrum", "ETH")
            | ("Arbitrum", "ARB")
            | ("Optimism", "ETH")
            | ("Optimism", "OP")
            | ("BNB Chain", "BNB")
            | ("Avalanche", "AVAX")
            | ("Hyperliquid", "HYPE")
    )
}

pub fn is_supported_evm_chain(chain_name: &str) -> bool {
    matches!(
        chain_name,
        "Ethereum"
            | "Ethereum Classic"
            | "Arbitrum"
            | "Optimism"
            | "BNB Chain"
            | "Avalanche"
            | "Hyperliquid"
    )
}

/// Convert a decimal amount (e.g. 1.5) to wei with 18 decimals as a decimal string.
/// Avoids float rounding by doing string arithmetic with up to 18 fractional digits.
fn amount_to_smallest_unit(amount: f64, decimals: u32) -> Result<String, EvmSendError> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(EvmSendError::InvalidAmount);
    }
    // Format with up to `decimals` fractional digits, then shift.
    let formatted = format!("{:.*}", decimals as usize, amount);
    let (int_part, frac_part) = match formatted.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (formatted.clone(), "0".repeat(decimals as usize)),
    };
    let mut digits = int_part;
    digits.push_str(&frac_part);
    // Strip leading zeros but keep at least "0".
    let trimmed = digits.trim_start_matches('0').to_string();
    Ok(if trimmed.is_empty() { "0".to_string() } else { trimmed })
}

fn encode_erc20_transfer_data(destination: &str, amount_smallest: &str) -> Result<String, EvmSendError> {
    let dst = normalize_evm_address(destination);
    if !is_valid_evm_address(&dst) {
        return Err(EvmSendError::InvalidDestination);
    }
    let addr_body = &dst[2..];
    let addr_padded = format!("{:0>64}", addr_body);
    // amount as hex, zero-padded to 32 bytes.
    let amount_hex = u128_str_to_hex(amount_smallest)?;
    let amount_padded = format!("{:0>64}", amount_hex);
    Ok(format!("0xa9059cbb{}{}", addr_padded, amount_padded))
}

fn u128_str_to_hex(decimal: &str) -> Result<String, EvmSendError> {
    // Token amounts generally fit in u128 even with 18 decimals up to ~3.4e20 tokens.
    let value: u128 = decimal.parse().map_err(|_| EvmSendError::InvalidAmount)?;
    Ok(format!("{:x}", value))
}

#[uniffi::export]
pub fn prepare_evm_send_assembly(
    input: EvmSendAssemblyInput,
) -> Result<EvmSendAssembly, EvmSendError> {
    if !is_supported_evm_chain(&input.chain_name) {
        return Err(EvmSendError::UnsupportedChain(input.chain_name));
    }
    if !is_valid_evm_address(&input.from_address) {
        return Err(EvmSendError::InvalidFromAddress);
    }
    if !is_valid_evm_address(&input.resolved_destination) {
        return Err(EvmSendError::InvalidDestination);
    }
    let destination = normalize_evm_address(&input.resolved_destination);

    if is_native_evm_asset(&input.chain_name, &input.symbol) {
        let wei = amount_to_smallest_unit(input.amount, 18)?;
        return Ok(EvmSendAssembly {
            value_wei: wei,
            to_address: destination,
            data_hex: "0x".to_string(),
            is_native: true,
        });
    }

    let Some(token) = input.token else {
        return Err(EvmSendError::UnsupportedAsset);
    };
    let smallest = amount_to_smallest_unit(input.amount, token.decimals)?;
    let data_hex = encode_erc20_transfer_data(&destination, &smallest)?;
    let contract = normalize_evm_address(&token.contract_address);
    if !is_valid_evm_address(&contract) {
        return Err(EvmSendError::UnsupportedAsset);
    }
    Ok(EvmSendAssembly {
        value_wei: "0".to_string(),
        to_address: contract,
        data_hex,
        is_native: false,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvmPreviewDecodeInput {
    pub raw_json: String,
    pub explicit_nonce: Option<i64>,
    pub custom_fees: Option<EvmCustomFeeConfiguration>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvmPreviewDecoded {
    pub nonce: i64,
    pub gas_limit: i64,
    pub max_fee_per_gas_gwei: f64,
    pub max_priority_fee_per_gas_gwei: f64,
    pub estimated_network_fee_eth: f64,
    pub spendable_balance: Option<f64>,
    pub fee_rate_description: Option<String>,
    pub max_sendable: Option<f64>,
}

pub fn decode_evm_send_preview(input: EvmPreviewDecodeInput) -> Option<EvmPreviewDecoded> {
    let value: serde_json::Value = serde_json::from_str(&input.raw_json).ok()?;
    let obj = value.as_object()?;

    let rpc_nonce = obj.get("nonce").and_then(|v| v.as_i64()).unwrap_or(0);
    let nonce = input.explicit_nonce.unwrap_or(rpc_nonce);
    let gas_limit = obj.get("gas_limit").and_then(|v| v.as_i64()).unwrap_or(21_000);
    let live_fee_gwei = obj
        .get("max_fee_per_gas_gwei")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let live_prio_gwei = obj
        .get("max_priority_fee_per_gas_gwei")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let (max_fee_gwei, prio_gwei, fee_eth, fee_desc) = match input.custom_fees {
        Some(cf) => {
            let fee_wei = (gas_limit as f64) * cf.max_fee_per_gas_gwei * 1_000_000_000.0;
            let fee_eth = fee_wei / 1_000_000_000_000_000_000.0;
            let desc = format!(
                "Max {:.2} gwei / Priority {:.2} gwei (custom)",
                cf.max_fee_per_gas_gwei, cf.max_priority_fee_per_gas_gwei
            );
            (
                cf.max_fee_per_gas_gwei,
                cf.max_priority_fee_per_gas_gwei,
                fee_eth,
                Some(desc),
            )
        }
        None => {
            let fee_eth = obj
                .get("estimated_fee_eth")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let desc = obj
                .get("fee_rate_description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (live_fee_gwei, live_prio_gwei, fee_eth, desc)
        }
    };
    let spendable = obj.get("spendable_eth").and_then(|v| v.as_f64());
    Some(EvmPreviewDecoded {
        nonce,
        gas_limit,
        max_fee_per_gas_gwei: max_fee_gwei,
        max_priority_fee_per_gas_gwei: prio_gwei,
        estimated_network_fee_eth: fee_eth,
        spendable_balance: spendable,
        fee_rate_description: fee_desc,
        max_sendable: spendable,
    })
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Build the JSON payload for a native EVM send. `overrides_fragment` is the
/// comma-prefixed string from `build_evm_overrides_json_fragment` (may be empty).
#[uniffi::export]
pub fn build_evm_native_send_payload(
    from_address: String,
    to_address: String,
    value_wei: String,
    private_key_hex: String,
    overrides_fragment: String,
) -> String {
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"value_wei\":\"{}\",\"private_key_hex\":\"{}\"{}}}",
        json_escape(&from_address),
        json_escape(&to_address),
        json_escape(&value_wei),
        json_escape(&private_key_hex),
        overrides_fragment
    )
}

#[uniffi::export]
pub fn build_evm_token_send_payload(
    from_address: String,
    contract_address: String,
    to_address: String,
    amount_raw: String,
    private_key_hex: String,
    overrides_fragment: String,
) -> String {
    format!(
        "{{\"from\":\"{}\",\"contract\":\"{}\",\"to\":\"{}\",\"amount_raw\":\"{}\",\"private_key_hex\":\"{}\"{}}}",
        json_escape(&from_address),
        json_escape(&contract_address),
        json_escape(&to_address),
        json_escape(&amount_raw),
        json_escape(&private_key_hex),
        overrides_fragment
    )
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendResultDecoded {
    pub txid: String,
    pub raw_tx_hex: String,
    pub nonce: i64,
    pub gas_limit: i64,
}

/// Internal helper: render the typed overrides into the comma-prefix JSON
/// fragment consumed by `build_evm_native_send_payload` /
/// `build_evm_token_send_payload`. Not UniFFI-exported — Swift now passes the
/// typed `EvmSendOverridesInput` on `SendExecutionRequest` instead.
pub(crate) fn render_evm_overrides_fragment(input: Option<&EvmSendOverridesInput>) -> String {
    let Some(o) = input else { return String::new() };
    let mut fragments: Vec<String> = Vec::new();
    if let Some(n) = o.nonce {
        fragments.push(format!("\"nonce\":{}", n));
    }
    if let Some(ref cf) = o.custom_fees {
        let max_fee_wei = (cf.max_fee_per_gas_gwei * 1e9).round() as u64;
        let prio_wei = (cf.max_priority_fee_per_gas_gwei * 1e9).round() as u64;
        fragments.push(format!("\"max_fee_per_gas_wei\":\"{}\"", max_fee_wei));
        fragments.push(format!("\"max_priority_fee_per_gas_wei\":\"{}\"", prio_wei));
    }
    if let Some(gl) = o.gas_limit {
        fragments.push(format!("\"gas_limit\":{}", gl));
    }
    if let Some(ref cd) = o.calldata_hex {
        let cd_clean = cd.trim_start_matches("0x");
        fragments.push(format!("\"calldata_hex\":\"{}\"", json_escape(cd_clean)));
    }
    if o.sign_only == Some(true) {
        fragments.push("\"sign_only\":true".to_string());
    }
    if let Some(ref al) = o.access_list_json {
        fragments.push(format!("\"access_list_json\":{}", al));
    }
    if fragments.is_empty() { String::new() } else { format!(",{}", fragments.join(",")) }
}

/// Internal helper: parse the broadcast result JSON into the typed EVM record.
/// Used by `execute_send` to populate `SendExecutionResult.evm` so Swift
/// doesn't have to re-parse the JSON.
pub(crate) fn decode_evm_send_result_internal(json: &str, fallback_nonce: i64) -> EvmSendResultDecoded {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => {
            return EvmSendResultDecoded { nonce: fallback_nonce, ..Default::default() };
        }
    };
    let obj = v.as_object();
    let get_str = |k: &str| -> String {
        obj.and_then(|o| o.get(k)).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    let nonce = obj
        .and_then(|o| o.get("nonce"))
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
        .unwrap_or(fallback_nonce);
    let gas_limit = obj
        .and_then(|o| o.get("gas_limit"))
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
        .unwrap_or(0);
    EvmSendResultDecoded {
        txid: get_str("txid"),
        raw_tx_hex: get_str("raw_tx_hex"),
        nonce,
        gas_limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_eth_assembly() {
        let a = prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "ETH".into(),
            from_address: "0x1111111111111111111111111111111111111111".into(),
            resolved_destination: "0x2222222222222222222222222222222222222222".into(),
            amount: 1.5,
            token: None,
        })
        .unwrap();
        assert!(a.is_native);
        assert_eq!(a.to_address, "0x2222222222222222222222222222222222222222");
        assert_eq!(a.data_hex, "0x");
        // 1.5 ETH = 1_500_000_000_000_000_000 wei
        assert_eq!(a.value_wei, "1500000000000000000");
    }

    #[test]
    fn erc20_assembly_has_transfer_selector() {
        let a = prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "USDC".into(),
            from_address: "0x1111111111111111111111111111111111111111".into(),
            resolved_destination: "0x2222222222222222222222222222222222222222".into(),
            amount: 100.0,
            token: Some(EvmSupportedToken {
                symbol: "USDC".into(),
                contract_address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".into(),
                decimals: 6,
            }),
        })
        .unwrap();
        assert!(!a.is_native);
        assert_eq!(a.value_wei, "0");
        assert_eq!(a.to_address, "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        assert!(a.data_hex.starts_with("0xa9059cbb"));
        // 100 USDC at 6 decimals = 100_000_000 = 0x5F5E100, padded
        assert!(a.data_hex.ends_with("0000000000000000000000000000000000000000000000000000000005f5e100"));
    }

    #[test]
    fn invalid_destination_rejected() {
        let err = prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "ETH".into(),
            from_address: "0x1111111111111111111111111111111111111111".into(),
            resolved_destination: "not-an-address".into(),
            amount: 1.0,
            token: None,
        })
        .unwrap_err();
        matches!(err, EvmSendError::InvalidDestination);
    }

    #[test]
    fn preview_decode_with_custom_fees() {
        let json = r#"{"nonce":7,"gas_limit":21000,"max_fee_per_gas_gwei":30.0,"max_priority_fee_per_gas_gwei":2.0,"estimated_fee_eth":0.00063,"fee_rate_description":"live desc","spendable_eth":4.2}"#;
        let decoded = decode_evm_send_preview(EvmPreviewDecodeInput {
            raw_json: json.into(),
            explicit_nonce: Some(12),
            custom_fees: Some(EvmCustomFeeConfiguration {
                max_fee_per_gas_gwei: 50.0,
                max_priority_fee_per_gas_gwei: 3.0,
            }),
        })
        .unwrap();
        assert_eq!(decoded.nonce, 12);
        assert_eq!(decoded.max_fee_per_gas_gwei, 50.0);
        assert!(decoded
            .fee_rate_description
            .as_deref()
            .unwrap_or("")
            .contains("custom"));
        // 21000 * 50 gwei = 0.00105 ETH
        assert!((decoded.estimated_network_fee_eth - 0.00105).abs() < 1e-9);
        assert_eq!(decoded.spendable_balance, Some(4.2));
    }

    #[test]
    fn overrides_fragment_builds_comma_prefixed() {
        let s = render_evm_overrides_fragment(Some(&EvmSendOverridesInput {
            nonce: Some(9),
            custom_fees: Some(EvmCustomFeeConfiguration { max_fee_per_gas_gwei: 50.0, max_priority_fee_per_gas_gwei: 3.0 }),
        }));
        assert!(s.starts_with(","));
        assert!(s.contains("\"nonce\":9"));
        assert!(s.contains("\"max_fee_per_gas_wei\":\"50000000000\""));
        assert!(s.contains("\"max_priority_fee_per_gas_wei\":\"3000000000\""));
    }

    #[test]
    fn overrides_fragment_empty_when_none() {
        assert!(render_evm_overrides_fragment(None).is_empty());
        assert!(render_evm_overrides_fragment(Some(&EvmSendOverridesInput::default())).is_empty());
    }

    #[test]
    fn decode_send_result_pulls_fields() {
        let json = r#"{"txid":"0xabc","raw_tx_hex":"0xf86...","nonce":12,"gas_limit":21000}"#;
        let r = decode_evm_send_result_internal(json, 0);
        assert_eq!(r.txid, "0xabc");
        assert_eq!(r.nonce, 12);
        assert_eq!(r.gas_limit, 21000);
    }

    #[test]
    fn native_payload_escapes_and_overrides() {
        let p = build_evm_native_send_payload(
            "0xfrom".into(), "0xto".into(), "1000".into(), "aa".into(),
            ",\"nonce\":5".into(),
        );
        assert!(p.contains("\"value_wei\":\"1000\""));
        assert!(p.contains("\"private_key_hex\":\"aa\""));
        assert!(p.ends_with(",\"nonce\":5}"));
    }

    #[test]
    fn native_payload_escapes_quotes() {
        let p = build_evm_native_send_payload(
            "0xfr\"om".into(), "0xto".into(), "0".into(), "k".into(), "".into(),
        );
        assert!(p.contains("0xfr\\\"om"));
        let v: serde_json::Value = serde_json::from_str(&p).unwrap();
        assert_eq!(v["from"], "0xfr\"om");
    }

    #[test]
    fn token_payload_shape() {
        let p = build_evm_token_send_payload(
            "0xfrom".into(), "0xcontract".into(), "0xto".into(), "1000000".into(), "k".into(), "".into(),
        );
        let v: serde_json::Value = serde_json::from_str(&p).unwrap();
        assert_eq!(v["contract"], "0xcontract");
        assert_eq!(v["amount_raw"], "1000000");
    }

    #[test]
    fn decode_send_result_uses_fallback_on_missing_nonce() {
        let r = decode_evm_send_result_internal(r#"{"txid":"x"}"#, 7);
        assert_eq!(r.nonce, 7);
        assert_eq!(r.gas_limit, 0);
    }

    #[test]
    fn preview_decode_without_overrides_uses_rpc() {
        let json = r#"{"nonce":3,"gas_limit":21000,"max_fee_per_gas_gwei":20.0,"max_priority_fee_per_gas_gwei":1.5,"estimated_fee_eth":0.00042,"fee_rate_description":"rpc desc","spendable_eth":2.0}"#;
        let decoded = decode_evm_send_preview(EvmPreviewDecodeInput {
            raw_json: json.into(),
            explicit_nonce: None,
            custom_fees: None,
        })
        .unwrap();
        assert_eq!(decoded.nonce, 3);
        assert_eq!(decoded.max_fee_per_gas_gwei, 20.0);
        assert_eq!(decoded.fee_rate_description.as_deref(), Some("rpc desc"));
    }
}

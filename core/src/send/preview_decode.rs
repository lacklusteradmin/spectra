// Send-preview decoders for the 17 non-EVM chains.
//
// Pure-logic: Swift fetches JSON over HTTP and hands it here; Rust parses and
// returns a typed decoded record. Mirrors ethereum_send.rs but for each of:
// Bitcoin / BCH / BSV / Litecoin (shared UTXO), Bitcoin HD (xpub),
// Dogecoin, Tron, and the 11 simple-fee chains
// (Solana / XRP / Stellar / Monero / Cardano / Sui / Aptos / TON / ICP / NEAR / Polkadot).

use serde::{Deserialize, Serialize};

const SAT_PER_COIN: f64 = 100_000_000.0;

fn json_escape_local(s: &str) -> String {
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

/// Generic JSON object builder for chain-specific send payloads.
/// Preserves insertion order. Numeric fields emit unquoted; Str fields emit
/// JSON-escaped quoted strings; Raw emits verbatim (caller's responsibility).
#[derive(Debug, Clone, uniffi::Enum)]
pub enum JsonFieldValue {
    Str { value: String },
    Int { value: i64 },
    UInt { value: u64 },
    Float { value: f64 },
    Bool { value: bool },
    /// Verbatim JSON text (e.g. a nested object or pre-formatted number).
    Raw { value: String },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct JsonField {
    pub name: String,
    pub value: JsonFieldValue,
}

pub fn build_json_object(fields: Vec<JsonField>) -> String {
    let mut out = String::from("{");
    for (i, f) in fields.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        out.push_str(&json_escape_local(&f.name));
        out.push_str("\":");
        match &f.value {
            JsonFieldValue::Str { value } => {
                out.push('"');
                out.push_str(&json_escape_local(value));
                out.push('"');
            }
            JsonFieldValue::Int { value } => out.push_str(&value.to_string()),
            JsonFieldValue::UInt { value } => out.push_str(&value.to_string()),
            JsonFieldValue::Float { value } => out.push_str(&value.to_string()),
            JsonFieldValue::Bool { value } => out.push_str(if *value { "true" } else { "false" }),
            JsonFieldValue::Raw { value } => out.push_str(value),
        }
    }
    out.push('}');
    out
}

pub fn build_utxo_sat_send_payload(
    from_address: String,
    to_address: String,
    amount_sat: u64,
    fee_sat: u64,
    private_key_hex: String,
) -> String {
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"amount_sat\":{},\"fee_sat\":{},\"private_key_hex\":\"{}\"}}",
        json_escape_local(&from_address),
        json_escape_local(&to_address),
        amount_sat,
        fee_sat,
        json_escape_local(&private_key_hex)
    )
}

/// Extract a top-level JSON field as a string. Numeric/bool values are stringified.
/// Missing keys or invalid JSON return "". Callers use this for loose scraping of
/// send-result JSON where the value may be a hash, signature, index, digest, etc.
pub fn extract_json_string_field(json: String, key: String) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
        return String::new();
    };
    let Some(obj) = v.as_object() else { return String::new() };
    match obj.get(&key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string().trim_matches('"').to_string(),
    }
}

/// Convert a decimal amount to its smallest-unit string representation.
/// Used for ETH→wei, NEAR→yocto, DOT→planck, etc. String-based to avoid f64 rounding.
pub fn amount_to_raw_units_string(amount: f64, decimals: u32) -> String {
    if !amount.is_finite() || amount < 0.0 {
        return "0".to_string();
    }
    let formatted = format!("{:.*}", decimals as usize, amount);
    let (int_part, frac_part) = match formatted.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (formatted, "0".repeat(decimals as usize)),
    };
    let mut digits = int_part;
    digits.push_str(&frac_part);
    let trimmed = digits.trim_start_matches('0').to_string();
    if trimmed.is_empty() { "0".to_string() } else { trimmed }
}

fn obj_f64(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<f64> {
    o.get(k).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
    })
}

fn obj_u64(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<u64> {
    o.get(k).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
    })
}

fn obj_str(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<String> {
    o.get(k).and_then(|v| v.as_str().map(|s| s.to_string()))
}

// ----- UTXO (BTC / BCH / BSV / LTC / DOGE base fields) -----

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UtxoSendPreview {
    pub estimated_fee_rate_sat_vb: u64,
    pub estimated_network_fee_coin: f64,
    pub fee_rate_description: String,
    pub spendable_balance: f64,
    pub estimated_transaction_bytes: i64,
    pub selected_input_count: i64,
    pub max_sendable: f64,
    pub estimated_fee_sat: u64,
    pub spendable_sat: u64,
    pub max_sendable_sat: u64,
}

pub fn decode_utxo_send_preview(json: String) -> Option<UtxoSendPreview> {
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let o = v.as_object()?;
    let rate = obj_u64(o, "fee_rate_svb").unwrap_or(1);
    let fee_sat = obj_u64(o, "estimated_fee_sat").unwrap_or(0);
    let tx_bytes = obj_u64(o, "estimated_tx_bytes").unwrap_or(0) as i64;
    let input_count = obj_u64(o, "selected_input_count").unwrap_or(0) as i64;
    let spend_sat = obj_u64(o, "spendable_balance_sat").unwrap_or(0);
    let max_sat = obj_u64(o, "max_sendable_sat").unwrap_or(0);
    Some(UtxoSendPreview {
        estimated_fee_rate_sat_vb: rate,
        estimated_network_fee_coin: fee_sat as f64 / SAT_PER_COIN,
        fee_rate_description: format!("{} sat/vB", rate),
        spendable_balance: spend_sat as f64 / SAT_PER_COIN,
        estimated_transaction_bytes: tx_bytes,
        selected_input_count: input_count,
        max_sendable: max_sat as f64 / SAT_PER_COIN,
        estimated_fee_sat: fee_sat,
        spendable_sat: spend_sat,
        max_sendable_sat: max_sat,
    })
}

// ----- Bitcoin HD (xpub) -----

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BitcoinHdSendPreview {
    pub estimated_fee_rate_sat_vb: u64,
    pub estimated_network_fee_btc: f64,
    pub fee_rate_description: String,
    pub spendable_balance: f64,
    pub estimated_transaction_bytes: i64,
    pub max_sendable: f64,
}

pub fn decode_bitcoin_hd_send_preview(
    balance_json: String,
    fee_json: String,
) -> Option<BitcoinHdSendPreview> {
    let bv: serde_json::Value = serde_json::from_str(&balance_json).ok()?;
    let bo = bv.as_object()?;
    let confirmed_sats = obj_u64(bo, "confirmed_sats").unwrap_or(0);

    let fv: serde_json::Value = serde_json::from_str(&fee_json).ok()?;
    let fo = fv.as_object()?;
    let raw = obj_f64(fo, "sats_per_vbyte").unwrap_or(1.0);
    let rate = raw.ceil().max(1.0) as u64;
    let bytes: u64 = 250;
    let fee_sat = rate * bytes;
    let spendable_sat = confirmed_sats.saturating_sub(fee_sat);
    Some(BitcoinHdSendPreview {
        estimated_fee_rate_sat_vb: rate,
        estimated_network_fee_btc: fee_sat as f64 / SAT_PER_COIN,
        fee_rate_description: format!("{} sat/vB", rate),
        spendable_balance: confirmed_sats as f64 / SAT_PER_COIN,
        estimated_transaction_bytes: bytes as i64,
        max_sendable: spendable_sat as f64 / SAT_PER_COIN,
    })
}

// ----- Dogecoin -----

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DogecoinSendPreviewDecoded {
    pub spendable_balance_doge: f64,
    pub requested_amount_doge: f64,
    pub estimated_network_fee_doge: f64,
    pub estimated_fee_rate_doge_per_kb: f64,
    pub estimated_transaction_bytes: i64,
    pub selected_input_count: i64,
    pub uses_change_output: bool,
    pub fee_priority: String,
    pub max_sendable_doge: f64,
    pub fee_rate_description: String,
}

pub fn decode_dogecoin_send_preview(
    json: String,
    requested_amount: f64,
    fee_priority: String,
) -> Option<DogecoinSendPreviewDecoded> {
    let base = decode_utxo_send_preview(json)?;
    if base.spendable_sat == 0 {
        return None;
    }
    let requested_sat = (requested_amount * SAT_PER_COIN) as u64;
    let uses_change = base.spendable_sat > requested_sat + base.estimated_fee_sat;
    Some(DogecoinSendPreviewDecoded {
        spendable_balance_doge: base.spendable_balance,
        requested_amount_doge: requested_amount,
        estimated_network_fee_doge: base.estimated_network_fee_coin,
        estimated_fee_rate_doge_per_kb: base.estimated_fee_rate_sat_vb as f64 * 1000.0 / SAT_PER_COIN,
        estimated_transaction_bytes: base.estimated_transaction_bytes,
        selected_input_count: base.selected_input_count,
        uses_change_output: uses_change,
        fee_priority,
        max_sendable_doge: base.max_sendable,
        fee_rate_description: base.fee_rate_description,
    })
}

// ----- Tron -----

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TronSendPreviewDecoded {
    pub estimated_network_fee_trx: f64,
    pub fee_limit_sun: i64,
    pub spendable_balance: f64,
    pub max_sendable: f64,
    pub fee_rate_description: Option<String>,
}

pub fn decode_tron_send_preview(json: String) -> Option<TronSendPreviewDecoded> {
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let o = v.as_object()?;
    let fee_trx = obj_f64(o, "estimated_fee_trx").unwrap_or(0.0);
    let fee_limit_sun = o
        .get("fee_limit_sun")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let spendable = obj_f64(o, "spendable_balance").unwrap_or(0.0);
    let max_sendable = obj_f64(o, "max_sendable").unwrap_or(spendable);
    Some(TronSendPreviewDecoded {
        estimated_network_fee_trx: fee_trx,
        fee_limit_sun,
        spendable_balance: spendable,
        max_sendable,
        fee_rate_description: obj_str(o, "fee_rate_description"),
    })
}

// ----- Simple-fee chains -----

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SimpleSendPreview {
    pub fee_display: f64,
    pub fee_raw: String,
    pub fee_rate_description: String,
    pub balance_display: f64,
    pub max_sendable: f64,
}

pub fn decode_simple_send_preview(json: String) -> SimpleSendPreview {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
        return SimpleSendPreview::default();
    };
    let Some(o) = v.as_object() else {
        return SimpleSendPreview::default();
    };
    let fee_display = obj_f64(o, "fee_display").unwrap_or(0.0);
    let fee_raw = obj_str(o, "fee_raw").unwrap_or_default();
    let fee_rate = obj_str(o, "fee_rate_description").unwrap_or_default();
    let balance = obj_f64(o, "balance_display").unwrap_or(0.0);
    let max_sendable =
        obj_f64(o, "max_sendable").unwrap_or_else(|| (balance - fee_display).max(0.0));
    SimpleSendPreview {
        fee_display,
        fee_raw,
        fee_rate_description: fee_rate,
        balance_display: balance,
        max_sendable,
    }
}

// Per-chain default fee constants (surfaced to Swift so both sides share one source).
// When the preview JSON omits fee_raw, callers fall back to these.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Enum)]
pub enum SimpleChain {
    Solana,
    Xrp,
    Stellar,
    Monero,
    Cardano,
    Sui,
    Aptos,
    Ton,
    Icp,
    Near,
    Polkadot,
}

// Unified record builders: return the final UniFFI preview Record directly so
// Swift stops hand-wiring per-chain wrapping code. Each consumes the same raw
// inputs the old Swift helpers used; decode + wrap happens in Rust.

pub fn build_evm_send_preview_record(
    input: crate::ethereum_send::EvmPreviewDecodeInput,
) -> Option<crate::wallet_core::EthereumSendPreview> {
    let d = crate::ethereum_send::decode_evm_send_preview(input)?;
    Some(crate::wallet_core::EthereumSendPreview {
        nonce: d.nonce,
        gasLimit: d.gas_limit,
        maxFeePerGasGwei: d.max_fee_per_gas_gwei,
        maxPriorityFeePerGasGwei: d.max_priority_fee_per_gas_gwei,
        estimatedNetworkFeeEth: d.estimated_network_fee_eth,
        spendableBalance: d.spendable_balance,
        feeRateDescription: d.fee_rate_description,
        estimatedTransactionBytes: None,
        selectedInputCount: None,
        usesChangeOutput: None,
        maxSendable: d.max_sendable,
    })
}

pub fn build_utxo_send_preview_record(
    json: String,
) -> Option<crate::wallet_core::BitcoinSendPreview> {
    let d = decode_utxo_send_preview(json)?;
    if d.spendable_sat == 0 {
        return None;
    }
    Some(crate::wallet_core::BitcoinSendPreview {
        estimatedFeeRateSatVb: d.estimated_fee_rate_sat_vb,
        estimatedNetworkFeeBtc: d.estimated_network_fee_coin,
        feeRateDescription: Some(d.fee_rate_description),
        spendableBalance: Some(d.spendable_balance),
        estimatedTransactionBytes: Some(d.estimated_transaction_bytes),
        selectedInputCount: Some(d.selected_input_count),
        usesChangeOutput: None,
        maxSendable: Some(d.max_sendable),
    })
}

pub fn build_bitcoin_hd_send_preview_record(
    balance_json: String,
    fee_json: String,
) -> Option<crate::wallet_core::BitcoinSendPreview> {
    let d = decode_bitcoin_hd_send_preview(balance_json, fee_json)?;
    Some(crate::wallet_core::BitcoinSendPreview {
        estimatedFeeRateSatVb: d.estimated_fee_rate_sat_vb,
        estimatedNetworkFeeBtc: d.estimated_network_fee_btc,
        feeRateDescription: Some(d.fee_rate_description),
        spendableBalance: Some(d.spendable_balance),
        estimatedTransactionBytes: Some(d.estimated_transaction_bytes),
        selectedInputCount: None,
        usesChangeOutput: None,
        maxSendable: Some(d.max_sendable),
    })
}

pub fn build_dogecoin_send_preview_record(
    json: String,
    requested_amount: f64,
    fee_priority: String,
) -> Option<crate::wallet_core::DogecoinSendPreview> {
    let d = decode_dogecoin_send_preview(json, requested_amount, fee_priority)?;
    Some(crate::wallet_core::DogecoinSendPreview {
        spendableBalanceDoge: d.spendable_balance_doge,
        requestedAmountDoge: d.requested_amount_doge,
        estimatedNetworkFeeDoge: d.estimated_network_fee_doge,
        estimatedFeeRateDogePerKb: d.estimated_fee_rate_doge_per_kb,
        estimatedTransactionBytes: d.estimated_transaction_bytes,
        selectedInputCount: d.selected_input_count,
        usesChangeOutput: d.uses_change_output,
        feePriority: d.fee_priority,
        maxSendableDoge: d.max_sendable_doge,
        spendableBalance: d.spendable_balance_doge,
        feeRateDescription: Some(d.fee_rate_description),
        maxSendable: d.max_sendable_doge,
    })
}

pub fn build_tron_send_preview_record(
    json: String,
) -> Option<crate::wallet_core::TronSendPreview> {
    let d = decode_tron_send_preview(json)?;
    Some(crate::wallet_core::TronSendPreview {
        estimatedNetworkFeeTrx: d.estimated_network_fee_trx,
        feeLimitSun: d.fee_limit_sun,
        simulationUsed: false,
        spendableBalance: d.spendable_balance,
        feeRateDescription: d.fee_rate_description,
        estimatedTransactionBytes: None,
        selectedInputCount: None,
        usesChangeOutput: None,
        maxSendable: d.max_sendable,
    })
}

// Tagged-union output for unified simple-chain preview builder.
// Swift dispatches on the enum variant to assign the right @Published preview.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum SimpleChainPreview {
    Solana { preview: crate::wallet_core::SolanaSendPreview },
    Xrp { preview: crate::wallet_core::XRPSendPreview },
    Stellar { preview: crate::wallet_core::StellarSendPreview },
    Monero { preview: crate::wallet_core::MoneroSendPreview },
    Cardano { preview: crate::wallet_core::CardanoSendPreview },
    Sui { preview: crate::wallet_core::SuiSendPreview },
    Aptos { preview: crate::wallet_core::AptosSendPreview },
    Ton { preview: crate::wallet_core::TONSendPreview },
    Icp { preview: crate::wallet_core::ICPSendPreview },
    Near { preview: crate::wallet_core::NearSendPreview },
    Polkadot { preview: crate::wallet_core::PolkadotSendPreview },
}

pub fn build_simple_chain_preview(json: String, chain: SimpleChain) -> SimpleChainPreview {
    use crate::wallet_core::*;
    let p = decode_simple_send_preview(json);
    let fee = p.fee_display;
    let bal = p.balance_display;
    let desc = p.fee_rate_description.clone();
    let max = p.max_sendable;
    let raw = p.fee_raw.clone();
    match chain {
        SimpleChain::Solana => SimpleChainPreview::Solana { preview: SolanaSendPreview {
            estimatedNetworkFeeSol: fee, spendableBalance: bal, feeRateDescription: Some(desc),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Xrp => SimpleChainPreview::Xrp { preview: XRPSendPreview {
            estimatedNetworkFeeXrp: fee, feeDrops: raw.parse().unwrap_or(12),
            sequence: 0, lastLedgerSequence: 0, spendableBalance: bal, feeRateDescription: Some(desc),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Stellar => SimpleChainPreview::Stellar { preview: StellarSendPreview {
            estimatedNetworkFeeXlm: fee, feeStroops: raw.parse().unwrap_or(100),
            sequence: 0, spendableBalance: bal, feeRateDescription: Some(desc),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Monero => SimpleChainPreview::Monero { preview: MoneroSendPreview {
            estimatedNetworkFeeXmr: fee, priorityLabel: "normal".into(), spendableBalance: bal,
            feeRateDescription: Some(desc), estimatedTransactionBytes: None, selectedInputCount: None,
            usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Cardano => SimpleChainPreview::Cardano { preview: CardanoSendPreview {
            estimatedNetworkFeeAda: fee, ttlSlot: 0, spendableBalance: bal, feeRateDescription: Some(desc),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Sui => SimpleChainPreview::Sui { preview: SuiSendPreview {
            estimatedNetworkFeeSui: fee, gasBudgetMist: raw.parse().unwrap_or(3_000_000),
            referenceGasPrice: 1_000, spendableBalance: bal, feeRateDescription: Some(desc),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Aptos => {
            let gas: u64 = raw.parse().unwrap_or(100);
            SimpleChainPreview::Aptos { preview: AptosSendPreview {
                estimatedNetworkFeeApt: fee, maxGasAmount: 10_000, gasUnitPriceOctas: gas,
                spendableBalance: bal, feeRateDescription: Some(format!("{} octas/unit", gas)),
                estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
            }}
        },
        SimpleChain::Ton => SimpleChainPreview::Ton { preview: TONSendPreview {
            estimatedNetworkFeeTon: fee, sequenceNumber: 0, spendableBalance: bal,
            feeRateDescription: Some(desc), estimatedTransactionBytes: None, selectedInputCount: None,
            usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Icp => SimpleChainPreview::Icp { preview: ICPSendPreview {
            estimatedNetworkFeeIcp: fee, feeE8s: raw.parse().unwrap_or(10_000),
            spendableBalance: bal, feeRateDescription: Some(desc),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Near => SimpleChainPreview::Near { preview: NearSendPreview {
            estimatedNetworkFeeNear: fee, gasPriceYoctoNear: raw.clone(), spendableBalance: bal,
            feeRateDescription: Some(raw),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
        SimpleChain::Polkadot => SimpleChainPreview::Polkadot { preview: PolkadotSendPreview {
            estimatedNetworkFeeDot: fee, spendableBalance: bal, feeRateDescription: Some(desc),
            estimatedTransactionBytes: None, selectedInputCount: None, usesChangeOutput: None, maxSendable: max,
        }},
    }
}

pub fn simple_chain_default_fee_raw(chain: SimpleChain) -> String {
    match chain {
        SimpleChain::Solana => "5000".into(),        // lamports
        SimpleChain::Xrp => "12".into(),             // drops
        SimpleChain::Stellar => "100".into(),        // stroops
        SimpleChain::Monero => "0".into(),
        SimpleChain::Cardano => "170000".into(),     // lovelace
        SimpleChain::Sui => "3000000".into(),        // mist gas budget
        SimpleChain::Aptos => "100".into(),          // octas (gas unit)
        SimpleChain::Ton => "10000000".into(),       // nano ton
        SimpleChain::Icp => "10000".into(),          // e8s
        SimpleChain::Near => "100000000".into(),     // yocto gas price
        SimpleChain::Polkadot => "10000000000".into(), // planck
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utxo_decode_basic() {
        let json = r#"{"fee_rate_svb":5,"estimated_fee_sat":1000,"estimated_tx_bytes":200,"selected_input_count":2,"spendable_balance_sat":500000000,"max_sendable_sat":499999000}"#;
        let d = decode_utxo_send_preview(json.into()).unwrap();
        assert_eq!(d.estimated_fee_rate_sat_vb, 5);
        assert_eq!(d.fee_rate_description, "5 sat/vB");
        assert!((d.spendable_balance - 5.0).abs() < 1e-9);
        assert_eq!(d.selected_input_count, 2);
    }

    #[test]
    fn bitcoin_hd_decode() {
        let bal = r#"{"confirmed_sats":100000}"#;
        let fee = r#"{"sats_per_vbyte":"2.3"}"#;
        let d = decode_bitcoin_hd_send_preview(bal.into(), fee.into()).unwrap();
        assert_eq!(d.estimated_fee_rate_sat_vb, 3); // ceil(2.3)
        assert_eq!(d.estimated_transaction_bytes, 250);
        // spendable = 100000 - 3*250 = 99250
        assert!((d.max_sendable - 99250.0 / 100_000_000.0).abs() < 1e-12);
    }

    #[test]
    fn dogecoin_uses_change_flag() {
        let json = r#"{"fee_rate_svb":1,"estimated_fee_sat":1000,"estimated_tx_bytes":200,"selected_input_count":1,"spendable_balance_sat":1000000000,"max_sendable_sat":999999000}"#;
        let d = decode_dogecoin_send_preview(json.into(), 1.0, "Normal".into()).unwrap();
        assert!(d.uses_change_output);
        assert_eq!(d.fee_priority, "Normal");
        assert!((d.estimated_fee_rate_doge_per_kb - 0.00001).abs() < 1e-12);
    }

    #[test]
    fn tron_decode() {
        let json = r#"{"estimated_fee_trx":0.27,"fee_limit_sun":1000000,"spendable_balance":42.0,"max_sendable":41.73,"fee_rate_description":"bandwidth ok"}"#;
        let d = decode_tron_send_preview(json.into()).unwrap();
        assert_eq!(d.fee_limit_sun, 1_000_000);
        assert!((d.max_sendable - 41.73).abs() < 1e-9);
        assert_eq!(d.fee_rate_description.as_deref(), Some("bandwidth ok"));
    }

    #[test]
    fn simple_decode_computes_max_sendable_when_absent() {
        let json = r#"{"fee_display":0.00005,"fee_raw":"5000","fee_rate_description":"5000 lamports","balance_display":2.5}"#;
        let d = decode_simple_send_preview(json.into());
        assert!((d.max_sendable - (2.5 - 0.00005)).abs() < 1e-9);
        assert_eq!(d.fee_raw, "5000");
    }

    #[test]
    fn simple_decode_uses_explicit_max_sendable() {
        let json = r#"{"fee_display":0.1,"fee_raw":"100","fee_rate_description":"100 stroops","balance_display":10.0,"max_sendable":9.5}"#;
        let d = decode_simple_send_preview(json.into());
        assert_eq!(d.max_sendable, 9.5);
    }

    #[test]
    fn simple_decode_handles_missing_fields() {
        let d = decode_simple_send_preview("{}".into());
        assert_eq!(d.fee_display, 0.0);
        assert_eq!(d.balance_display, 0.0);
        assert_eq!(d.fee_raw, "");
    }

    #[test]
    fn json_object_builder_mixed_types_and_escapes() {
        let s = build_json_object(vec![
            JsonField { name: "from".into(), value: JsonFieldValue::Str { value: "a\"b".into() } },
            JsonField { name: "amount_sat".into(), value: JsonFieldValue::UInt { value: 42 } },
            JsonField { name: "memo".into(), value: JsonFieldValue::Raw { value: "null".into() } },
            JsonField { name: "flag".into(), value: JsonFieldValue::Bool { value: true } },
        ]);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["from"], "a\"b");
        assert_eq!(v["amount_sat"], 42);
        assert!(v["memo"].is_null());
        assert_eq!(v["flag"], true);
    }

    #[test]
    fn utxo_payload_has_numeric_sat_fields() {
        let p = build_utxo_sat_send_payload("bc1from".into(), "bc1to".into(), 150000, 1000, "kk".into());
        let v: serde_json::Value = serde_json::from_str(&p).unwrap();
        assert_eq!(v["amount_sat"], 150000);
        assert_eq!(v["fee_sat"], 1000);
        assert_eq!(v["from"], "bc1from");
    }

    #[test]
    fn extract_field_handles_strings_numbers_and_missing() {
        let j = r#"{"txid":"0xabc","nonce":7,"flag":true}"#;
        assert_eq!(extract_json_string_field(j.into(), "txid".into()), "0xabc");
        assert_eq!(extract_json_string_field(j.into(), "nonce".into()), "7");
        assert_eq!(extract_json_string_field(j.into(), "flag".into()), "true");
        assert_eq!(extract_json_string_field(j.into(), "missing".into()), "");
        assert_eq!(extract_json_string_field("{not json".into(), "x".into()), "");
    }

    #[test]
    fn amount_to_raw_units_handles_decimals() {
        assert_eq!(amount_to_raw_units_string(1.5, 18), "1500000000000000000");
        assert_eq!(amount_to_raw_units_string(0.0, 18), "0");
        assert_eq!(amount_to_raw_units_string(100.0, 6), "100000000");
        assert_eq!(amount_to_raw_units_string(-1.0, 18), "0");
    }

    #[test]
    fn simple_chain_default_fees() {
        assert_eq!(simple_chain_default_fee_raw(SimpleChain::Solana), "5000");
        assert_eq!(simple_chain_default_fee_raw(SimpleChain::Xrp), "12");
        assert_eq!(simple_chain_default_fee_raw(SimpleChain::Stellar), "100");
        assert_eq!(simple_chain_default_fee_raw(SimpleChain::Icp), "10000");
    }
}

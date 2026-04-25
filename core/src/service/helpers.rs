//! Internal helper functions used by the WalletService impl blocks.
//! None of these are UniFFI-exported.

use crate::registry::Chain;
use crate::state::AssetHolding;
use crate::SpectraBridgeError;
use serde::Serialize;
use serde_json::json;

// ── Param extraction ──────────────────────────────────────────────────────

pub(super) fn str_field<'a>(
    params: &'a serde_json::Value,
    key: &str,
) -> Result<&'a str, SpectraBridgeError> {
    params[key]
        .as_str()
        .ok_or_else(|| SpectraBridgeError::from(format!("missing field: {key}")))
}

pub(super) fn hex_field(
    params: &serde_json::Value,
    key: &str,
) -> Result<Vec<u8>, SpectraBridgeError> {
    let s = str_field(params, key)?;
    hex::decode(s).map_err(|e| SpectraBridgeError::from(format!("{key} hex decode: {e}")))
}

// ── Decimal scaling ───────────────────────────────────────────────────────

/// Format a smallest-unit `u128` amount as a fixed-decimal string. Used for
/// chains whose typed balance struct doesn't already provide a `_display` field.
pub(super) fn format_smallest_unit_decimal(amount: u128, decimals: u32) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let scale = 10u128.pow(decimals);
    let whole = amount / scale;
    let frac = amount % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    if trimmed.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{trimmed}")
    }
}

/// Scale a raw integer by `10^decimals` into a human-readable decimal string
/// with up to 6 fractional digits of precision.
pub(super) fn format_decimals(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let divisor: u128 = 10u128.pow(decimals as u32);
    let whole = raw / divisor;
    let frac = raw % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

// ── Balance JSON parsing ──────────────────────────────────────────────────

/// Extract the normalised native balance (in display units) from a balance
/// JSON value. Returns 0.0 for unknown / unsupported chains.
pub(super) fn simple_chain_balance_display(chain_id: u32, obj: &serde_json::Value) -> f64 {
    let u64_field = |key: &str| -> f64 {
        obj[key]
            .as_u64()
            .map(|n| n as f64)
            .or_else(|| obj[key].as_str().and_then(|s| s.parse::<u64>().ok()).map(|n| n as f64))
            .unwrap_or(0.0)
    };
    let i64_field = |key: &str| -> f64 {
        obj[key]
            .as_i64()
            .map(|n| n as f64)
            .or_else(|| obj[key].as_str().and_then(|s| s.parse::<i64>().ok()).map(|n| n as f64))
            .unwrap_or(0.0)
    };
    let Some(chain) = Chain::from_id(chain_id) else {
        return 0.0;
    };
    let factor = 10f64.powi(chain.native_decimals() as i32);
    match chain {
        Chain::Stellar => i64_field("stroops") / factor,
        Chain::Polkadot => obj["planck"]
            .as_u64()
            .map(|n| n as f64)
            .or_else(|| obj["planck"].as_str().and_then(|s| s.parse::<f64>().ok()))
            .unwrap_or(0.0)
            / factor,
        Chain::Near => {
            if let Some(s) = obj["near_display"].as_str() {
                s.parse::<f64>().unwrap_or(0.0)
            } else {
                obj["yocto_near"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|y| y / factor)
                    .unwrap_or(0.0)
            }
        }
        Chain::Solana
        | Chain::Xrp
        | Chain::Cardano
        | Chain::Sui
        | Chain::Aptos
        | Chain::Ton
        | Chain::Icp
        | Chain::Monero => match chain.native_balance_field() {
            Some(field) => u64_field(field) / factor,
            None => 0.0,
        },
        _ => 0.0,
    }
}

/// Parse a chain-specific balance JSON blob and return the native coin amount
/// as a plain `f64`. Mirrors the RustBalanceDecoder logic on the Swift side.
pub(super) fn native_amount_from_balance_json(chain_id: u32, json: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let chain = Chain::from_id(chain_id)?;
    let factor = 10f64.powi(chain.native_decimals() as i32);
    match chain {
        c if c.is_evm() => v["balance_display"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| {
                v["balance_wei"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|w| w / factor)
            })
            .or_else(|| v["balance_wei"].as_f64().map(|w| w / factor)),
        Chain::Near => v["near_display"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| {
                v["yocto_near"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|y| y / factor)
            })
            .or_else(|| v["yocto_near"].as_f64().map(|y| y / factor)),
        Chain::Stellar => {
            let field = chain.native_balance_field()?;
            v[field].as_i64().map(|s| s.unsigned_abs() as f64 / factor)
        }
        Chain::Polkadot => {
            let field = chain.native_balance_field()?;
            v[field]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| v[field].as_f64())
                .map(|p| p / factor)
        }
        c => {
            let field = c.native_balance_field()?;
            v[field].as_u64().map(|n| n as f64 / factor)
        }
    }
}

// ── Fee preview JSON shapes ───────────────────────────────────────────────

/// Flat struct for direct serialization — avoids the intermediate
/// `serde_json::Value` + `Map` heap allocation that `json!()` would produce.
#[derive(Serialize)]
struct FeePreview<'a> {
    chain_id: u32,
    native_fee_raw: &'a str,
    native_fee_display: &'a str,
    unit: &'a str,
    source: &'a str,
}

/// Build a `fee_preview` JSON string from an integer raw amount plus decimals.
/// Scales the raw amount down for a human-readable display field.
pub(super) fn fee_preview(chain_id: u32, raw: u128, decimals: u8, unit: &str, source: &str) -> String {
    let display = format_decimals(raw, decimals);
    let raw_str = raw.to_string();
    serde_json::to_string(&FeePreview {
        chain_id,
        native_fee_raw: &raw_str,
        native_fee_display: &display,
        unit,
        source,
    })
    .unwrap()
}

/// Variant that accepts pre-computed raw/display strings. Used when the raw
/// amount doesn't fit in `u128` (e.g. NEAR's 10^21 yoctoNEAR).
pub(super) fn fee_preview_str(
    chain_id: u32,
    raw: &str,
    display: &str,
    unit: &str,
    source: &str,
) -> String {
    serde_json::to_string(&FeePreview {
        chain_id,
        native_fee_raw: raw,
        native_fee_display: display,
        unit,
        source,
    })
    .unwrap()
}

/// Compute a UTXO capacity fee preview using P2PKH sizing (148 B/input,
/// 34 B/output, 10 B overhead). Assumes all confirmed UTXOs above the
/// 546-satoshi dust threshold are selected, single-output (max-send) tx.
pub(super) fn utxo_fee_preview_json(utxo_values: Vec<u64>, fee_rate: u64) -> String {
    const INPUT_BYTES: u64 = 148;
    const OUTPUT_BYTES: u64 = 34;
    const OVERHEAD: u64 = 10;
    const DUST: u64 = 546;

    let spendable: Vec<u64> = utxo_values.into_iter().filter(|&v| v >= DUST).collect();
    let n = spendable.len() as u64;
    let total: u64 = spendable.iter().sum();

    if n == 0 || total == 0 {
        return json!({
            "fee_rate_svb": fee_rate,
            "estimated_fee_sat": 0_u64,
            "estimated_tx_bytes": 0_u64,
            "selected_input_count": 0_u64,
            "uses_change_output": false,
            "spendable_balance_sat": 0_u64,
            "max_sendable_sat": 0_u64,
        })
        .to_string();
    }

    let tx_bytes = OVERHEAD + n * INPUT_BYTES + OUTPUT_BYTES;
    let fee = tx_bytes * fee_rate;
    let max_sendable = total.saturating_sub(fee);

    json!({
        "fee_rate_svb": fee_rate,
        "estimated_fee_sat": fee,
        "estimated_tx_bytes": tx_bytes,
        "selected_input_count": n,
        "uses_change_output": false,
        "spendable_balance_sat": total,
        "max_sendable_sat": max_sendable,
    })
    .to_string()
}

// ── EVM overrides parser ──────────────────────────────────────────────────

/// Parse optional EVM transaction overrides from a `sign_and_send` params blob.
/// All fields default to `None` — the Rust client then falls back to its
/// standard pending-nonce / recommended-fee / estimated-gas behavior.
pub(super) fn read_evm_overrides(
    params: &serde_json::Value,
) -> crate::send::chains::evm::EvmSendOverrides {
    let nonce = params["nonce"]
        .as_u64()
        .or_else(|| params["nonce"].as_str().and_then(|s| s.parse().ok()));
    let gas_limit = params["gas_limit"]
        .as_u64()
        .or_else(|| params["gas_limit"].as_str().and_then(|s| s.parse().ok()));
    let max_fee_per_gas_wei = params["max_fee_per_gas_wei"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| params["max_fee_per_gas_wei"].as_u64().map(|n| n as u128));
    let max_priority_fee_per_gas_wei = params["max_priority_fee_per_gas_wei"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| params["max_priority_fee_per_gas_wei"].as_u64().map(|n| n as u128));
    crate::send::chains::evm::EvmSendOverrides {
        nonce,
        max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei,
        gas_limit,
    }
}

// ── SQLite blocking helpers ───────────────────────────────────────────────

pub(super) fn sqlite_open(db_path: &str) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| format!("sqlite open {db_path}: {e}"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS state (
            key      TEXT    PRIMARY KEY,
            value    TEXT    NOT NULL,
            saved_at INTEGER NOT NULL
        );",
    )
    .map_err(|e| format!("sqlite create table: {e}"))?;
    Ok(conn)
}

pub(super) fn sqlite_load(db_path: &str, key: &str) -> Result<String, String> {
    let conn = sqlite_open(db_path)?;
    let result: rusqlite::Result<String> = conn.query_row(
        "SELECT value FROM state WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get(0),
    );
    match result {
        Ok(v) => Ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok("{}".to_string()),
        Err(e) => Err(format!("sqlite load: {e}")),
    }
}

pub(super) fn sqlite_save(db_path: &str, key: &str, value: &str) -> Result<(), String> {
    let conn = sqlite_open(db_path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO state (key, value, saved_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, saved_at = excluded.saved_at",
        rusqlite::params![key, value, now],
    )
    .map_err(|e| format!("sqlite save: {e}"))?;
    Ok(())
}

// ── State helpers ─────────────────────────────────────────────────────────

/// Return a zero-amount AssetHolding template for the native coin of each
/// chain. Used as the default when the holding doesn't exist yet.
pub(super) fn native_coin_template(chain_id: u32) -> Option<AssetHolding> {
    let chain = Chain::from_id(chain_id)?;
    Some(AssetHolding {
        name: chain.coin_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        coin_gecko_id: chain.coin_gecko_id().to_string(),
        chain_name: chain.chain_display_name().to_string(),
        token_standard: "Native".to_string(),
        contract_address: None,
        amount: 0.0,
        price_usd: 0.0,
    })
}

/// Upsert a holding by `(chain_name, contract_address)` for tokens or
/// `(chain_name, symbol)` for native coins. Replaces the full holding when
/// found, appends otherwise.
pub(super) fn upsert_asset_holding(holdings: &mut Vec<AssetHolding>, new: AssetHolding) {
    let pos = match &new.contract_address {
        Some(contract) => holdings.iter().position(|h| {
            h.chain_name == new.chain_name
                && h.contract_address.as_deref() == Some(contract.as_str())
        }),
        None => holdings.iter().position(|h| {
            h.chain_name == new.chain_name
                && h.symbol == new.symbol
                && h.contract_address.is_none()
        }),
    };
    if let Some(idx) = pos {
        holdings[idx] = new;
    } else {
        holdings.push(new);
    }
}

/// Returns `true` when `s` starts with a BIP-32 extended public key prefix.
pub(super) fn is_extended_public_key(s: &str) -> bool {
    matches!(
        s.get(..4),
        Some("xpub") | Some("ypub") | Some("zpub") | Some("Ypub") | Some("Zpub")
    )
}

/// Convert typed history encode inputs from Swift into the base64-payload
/// `HistoryRecord` shape expected by the SQLite layer.
pub(super) fn history_records_from_encode_inputs(
    records: Vec<crate::ffi::HistoryRecordEncodeInput>,
) -> Vec<crate::wallet_db::HistoryRecord> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    records
        .into_iter()
        .map(|r| crate::wallet_db::HistoryRecord {
            id: r.id,
            wallet_id: r.wallet_id,
            chain_name: r.chain_name,
            tx_hash: r.tx_hash,
            created_at: r.created_at,
            payload: engine.encode(r.payload_json.as_bytes()),
        })
        .collect()
}

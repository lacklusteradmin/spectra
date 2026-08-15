//! Parsing, scaling and SQLite plumbing shared by the three owners. None of
//! this is exported.
//!
//! ## Error message convention
//!
//! Errors raised here read `"<context>: <reason>"`, where the context names
//! the offending field (`private_key_hex`, `planck`, `chain_id`) and the
//! reason names the failure (`hex decode: …`, `wrong length: …`). The field
//! name comes first so the most diagnostic part survives a truncated log line.

use super::*;

use serde::Serialize;

// ── Chain ID lookup ───────────────────────────────────────────────────────

pub(super) fn chain_for_id(chain_id: &str) -> Result<Chain, SpectraBridgeError> {
    Chain::from_str_id(chain_id)
        .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain_id: {chain_id}")))
}

pub(super) fn chain_for_evm_id(chain_id: &str) -> Result<Chain, SpectraBridgeError> {
    Chain::from_str_id(chain_id)
        .filter(|c| c.is_evm())
        .ok_or_else(|| SpectraBridgeError::from(format!("unsupported EVM chain_id: {chain_id}")))
}

/// Serialize a value to JSON, returning the bridge error type directly.
/// Used by chain dispatch arms whose FFI signature is
/// `Result<String, SpectraBridgeError>`. New endpoints should return a
/// typed `#[derive(uniffi::Record)]` value directly rather than going
/// through this helper.
pub(super) fn json_response<T: Serialize>(value: &T) -> Result<String, SpectraBridgeError> {
    serde_json::to_string(value).map_err(Into::into)
}

/// Parse `params` as a typed payload. Lets a chain dispatch arm collapse
/// six lines of ad-hoc `params["x"].as_y()` extraction into one decode step:
///
/// ```ignore
/// let p: PolkadotSendParams = parse_params(&params)?;
/// ```
///
/// Serde populates the error message with the offending field name, so each
/// missing-field error is "<chain>: missing field `planck` at line N column M"
/// instead of an inscrutable `"missing planck"`.
pub(super) fn parse_params<T: for<'de> serde::Deserialize<'de>>(
    params: &serde_json::Value,
) -> Result<T, SpectraBridgeError> {
    serde_json::from_value(params.clone())
        .map_err(|e| SpectraBridgeError::from(format!("invalid params: {e}")))
}

/// Decode a hex string of an exact byte length. Replaces repeated
/// per-arm hex decoding and fixed-length conversion boilerplate. Includes
/// the field name in both the
/// "not hex" and "wrong length" error variants.
pub(super) fn decode_hex_array<const N: usize>(
    hex_str: &str,
    field_name: &str,
) -> Result<[u8; N], SpectraBridgeError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| SpectraBridgeError::from(format!("{field_name} hex decode: {e}")))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        SpectraBridgeError::from(format!(
            "{field_name} wrong length: expected {N} bytes, got {}",
            v.len()
        ))
    })
}

// ── Decimal scaling ───────────────────────────────────────────────────────

/// Format a smallest-unit `u128` amount as a fixed-decimal string. Used for
/// chains whose typed balance struct doesn't already provide a `_display` field.
pub(crate) fn format_units(raw: u128, decimals: u32, max_frac: usize) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let scale = 10u128.pow(decimals);
    let whole = raw / scale;
    let frac = raw % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    if trimmed.is_empty() {
        return whole.to_string();
    }
    let capped = if trimmed.len() > max_frac {
        &trimmed[..max_frac]
    } else {
        trimmed
    };
    format!("{whole}.{capped}")
}

pub(super) fn format_smallest_unit_decimal(amount: u128, decimals: u32) -> String {
    format_units(amount, decimals, usize::MAX)
}

/// Scale a raw integer by `10^decimals` into a human-readable decimal string
/// with up to 6 fractional digits of precision.
pub(super) fn format_decimals(raw: u128, decimals: u8) -> String {
    format_units(raw, decimals as u32, 6)
}

// ── Balance JSON parsing ──────────────────────────────────────────────────

/// Extract the normalised native balance (in display units) from a balance
/// JSON value. Returns 0.0 for unknown / unsupported chains.
pub(super) fn simple_chain_balance_display(chain_id: &str, obj: &serde_json::Value) -> f64 {
    let u64_field = |key: &str| -> f64 {
        obj[key]
            .as_u64()
            .map(|n| n as f64)
            .or_else(|| {
                obj[key]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|n| n as f64)
            })
            .unwrap_or(0.0)
    };
    let i64_field = |key: &str| -> f64 {
        obj[key]
            .as_i64()
            .map(|n| n as f64)
            .or_else(|| {
                obj[key]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|n| n as f64)
            })
            .unwrap_or(0.0)
    };
    let Some(chain) = Chain::from_str_id(chain_id) else {
        return 0.0;
    };
    let factor = 10f64.powi(chain.native_decimals() as i32);
    match chain {
        Chain::Stellar => i64_field("stroops") / factor,
        Chain::Polkadot => {
            obj["planck"]
                .as_u64()
                .map(|n| n as f64)
                .or_else(|| obj["planck"].as_str().and_then(|s| s.parse::<f64>().ok()))
                .unwrap_or(0.0)
                / factor
        }
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

// ── Fee preview JSON shapes ───────────────────────────────────────────────

/// Flat struct for direct serialization — avoids the intermediate
/// `serde_json::Value` + `Map` heap allocation that `json!()` would produce.
#[derive(Serialize)]
pub(crate) struct FeePreview<'a> {
    chain_id: &'a str,
    native_fee_raw: &'a str,
    native_fee_display: &'a str,
    unit: &'a str,
    source: &'a str,
}

pub(super) fn fee_preview(
    chain_id: &str,
    raw: u128,
    decimals: u8,
    unit: &str,
    source: &str,
) -> String {
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

pub(super) fn fee_preview_str(
    chain_id: &str,
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
    use crate::send::chains::evm::AccessListEntry;

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
        .or_else(|| {
            params["max_priority_fee_per_gas_wei"]
                .as_u64()
                .map(|n| n as u128)
        });
    let calldata = params["calldata_hex"]
        .as_str()
        .and_then(|s| hex::decode(s.trim_start_matches("0x")).ok());
    let sign_only = params["sign_only"].as_bool().unwrap_or(false);
    let access_list = params["access_list_json"]
        .as_str()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|entry| {
                    let addr_str = entry["address"].as_str()?;
                    let addr_bytes = hex::decode(addr_str.trim_start_matches("0x")).ok()?;
                    let addr: [u8; 20] = addr_bytes.try_into().ok()?;
                    let keys = entry["storageKeys"]
                        .as_array()
                        .map(|ks| {
                            ks.iter()
                                .filter_map(|k| {
                                    let kb =
                                        hex::decode(k.as_str()?.trim_start_matches("0x")).ok()?;
                                    kb.try_into().ok()
                                })
                                .collect::<Vec<[u8; 32]>>()
                        })
                        .unwrap_or_default();
                    Some(AccessListEntry {
                        address: addr,
                        storage_keys: keys,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let gas_buffer_pct = params["gas_buffer_pct"].as_u64().map(|n| n as u32);

    crate::send::chains::evm::EvmSendOverrides {
        nonce,
        max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei,
        gas_limit,
        calldata,
        sign_only,
        access_list,
        gas_buffer_pct,
    }
}

// ── SQLite blocking helpers ───────────────────────────────────────────────
//
// Key/value `state` table backing AppState persistence (wallets, settings,
// fiat rates, live prices, etc.). Mirrors the `with_conn` pool already in
// `store/wallet_db.rs` — re-uses a single `Connection` per `db_path` instead
// of opening + running DDL + closing on every load/save. With ~5–10 persists
// per refresh cycle, the previous open-per-call cost was meaningful.
//
// PRAGMAs applied once per connection:
//   - `journal_mode = WAL`     concurrent reads while a write is in flight
//   - `synchronous  = NORMAL`  fsync only at checkpoint, ~5× faster writes
//                              (still durable; only loses ms on power loss)
//   - `temp_store   = MEMORY`  query temp tables don't hit disk

use parking_lot::Mutex as PlMutex;

pub(crate) static SQLITE_POOL: std::sync::LazyLock<PlMutex<HashMap<String, rusqlite::Connection>>> =
    std::sync::LazyLock::new(|| PlMutex::new(HashMap::new()));

pub(crate) fn with_state_conn<T>(
    db_path: &str,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    let mut pool = SQLITE_POOL.lock();
    if !pool.contains_key(db_path) {
        pool.insert(db_path.to_string(), open_state_conn(db_path)?);
    }
    f(pool.get(db_path).unwrap())
}

pub(crate) fn open_state_conn(db_path: &str) -> Result<rusqlite::Connection, String> {
    let conn =
        rusqlite::Connection::open(db_path).map_err(|e| format!("sqlite open {db_path}: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         CREATE TABLE IF NOT EXISTS state (
             key      TEXT    PRIMARY KEY,
             value    TEXT    NOT NULL,
             saved_at INTEGER NOT NULL
         );",
    )
    .map_err(|e| format!("sqlite init: {e}"))?;
    Ok(conn)
}

pub(super) fn sqlite_load(db_path: &str, key: &str) -> Result<String, String> {
    with_state_conn(db_path, |conn| {
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
    })
}

pub(super) fn sqlite_save(db_path: &str, key: &str, value: &str) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    with_state_conn(db_path, |conn| {
        conn.execute(
            "INSERT INTO state (key, value, saved_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, saved_at = excluded.saved_at",
            rusqlite::params![key, value, now],
        )
        .map_err(|e| format!("sqlite save: {e}"))?;
        Ok(())
    })
}

// ── State helpers ─────────────────────────────────────────────────────────

/// Return a zero-amount AssetHolding template for the native coin of each
/// chain. Used as the default when the holding doesn't exist yet.
pub(crate) fn native_coin_template(chain_id: &str) -> Option<AssetHolding> {
    let chain = Chain::from_str_id(chain_id)?;
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

/// Returns `true` when `s` starts with a BIP-32 extended public key prefix.
pub(super) fn is_extended_public_key(s: &str) -> bool {
    matches!(
        s.get(..4),
        Some("xpub") | Some("ypub") | Some("zpub") | Some("Ypub") | Some("Zpub")
    )
}

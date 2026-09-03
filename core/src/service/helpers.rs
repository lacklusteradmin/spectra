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

// ── Balance projection ────────────────────────────────────────────────────

/// The native balance in display units, from the typed summary.
///
/// This read a `serde_json::Value` before — `fetch_balance` serialized a
/// chain's typed balance struct to JSON and this dug the number back out by
/// field name, which is why `Chain::native_balance_field` existed: a table of
/// twenty JSON key names (`lamports`, `stroops`, `planck`, `nanotons`, …)
/// whose only job was to undo a serialization that had just happened in the
/// same call. `fetch_native_balance_summary` already returns the same numbers
/// typed, so both the second 24-arm dispatch and the field-name table are
/// gone; what is left is the arithmetic they were wrapped around.
pub(super) fn summary_display_balance(
    chain_id: &str,
    summary: &crate::service::types::NativeBalanceSummary,
) -> f64 {
    let Some(chain) = Chain::from_str_id(chain_id) else {
        return 0.0;
    };
    // NEAR's smallest unit is 10^24 yocto. Dividing that through an f64 loses
    // precision well before the decimal point, so the client's own display
    // string is the better source — the one chain where the old JSON version
    // also preferred `near_display` over dividing `yocto_near` itself.
    if chain == Chain::Near {
        return summary.amount_display.parse::<f64>().unwrap_or(0.0);
    }
    let factor = 10f64.powi(chain.native_decimals() as i32);
    summary
        .smallest_unit
        .parse::<f64>()
        .map(|units| units / factor)
        .unwrap_or(0.0)
}

// ── Fee estimate ──────────────────────────────────────────────────────────

/// A chain's fee, quoted in that chain's own native unit.
///
/// Was a `FeePreview<'a>` serialized to JSON by `fee_preview` /
/// `fee_preview_str` so that the one caller could parse it back and read
/// three of its fields by name. The struct is the value now; nothing
/// serializes it on the way between two functions in the same call.
///
/// Two of `FeePreview`'s five fields are gone with the JSON: `chain_id` and
/// `unit` were serialized on every call and read by nobody.
#[derive(Debug, Clone)]
pub(crate) struct NativeFeeEstimate {
    /// Smallest units, as a decimal string — some chains' fees do not fit u64.
    pub raw: String,
    /// The same amount at display scale.
    pub display: String,
    /// `"rpc"` when a node quoted it, `"static"` when the catalog did.
    pub source: &'static str,
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

#[cfg(test)]
mod display_balance_from_a_typed_summary {
    use super::summary_display_balance;
    use crate::service::types::NativeBalanceSummary;

    fn summary(smallest: &str, display: &str) -> NativeBalanceSummary {
        NativeBalanceSummary {
            smallest_unit: smallest.to_string(),
            amount_display: display.to_string(),
            utxo_count: 0,
        }
    }

    /// Every chain divides its smallest unit by its own decimals — the same
    /// arithmetic the JSON version did after digging the number out by field
    /// name. The point of this test is that the *numbers* did not move when
    /// the field-name table went away.
    #[test]
    fn each_chain_divides_by_its_own_decimals() {
        // (chain_id, smallest unit, expected display)
        for (chain_id, smallest, expected) in [
            ("solana", "1500000000", 1.5),           // 9 decimals, was "lamports"
            ("stellar", "15000000", 1.5),            // 7 decimals, was "stroops"
            ("polkadot", "12500000000", 1.25),       // 10 decimals, was "planck"
            ("bitcoin", "150000000", 1.5),           // 8 decimals, was "confirmed_sats"
            ("ton", "1500000000", 1.5),              // 9 decimals, was "nanotons"
            ("cardano", "1500000", 1.5),             // 6 decimals, was "lovelace"
            ("tron", "1500000", 1.5),                // 6 decimals, was "sun"
        ] {
            let got = summary_display_balance(chain_id, &summary(smallest, "ignored"));
            assert!(
                (got - expected).abs() < 1e-9,
                "{chain_id}: {smallest} -> {got}, expected {expected}"
            );
        }
    }

    /// NEAR is the exception, and it is the reason the exception exists:
    /// 10^24 yocto does not survive an f64 division intact, so the client's
    /// own display string is read instead — exactly what the JSON version
    /// did when it preferred `near_display` over dividing `yocto_near`.
    #[test]
    fn near_reads_the_display_string_rather_than_dividing_yocto() {
        let s = summary("100000000000000000000000", "0.1");
        assert_eq!(summary_display_balance("near", &s), 0.1);
        // And the smallest-unit path would not have produced it exactly.
        let divided = 1e23 / 10f64.powi(24);
        assert!(
            (divided - 0.1).abs() > 0.0 || divided != 0.1,
            "if f64 division were exact here the special case would be unnecessary"
        );
    }

    /// An unknown chain is 0.0, not a panic — same as before.
    #[test]
    fn an_unknown_chain_is_zero() {
        assert_eq!(summary_display_balance("not-a-chain", &summary("100", "1")), 0.0);
    }
}

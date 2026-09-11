use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::registry::Chain;

// ── Normalized chain history — standard output from fetch_normalized_history_json

/// A chain history entry normalized to a standard format that Swift can map
/// directly to `CoreTransactionRecord` without any chain-specific parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainHistoryEntry {
    pub kind: String,   // "receive" | "send"
    pub status: String, // "confirmed" | "pending"
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub amount: f64,
    pub counterparty: String,
    pub tx_hash: String,
    pub block_height: Option<i64>,
    pub timestamp: f64, // Unix seconds
}
/// Where the transaction id lives. ICP has no hash — it identifies a transfer
/// by ledger block index, which arrives as a number.
enum HashField {
    Text(&'static str),
    Number(&'static str),
}

/// What an entry means when it carries no `is_incoming` flag. The UTXO clients
/// signal direction by the sign of the net amount instead.
enum DirectionFallback {
    Outgoing,
    AmountSign,
}

enum StatusRule {
    /// The chain only reports finalized transfers.
    AlwaysConfirmed,
    /// Esplora's `confirmed` boolean.
    ConfirmedFlag,
    /// Blockbook reports height 0 (or none) until the transaction is mined.
    ConfirmedWhenMined,
}

/// Whether a row may name an asset other than the chain's native one.
enum SymbolOverride {
    /// Native asset only.
    None,
    /// A row's `symbol` names its own asset — as Solana's SPL and Tron's
    /// TRC-20 transfers do. The display name is the token catalog's, and the
    /// ticker itself for a token the catalog does not carry.
    RowNamesAsset,
}

/// Where one chain's history JSON keeps the fields a normalized entry needs,
/// and what units they arrive in.
///
/// This was fifteen match arms of twenty near-identical lines: they differed
/// in field names, in one divisor, and in nothing else, while each re-derived
/// the same `if is_incoming { "receive" } else { "send" }`. A chain with no arm
/// of its own returned an empty history however well its fetch had gone, which
/// is what every UTXO testnet did.
struct HistoryShape {
    hash: HashField,
    /// Field holding the amount.
    amount: &'static str,
    /// Whether `amount` is in the chain's smallest unit — satoshis, wei,
    /// planck — and so has to be divided by the native factor. Solana and Tron
    /// hand over a display-unit string that is already divided.
    amount_in_base_units: bool,
    /// Timestamp field, and what divides it into Unix seconds.
    time: &'static str,
    time_divisor: f64,
    direction_fallback: DirectionFallback,
    status: StatusRule,
    block_height: Option<&'static str>,
    /// Fields naming the other party, as `(when incoming, when outgoing)`.
    counterparty: Option<(&'static str, &'static str)>,
    symbol_override: SymbolOverride,
}

impl HistoryShape {
    /// The shape shared by every chain that reports finalized native transfers
    /// with second-resolution timestamps — the majority, which then override
    /// only what they actually do differently.
    const fn confirmed_native(amount: &'static str) -> Self {
        Self {
            hash: HashField::Text("txid"),
            amount,
            amount_in_base_units: true,
            time: "timestamp",
            time_divisor: 1.0,
            direction_fallback: DirectionFallback::Outgoing,
            status: StatusRule::AlwaysConfirmed,
            block_height: None,
            counterparty: None,
            symbol_override: SymbolOverride::None,
        }
    }

    const fn with_counterparty(mut self, incoming: &'static str, outgoing: &'static str) -> Self {
        self.counterparty = Some((incoming, outgoing));
        self
    }

    const fn with_time(mut self, field: &'static str, divisor: f64) -> Self {
        self.time = field;
        self.time_divisor = divisor;
        self
    }
}

/// A testnet reads its mainnet's shape: the client behind it is the same code
/// returning the same JSON.
fn history_shape(chain: Chain) -> Option<HistoryShape> {
    use DirectionFallback::AmountSign;
    use StatusRule::{ConfirmedFlag, ConfirmedWhenMined};

    let shape = match chain.mainnet_counterpart() {
        // Esplora: {txid, confirmed, block_height, block_time, net_sats}
        Chain::Bitcoin => HistoryShape {
            direction_fallback: AmountSign,
            status: ConfirmedFlag,
            block_height: Some("block_height"),
            ..HistoryShape::confirmed_native("net_sats").with_time("block_time", 1.0)
        },

        // Blockbook: {txid, amount_sat, block_height, timestamp, is_incoming}
        Chain::Litecoin | Chain::BitcoinCash | Chain::BitcoinSV => HistoryShape {
            direction_fallback: AmountSign,
            status: ConfirmedWhenMined,
            block_height: Some("block_height"),
            ..HistoryShape::confirmed_native("amount_sat")
        },

        // Dogecoin reports the same shape under a name of its own.
        Chain::Dogecoin => HistoryShape {
            direction_fallback: AmountSign,
            status: ConfirmedWhenMined,
            block_height: Some("block_height"),
            ..HistoryShape::confirmed_native("amount_koin")
        },

        Chain::Xrp => {
            HistoryShape::confirmed_native("amount_drops").with_counterparty("from", "to")
        }

        // Stellar's timestamp may be ISO-8601 rather than a number; every
        // shape accepts either.
        Chain::Stellar => {
            HistoryShape::confirmed_native("amount_stroops").with_counterparty("from", "to")
        }

        Chain::Cardano => {
            HistoryShape::confirmed_native("amount_lovelace").with_time("block_time", 1.0)
        }

        Chain::Polkadot => {
            HistoryShape::confirmed_native("amount_planck").with_counterparty("from", "to")
        }

        // SPL transfers ride the same feed as native ones, carrying their own
        // symbol and an amount already in display units.
        Chain::Solana => HistoryShape {
            hash: HashField::Text("signature"),
            amount_in_base_units: false,
            symbol_override: SymbolOverride::RowNamesAsset,
            ..HistoryShape::confirmed_native("amount_display").with_counterparty("from", "to")
        },

        // TRC20 transfers, likewise.
        Chain::Tron => HistoryShape {
            amount_in_base_units: false,
            symbol_override: SymbolOverride::RowNamesAsset,
            ..HistoryShape::confirmed_native("amount_display")
                .with_counterparty("from", "to")
                .with_time("timestamp_ms", 1e3)
        },

        Chain::Sui => HistoryShape {
            hash: HashField::Text("digest"),
            ..HistoryShape::confirmed_native("amount_mist").with_time("timestamp_ms", 1e3)
        },

        Chain::Aptos => HistoryShape::confirmed_native("amount_octas")
            .with_counterparty("from", "to")
            .with_time("timestamp_us", 1e6),

        Chain::Ton => {
            HistoryShape::confirmed_native("amount_nanotons").with_counterparty("from", "to")
        }

        Chain::Near => HistoryShape::confirmed_native("amount_yocto")
            .with_counterparty("signer_id", "receiver_id")
            .with_time("timestamp_ns", 1e9),

        Chain::Icp => HistoryShape {
            hash: HashField::Number("block_index"),
            ..HistoryShape::confirmed_native("amount_e8s")
                .with_counterparty("from", "to")
                .with_time("timestamp_ns", 1e9)
        },

        Chain::Monero => HistoryShape::confirmed_native("amount_piconeros"),

        // Every EVM chain. `EvmHistoryEntry` is one shape for all of them,
        // which is why this is a guard rather than twenty-three names.
        //
        // There was no arm here at all: fifteen chains had one and the EVM
        // family fell to `_ => vec![]`, so `fetch_normalized_history` returned
        // nothing for Ethereum and every chain like it however well the fetch
        // itself had gone. It was invisible while the fetch was also returning
        // nothing — the explorer refusing without an API key — and only shows
        // up once that is fixed.
        c if c.is_evm() => HistoryShape {
            block_height: Some("block_number"),
            ..HistoryShape::confirmed_native("value_wei").with_counterparty("from", "to")
        },

        _ => return None,
    };
    Some(shape)
}

/// A JSON number, or a decimal string holding one. NEAR's yocto amounts and
/// every EVM `value_wei` overflow an `f64`'s integer range and so arrive as
/// strings.
fn json_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// Convert a raw history JSON string (as returned by `fetch_history`) into
/// normalized `ChainHistoryEntry` records that Swift can consume without
/// any chain-specific parsing logic.
pub fn normalize_chain_history(chain_id: &str, raw_json: &str) -> Vec<ChainHistoryEntry> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(raw_json) else {
        return vec![];
    };
    let Some(chain) = Chain::from_str_id(chain_id) else {
        return vec![];
    };
    let Some(shape) = history_shape(chain) else {
        return vec![];
    };

    let (asset_name, symbol, chain_name) = (
        chain.coin_name(),
        chain.coin_symbol(),
        chain.chain_display_name(),
    );
    let factor = 10f64.powi(chain.native_decimals() as i32);

    entries
        .iter()
        .filter_map(|entry| {
            let tx_hash = match shape.hash {
                HashField::Text(field) => entry[field].as_str()?.to_string(),
                HashField::Number(field) => entry[field].as_i64().unwrap_or(0).to_string(),
            };

            // Signed while direction is still being decided; the entry itself
            // reports a magnitude and says which way it went in `kind`.
            let signed_amount = json_number(&entry[shape.amount]).unwrap_or(0.0);
            let is_incoming =
                entry["is_incoming"]
                    .as_bool()
                    .unwrap_or(match shape.direction_fallback {
                        DirectionFallback::Outgoing => false,
                        DirectionFallback::AmountSign => signed_amount >= 0.0,
                    });

            let block_height = shape.block_height.and_then(|field| entry[field].as_i64());
            let status = match shape.status {
                StatusRule::AlwaysConfirmed => "confirmed",
                StatusRule::ConfirmedFlag => {
                    if entry["confirmed"].as_bool().unwrap_or(false) {
                        "confirmed"
                    } else {
                        "pending"
                    }
                }
                StatusRule::ConfirmedWhenMined => {
                    if block_height.unwrap_or(0) > 0 {
                        "confirmed"
                    } else {
                        "pending"
                    }
                }
            };

            let (entry_asset, entry_symbol) = match shape.symbol_override {
                SymbolOverride::None => (asset_name, symbol),
                SymbolOverride::RowNamesAsset => {
                    let found = entry["symbol"].as_str().unwrap_or(symbol);
                    if found == symbol {
                        (asset_name, found)
                    } else {
                        let named = crate::tokens::token_name_on_chain(chain.str_id(), found);
                        (named.unwrap_or(found), found)
                    }
                }
            };

            let raw_time = &entry[shape.time];
            // A number is in the shape's own unit — `timestamp_ms`,
            // `timestamp_ns` — and needs the divisor. A string is RFC 3339 and
            // parses straight to seconds, so applying the divisor to it too
            // would put a nanosecond chain's dates in 1970. An unreadable
            // stamp still yields the row: a transaction with a wrong date is
            // worth more than a transaction the history does not show.
            let timestamp = match raw_time.as_f64() {
                Some(units) => units / shape.time_divisor,
                None => raw_time
                    .as_str()
                    .and_then(parse_iso8601_timestamp)
                    .unwrap_or(0.0),
            };

            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: status.to_string(),
                asset_name: entry_asset.to_string(),
                symbol: entry_symbol.to_string(),
                chain_name: chain_name.to_string(),
                amount: if shape.amount_in_base_units {
                    signed_amount.abs() / factor
                } else {
                    signed_amount.abs()
                },
                counterparty: shape
                    .counterparty
                    .map(|(incoming, outgoing)| {
                        entry[if is_incoming { incoming } else { outgoing }]
                            .as_str()
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
                    .to_string(),
                tx_hash,
                block_height,
                timestamp,
            })
        })
        .collect()
}

/// Parse an RFC 3339 / ISO-8601 timestamp to Unix seconds, or `None` when the
/// string is not one. The caller decides what an unreadable stamp becomes.
///
/// Hand-rolled to keep a date-time crate out of the tree, and hand-rolled
/// carefully — the parser this replaces got two things wrong that a provider's
/// response could reach:
///
/// - it indexed the string by byte offset without checking char boundaries, so
///   any non-ASCII character in a response of nineteen bytes or more panicked
///   inside core rather than failing to parse;
/// - it read every stamp as UTC, including the `+00:00` form its own comment
///   claimed to support, so an offset stamp landed hours away from its instant.
///
/// It also answered `0.0` for anything it could not read, which is a real date
/// and not a refusal.
fn parse_iso8601_timestamp(s: &str) -> Option<f64> {
    let s = s.trim();
    // Every byte index below is sound exactly because of this check: a
    // timestamp is ASCII by definition, and anything else is not one.
    if !s.is_ascii() {
        return None;
    }
    let bytes = s.as_bytes();
    // `YYYY-MM-DDTHH:MM:SS` is the shortest form read. Fractional seconds and
    // a zone offset may follow, and are handled by `zone_offset_seconds`.
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let field = |range: std::ops::Range<usize>| -> Option<i64> {
        let text = s.get(range)?;
        // `parse` would take a sign, and `+123-01-01…` is not a date.
        text.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| text.parse().ok())
            .flatten()
    };
    let year = field(0..4)?;
    let month = field(5..7)?;
    let day = field(8..10)?;
    let hour = field(11..13)?;
    let minute = field(14..16)?;
    let second = field(17..19)?;
    // Ranges, so a malformed field cannot roll the date somewhere plausible.
    // Second 60 stays in: a leap second is a real reading.
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let wall = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    // The fields above are a wall clock in the stamp's own zone; the offset is
    // how far that zone runs ahead of UTC.
    Some((wall - zone_offset_seconds(&s[19..])?) as f64)
}

/// How far the stamp's zone runs ahead of UTC, in seconds.
///
/// Accepts `Z`, an empty suffix (both UTC) and `±HH:MM` / `±HHMM` / `±HH`,
/// each optionally preceded by fractional seconds. Anything else means the
/// string was not the timestamp it looked like, so it is `None` rather than a
/// silent zero.
fn zone_offset_seconds(suffix: &str) -> Option<i64> {
    // Fractional seconds sit between the seconds field and the zone. Their
    // precision is below what a history row renders, so they are skipped.
    let suffix = match suffix.strip_prefix('.') {
        Some(rest) => rest.trim_start_matches(|c: char| c.is_ascii_digit()),
        None => suffix,
    };
    if suffix.is_empty() || suffix.eq_ignore_ascii_case("Z") {
        return Some(0);
    }
    let (sign, body) = match suffix.as_bytes()[0] {
        b'+' => (1, &suffix[1..]),
        b'-' => (-1, &suffix[1..]),
        _ => return None,
    };
    let digits: String = body.chars().filter(|c| *c != ':').collect();
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes) = match digits.len() {
        2 => (digits.parse::<i64>().ok()?, 0),
        4 => (
            digits[..2].parse::<i64>().ok()?,
            digits[2..].parse::<i64>().ok()?,
        ),
        _ => return None,
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWallet {
    pub wallet_id: String,
    pub selected_chain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTransaction {
    pub id: String,
    pub wallet_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub address: String,
    pub transaction_hash: Option<String>,
    pub transaction_history_source: Option<String>,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeHistoryRequest {
    pub wallets: Vec<HistoryWallet>,
    pub transactions: Vec<HistoryTransaction>,
    pub unknown_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreNormalizedHistoryEntry {
    pub id: String,
    pub transaction_id: String,
    pub dedupe_key: String,
    pub created_at_unix: f64,
    pub kind: String,
    pub status: String,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub address: String,
    pub transaction_hash: Option<String>,
    pub source_tag: String,
    pub provider_count: u64,
    pub search_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreBitcoinHistorySnapshot {
    pub txid: String,
    pub amount_btc: f64,
    pub kind: String,
    pub status: String,
    pub counterparty_address: String,
    pub block_height: Option<i64>,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MergeBitcoinHistorySnapshotsRequest {
    pub snapshots: Vec<CoreBitcoinHistorySnapshot>,
    pub owned_addresses: Vec<String>,
    pub limit: u64,
}

pub fn normalize_history(request: NormalizeHistoryRequest) -> Vec<CoreNormalizedHistoryEntry> {
    let wallet_by_id = request
        .wallets
        .into_iter()
        .map(|wallet| (wallet.wallet_id, wallet.selected_chain))
        .collect::<BTreeMap<_, _>>();

    let mut grouped_by_dedupe_key = BTreeMap::<String, Vec<CoreNormalizedHistoryEntry>>::new();
    for transaction in request.transactions {
        let Some(wallet_id) = transaction.wallet_id.as_ref() else {
            continue;
        };
        let Some(selected_chain) = wallet_by_id.get(wallet_id) else {
            continue;
        };
        if *selected_chain != transaction.chain_name {
            continue;
        }
        let entry = normalized_entry(transaction, &request.unknown_label);
        grouped_by_dedupe_key
            .entry(entry.dedupe_key.clone())
            .or_default()
            .push(entry);
    }

    let mut deduped = grouped_by_dedupe_key
        .into_values()
        .filter_map(|entries| {
            if entries.is_empty() {
                return None;
            }
            let provider_count = entries
                .iter()
                .map(|entry| entry.source_tag.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                .max(1) as u64;
            let best = entries.into_iter().max_by(compare_entries)?;
            Some(CoreNormalizedHistoryEntry {
                provider_count,
                ..best
            })
        })
        .collect::<Vec<_>>();

    deduped.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| lhs.id.cmp(&rhs.id))
    });
    deduped
}

pub fn merge_bitcoin_history_snapshots(
    request: MergeBitcoinHistorySnapshotsRequest,
) -> Vec<CoreBitcoinHistorySnapshot> {
    let owned_addresses = request
        .owned_addresses
        .into_iter()
        .map(|address| address.trim().to_lowercase())
        .filter(|address| !address.is_empty())
        .collect::<std::collections::BTreeSet<_>>();

    let grouped = request.snapshots.into_iter().fold(
        BTreeMap::<String, Vec<CoreBitcoinHistorySnapshot>>::new(),
        |mut grouped, snapshot| {
            grouped
                .entry(snapshot.txid.clone())
                .or_default()
                .push(snapshot);
            grouped
        },
    );

    let mut merged = grouped
        .into_values()
        .filter_map(|entries| merge_bitcoin_snapshot_group(entries, &owned_addresses))
        .collect::<Vec<_>>();

    merged.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| lhs.txid.cmp(&rhs.txid))
    });
    merged.truncate(request.limit.max(1) as usize);
    merged
}

fn normalized_entry(
    transaction: HistoryTransaction,
    unknown_label: &str,
) -> CoreNormalizedHistoryEntry {
    let wallet_key = transaction
        .wallet_id
        .clone()
        .unwrap_or_else(|| "unknown-wallet".to_string())
        .to_lowercase();
    let normalized_chain = transaction.chain_name.to_lowercase();
    let normalized_symbol = transaction.symbol.trim().to_lowercase();
    let (dedupe_key, stable_id) = match transaction
        .transaction_hash
        .as_ref()
        .map(|value| value.to_lowercase())
        .filter(|value| !value.is_empty())
    {
        Some(transaction_hash) => {
            let key = format!(
                "{}|{}|{}|{}",
                wallet_key, normalized_chain, normalized_symbol, transaction_hash
            );
            (key.clone(), key)
        }
        None => {
            let key = format!("local|{}|{}", wallet_key, transaction.id.to_lowercase());
            (key.clone(), key)
        }
    };

    let source_tag = normalized_source_tag(
        transaction.transaction_history_source.as_deref(),
        unknown_label,
    );
    let search_index = [
        transaction.wallet_name.as_str(),
        transaction.asset_name.as_str(),
        transaction.symbol.as_str(),
        transaction.chain_name.as_str(),
        transaction.address.as_str(),
        transaction.transaction_hash.as_deref().unwrap_or(""),
        source_tag.as_str(),
    ]
    .join(" ")
    .to_lowercase();

    CoreNormalizedHistoryEntry {
        id: stable_id,
        transaction_id: transaction.id,
        dedupe_key,
        created_at_unix: transaction.created_at_unix,
        kind: transaction.kind,
        status: transaction.status,
        wallet_name: transaction.wallet_name,
        asset_name: transaction.asset_name,
        symbol: transaction.symbol,
        chain_name: transaction.chain_name,
        address: transaction.address,
        transaction_hash: transaction.transaction_hash,
        source_tag,
        provider_count: 1,
        search_index,
    }
}

fn merge_bitcoin_snapshot_group(
    entries: Vec<CoreBitcoinHistorySnapshot>,
    owned_addresses: &std::collections::BTreeSet<String>,
) -> Option<CoreBitcoinHistorySnapshot> {
    if entries.is_empty() {
        return None;
    }

    let net_amount = entries.iter().fold(0.0, |amount, entry| {
        amount
            + if entry.kind == "receive" {
                entry.amount_btc
            } else {
                -entry.amount_btc
            }
    });
    if net_amount == 0.0 {
        return None;
    }

    let mut ordered_entries = entries;
    ordered_entries.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                rhs.amount_btc
                    .partial_cmp(&lhs.amount_btc)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let counterparty_address = ordered_entries
        .iter()
        .map(|entry| entry.counterparty_address.trim().to_string())
        .find(|address| {
            let normalized = address.to_lowercase();
            !normalized.is_empty() && !owned_addresses.contains(&normalized)
        })
        .or_else(|| {
            ordered_entries
                .first()
                .map(|entry| entry.counterparty_address.clone())
        })
        .unwrap_or_default();

    let representative = ordered_entries.first()?.clone();
    Some(CoreBitcoinHistorySnapshot {
        txid: representative.txid,
        amount_btc: net_amount.abs(),
        kind: if net_amount > 0.0 {
            "receive".to_string()
        } else {
            "send".to_string()
        },
        status: if ordered_entries
            .iter()
            .any(|entry| entry.status == "pending")
        {
            "pending".to_string()
        } else {
            "confirmed".to_string()
        },
        counterparty_address,
        block_height: ordered_entries
            .iter()
            .filter_map(|entry| entry.block_height)
            .max(),
        created_at_unix: ordered_entries
            .iter()
            .map(|entry| entry.created_at_unix)
            .fold(representative.created_at_unix, f64::max),
    })
}

fn compare_entries(
    lhs: &CoreNormalizedHistoryEntry,
    rhs: &CoreNormalizedHistoryEntry,
) -> std::cmp::Ordering {
    status_rank(&lhs.status)
        .cmp(&status_rank(&rhs.status))
        .then_with(|| {
            lhs.created_at_unix
                .partial_cmp(&rhs.created_at_unix)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| lhs.transaction_id.cmp(&rhs.transaction_id))
}

fn status_rank(status: &str) -> i32 {
    match status {
        "confirmed" => 3,
        "pending" => 2,
        "failed" => 1,
        _ => 0,
    }
}

fn normalized_source_tag(raw_source: Option<&str>, unknown_label: &str) -> String {
    let trimmed = raw_source.unwrap_or("").trim().to_lowercase();
    if trimmed.is_empty() {
        return unknown_label.to_string();
    }
    match trimmed.as_str() {
        "esplora" => "Esplora".to_string(),
        "litecoinspace" => "LitecoinSpace".to_string(),
        "blockchair" => "Blockchair".to_string(),
        "blockcypher" => "BlockCypher".to_string(),
        "dogecoin.providers" => "DOGE Providers".to_string(),
        "rpc" => "RPC".to_string(),
        "etherscan" => "Etherscan".to_string(),
        "blockscout" => "Blockscout".to_string(),
        "ethplorer" => "Ethplorer".to_string(),
        "none" => unknown_label.to_string(),
        _ => title_case(&trimmed),
    }
}

fn title_case(value: &str) -> String {
    value
        .split_whitespace()
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Not exported: `WalletService::normalized_history` is the entry point: the records it
/// normalizes are core's own.
pub fn core_normalize_history(request: NormalizeHistoryRequest) -> Vec<CoreNormalizedHistoryEntry> {
    normalize_history(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_history_by_hash_and_prefers_confirmed() {
        let entries = normalize_history(NormalizeHistoryRequest {
            wallets: vec![HistoryWallet {
                wallet_id: "wallet-1".to_string(),
                selected_chain: "Ethereum".to_string(),
            }],
            transactions: vec![
                HistoryTransaction {
                    id: "tx-1".to_string(),
                    wallet_id: Some("wallet-1".to_string()),
                    kind: "send".to_string(),
                    status: "pending".to_string(),
                    wallet_name: "Main".to_string(),
                    asset_name: "Ether".to_string(),
                    symbol: "ETH".to_string(),
                    chain_name: "Ethereum".to_string(),
                    address: "0xabc".to_string(),
                    transaction_hash: Some("0xhash".to_string()),
                    transaction_history_source: Some("rpc".to_string()),
                    created_at_unix: 100.0,
                },
                HistoryTransaction {
                    id: "tx-2".to_string(),
                    wallet_id: Some("wallet-1".to_string()),
                    kind: "send".to_string(),
                    status: "confirmed".to_string(),
                    wallet_name: "Main".to_string(),
                    asset_name: "Ether".to_string(),
                    symbol: "ETH".to_string(),
                    chain_name: "Ethereum".to_string(),
                    address: "0xabc".to_string(),
                    transaction_hash: Some("0xhash".to_string()),
                    transaction_history_source: Some("etherscan".to_string()),
                    created_at_unix: 110.0,
                },
            ],
            unknown_label: "Unknown".to_string(),
        });

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "confirmed");
        assert_eq!(entries[0].provider_count, 2);
    }

    #[test]
    fn merges_bitcoin_inventory_snapshots_into_net_entries() {
        let merged = merge_bitcoin_history_snapshots(MergeBitcoinHistorySnapshotsRequest {
            snapshots: vec![
                CoreBitcoinHistorySnapshot {
                    txid: "tx-1".to_string(),
                    amount_btc: 0.75,
                    kind: "receive".to_string(),
                    status: "confirmed".to_string(),
                    counterparty_address: "bc1-owned".to_string(),
                    block_height: Some(100),
                    created_at_unix: 100.0,
                },
                CoreBitcoinHistorySnapshot {
                    txid: "tx-1".to_string(),
                    amount_btc: 0.25,
                    kind: "send".to_string(),
                    status: "pending".to_string(),
                    counterparty_address: "bc1-other".to_string(),
                    block_height: Some(101),
                    created_at_unix: 110.0,
                },
            ],
            owned_addresses: vec!["bc1-owned".to_string()],
            limit: 25,
        });

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].txid, "tx-1");
        assert_eq!(merged[0].amount_btc, 0.5);
        assert_eq!(merged[0].kind, "receive");
        assert_eq!(merged[0].status, "pending");
        assert_eq!(merged[0].counterparty_address, "bc1-other");
        assert_eq!(merged[0].block_height, Some(101));
        assert_eq!(merged[0].created_at_unix, 110.0);
    }
}

// ── FFI surface ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod normalize_chain_history_tests {
    use super::*;

    /// One populated entry per chain shape, in the JSON that chain's client
    /// serializes, against the row it must normalize to. Field names, units
    /// and timestamp scales all live in `history_shape`, so this is where a
    /// wrong one shows up.
    const CASES: &[(&str, &str, &str)] = &[
        (
            "bitcoin",
            r#"[{"txid":"a1","confirmed":true,"block_height":800000,"block_time":1700000000,"net_sats":-12345}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Bitcoin","symbol":"BTC","chain_name":"Bitcoin","amount":0.00012345,"counterparty":"","tx_hash":"a1","block_height":800000,"timestamp":1700000000.0}]"#,
        ),
        (
            "bitcoin",
            r#"[{"txid":"a2","confirmed":false,"block_height":null,"block_time":1700000001,"net_sats":6789}]"#,
            r#"[{"kind":"receive","status":"pending","asset_name":"Bitcoin","symbol":"BTC","chain_name":"Bitcoin","amount":0.00006789,"counterparty":"","tx_hash":"a2","block_height":null,"timestamp":1700000001.0}]"#,
        ),
        (
            "litecoin",
            r#"[{"txid":"b1","amount_sat":-500000,"block_height":250000,"timestamp":1700000002,"is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Litecoin","symbol":"LTC","chain_name":"Litecoin","amount":0.005,"counterparty":"","tx_hash":"b1","block_height":250000,"timestamp":1700000002.0}]"#,
        ),
        (
            "bitcoin-cash",
            r#"[{"txid":"b2","amount_sat":700000,"block_height":0,"timestamp":1700000003}]"#,
            r#"[{"kind":"receive","status":"pending","asset_name":"Bitcoin Cash","symbol":"BCH","chain_name":"Bitcoin Cash","amount":0.007,"counterparty":"","tx_hash":"b2","block_height":0,"timestamp":1700000003.0}]"#,
        ),
        (
            "bitcoin-sv",
            r#"[{"txid":"b3","amount_sat":900000,"block_height":10,"timestamp":1700000004,"is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Bitcoin SV","symbol":"BSV","chain_name":"Bitcoin SV","amount":0.009,"counterparty":"","tx_hash":"b3","block_height":10,"timestamp":1700000004.0}]"#,
        ),
        (
            "dogecoin",
            r#"[{"txid":"c1","amount_koin":-123456789,"block_height":5,"timestamp":1700000005,"is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Dogecoin","symbol":"DOGE","chain_name":"Dogecoin","amount":1.23456789,"counterparty":"","tx_hash":"c1","block_height":5,"timestamp":1700000005.0}]"#,
        ),
        (
            "xrp",
            r#"[{"txid":"d1","timestamp":1700000006,"from":"rFrom","to":"rTo","amount_drops":250000,"is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"XRP","symbol":"XRP","chain_name":"XRP Ledger","amount":0.25,"counterparty":"rFrom","tx_hash":"d1","block_height":null,"timestamp":1700000006.0}]"#,
        ),
        (
            "stellar",
            r#"[{"txid":"e1","timestamp":"2023-11-14T22:13:20Z","from":"GFrom","to":"GTo","amount_stroops":3000000,"is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Stellar","symbol":"XLM","chain_name":"Stellar","amount":0.3,"counterparty":"GTo","tx_hash":"e1","block_height":null,"timestamp":1700000000.0}]"#,
        ),
        (
            "stellar",
            r#"[{"txid":"e2","timestamp":1700000008,"from":"GFrom","to":"GTo","amount_stroops":-4000000,"is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Stellar","symbol":"XLM","chain_name":"Stellar","amount":0.4,"counterparty":"GFrom","tx_hash":"e2","block_height":null,"timestamp":1700000008.0}]"#,
        ),
        (
            "cardano",
            r#"[{"txid":"f1","block_time":1700000009,"amount_lovelace":-5000000,"is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Cardano","symbol":"ADA","chain_name":"Cardano","amount":5.0,"counterparty":"","tx_hash":"f1","block_height":null,"timestamp":1700000009.0}]"#,
        ),
        (
            "polkadot",
            r#"[{"txid":"g1","amount_planck":60000000000.0,"timestamp":1700000010,"from":"5From","to":"5To","is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Polkadot","symbol":"DOT","chain_name":"Polkadot","amount":6.0,"counterparty":"5From","tx_hash":"g1","block_height":null,"timestamp":1700000010.0}]"#,
        ),
        (
            "solana",
            r#"[{"signature":"h1","timestamp":1700000011,"is_incoming":false,"amount_display":"1.25","symbol":"SOL","mint":null,"from":"sFrom","to":"sTo"}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Solana","symbol":"SOL","chain_name":"Solana","amount":1.25,"counterparty":"sTo","tx_hash":"h1","block_height":null,"timestamp":1700000011.0}]"#,
        ),
        (
            "solana",
            r#"[{"signature":"h2","timestamp":1700000012,"is_incoming":true,"amount_display":"42.5","symbol":"USDC","from":"sFrom","to":"sTo"}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"USD Coin","symbol":"USDC","chain_name":"Solana","amount":42.5,"counterparty":"sFrom","tx_hash":"h2","block_height":null,"timestamp":1700000012.0}]"#,
        ),
        (
            "tron",
            r#"[{"txid":"i1","timestamp_ms":1700000013000,"from":"TFrom","to":"TTo","amount_display":"7.5","symbol":"USDT","is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Tether USD","symbol":"USDT","chain_name":"Tron","amount":7.5,"counterparty":"TFrom","tx_hash":"i1","block_height":null,"timestamp":1700000013.0}]"#,
        ),
        (
            "tron",
            r#"[{"txid":"i2","timestamp_ms":1700000014000,"from":"TFrom","to":"TTo","amount_display":"3.5","symbol":"TRX","is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Tron","symbol":"TRX","chain_name":"Tron","amount":3.5,"counterparty":"TTo","tx_hash":"i2","block_height":null,"timestamp":1700000014.0}]"#,
        ),
        // The token catalog names a TRC-20, so every token on the chain has a
        // name. A four-entry table in this file used to, and TrueUSD was one
        // of the ones it did not reach.
        (
            "tron",
            r#"[{"txid":"i3","timestamp_ms":1700000014000,"from":"TFrom","to":"TTo","amount_display":"2.0","symbol":"TUSD","is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"TrueUSD","symbol":"TUSD","chain_name":"Tron","amount":2.0,"counterparty":"TFrom","tx_hash":"i3","block_height":null,"timestamp":1700000014.0}]"#,
        ),
        // A ticker the catalog does not carry stays the ticker: a row nobody
        // can name is still a row, and inventing a name for it would be worse.
        (
            "tron",
            r#"[{"txid":"i4","timestamp_ms":1700000014000,"from":"TFrom","to":"TTo","amount_display":"1.0","symbol":"NOTATOKEN","is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"NOTATOKEN","symbol":"NOTATOKEN","chain_name":"Tron","amount":1.0,"counterparty":"TFrom","tx_hash":"i4","block_height":null,"timestamp":1700000014.0}]"#,
        ),
        (
            "sui",
            r#"[{"digest":"j1","amount_mist":800000000.0,"timestamp_ms":1700000015000,"is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Sui","symbol":"SUI","chain_name":"Sui","amount":0.8,"counterparty":"","tx_hash":"j1","block_height":null,"timestamp":1700000015.0}]"#,
        ),
        (
            "aptos",
            r#"[{"txid":"k1","amount_octas":900000000.0,"timestamp_us":1700000016000000,"from":"aFrom","to":"aTo","is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Aptos","symbol":"APT","chain_name":"Aptos","amount":9.0,"counterparty":"aTo","tx_hash":"k1","block_height":null,"timestamp":1700000016.0}]"#,
        ),
        (
            "ton",
            r#"[{"txid":"l1","amount_nanotons":1000000000.0,"timestamp":1700000017,"from":"tFrom","to":"tTo","is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Toncoin","symbol":"TON","chain_name":"TON","amount":1.0,"counterparty":"tFrom","tx_hash":"l1","block_height":null,"timestamp":1700000017.0}]"#,
        ),
        (
            "near",
            r#"[{"txid":"m1","timestamp_ns":1700000018000000000,"signer_id":"nSigner","receiver_id":"nReceiver","amount_yocto":"1500000000000000000000000","is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"NEAR","symbol":"NEAR","chain_name":"NEAR","amount":1.5,"counterparty":"nReceiver","tx_hash":"m1","block_height":null,"timestamp":1700000018.0}]"#,
        ),
        (
            "internet-computer",
            r#"[{"block_index":42,"amount_e8s":250000000.0,"timestamp_ns":1700000019000000000,"from":"iFrom","to":"iTo","is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Internet Computer","symbol":"ICP","chain_name":"Internet Computer","amount":2.5,"counterparty":"iFrom","tx_hash":"42","block_height":null,"timestamp":1700000019.0}]"#,
        ),
        (
            "monero",
            r#"[{"txid":"o1","amount_piconeros":1250000000000.0,"timestamp":1700000020,"is_incoming":false}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Monero","symbol":"XMR","chain_name":"Monero","amount":1.25,"counterparty":"","tx_hash":"o1","block_height":null,"timestamp":1700000020.0}]"#,
        ),
        (
            "ethereum",
            r#"[{"txid":"p1","is_incoming":true,"value_wei":"1500000000000000000","from":"0xFrom","to":"0xTo","block_number":18000000,"timestamp":1700000021}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Ethereum","symbol":"ETH","chain_name":"Ethereum","amount":1.5,"counterparty":"0xFrom","tx_hash":"p1","block_height":18000000,"timestamp":1700000021.0}]"#,
        ),
        (
            "polygon",
            r#"[{"txid":"p2","is_incoming":false,"value_wei":"250000000000000000","from":"0xFrom","to":"0xTo","block_number":49000000,"timestamp":1700000022}]"#,
            r#"[{"kind":"send","status":"confirmed","asset_name":"Polygon","symbol":"POL","chain_name":"Polygon","amount":0.25,"counterparty":"0xTo","tx_hash":"p2","block_height":49000000,"timestamp":1700000022.0}]"#,
        ),
        (
            "bitcoin-testnet",
            r#"[{"txid":"q1","confirmed":true,"block_height":2500000,"block_time":1700000023,"net_sats":4242}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Bitcoin","symbol":"BTC","chain_name":"Bitcoin Testnet","amount":0.00004242,"counterparty":"","tx_hash":"q1","block_height":2500000,"timestamp":1700000023.0}]"#,
        ),
        (
            "litecoin-testnet",
            r#"[{"txid":"q2","amount_sat":31337,"block_height":9,"timestamp":1700000024,"is_incoming":true}]"#,
            r#"[{"kind":"receive","status":"confirmed","asset_name":"Litecoin","symbol":"LTC","chain_name":"Litecoin Testnet","amount":0.00031337,"counterparty":"","tx_hash":"q2","block_height":9,"timestamp":1700000024.0}]"#,
        ),
    ];

    #[test]
    fn every_chain_shape_normalizes_to_its_expected_row() {
        for (chain, raw, expected) in CASES {
            let rows = normalize_chain_history(chain, raw);
            assert_eq!(
                &serde_json::to_string(&rows).unwrap(),
                expected,
                "{chain} normalized differently"
            );
        }
    }

    /// A testnet's client is its mainnet's client returning the same JSON, so
    /// its history has to normalize the same way. Every UTXO testnet used to
    /// fall through to an empty result.
    #[test]
    fn testnets_normalize_like_their_mainnets() {
        for (testnet, mainnet) in [
            ("bitcoin-testnet", "bitcoin"),
            ("bitcoin-testnet-4", "bitcoin"),
            ("bitcoin-signet", "bitcoin"),
            ("litecoin-testnet", "litecoin"),
            ("bitcoin-cash-testnet", "bitcoin-cash"),
            ("dogecoin-testnet", "dogecoin"),
            ("ethereum-sepolia", "ethereum"),
        ] {
            let raw = CASES
                .iter()
                .find(|(chain, _, _)| *chain == mainnet)
                .map(|(_, raw, _)| *raw)
                .expect("mainnet fixture");
            let rows = normalize_chain_history(testnet, raw);
            assert_eq!(rows.len(), 1, "{testnet} normalized nothing");
            let mainnet_rows = normalize_chain_history(mainnet, raw);
            assert_eq!(rows[0].kind, mainnet_rows[0].kind);
            assert_eq!(rows[0].amount, mainnet_rows[0].amount);
            assert_eq!(rows[0].tx_hash, mainnet_rows[0].tx_hash);
            assert_eq!(rows[0].timestamp, mainnet_rows[0].timestamp);
        }
    }

    /// `amount` is a magnitude and `kind` says which way the transfer went.
    /// The sign of the raw field only decides direction where the chain sends
    /// no `is_incoming` flag.
    #[test]
    fn amounts_are_magnitudes_whatever_sign_the_chain_reports() {
        let negative = r#"[{"txid":"x","amount_planck":-60000000000.0,"timestamp":1,"from":"a","to":"b","is_incoming":true}]"#;
        let rows = normalize_chain_history("polkadot", negative);
        assert_eq!(rows[0].amount, 6.0);
        assert_eq!(rows[0].kind, "receive");

        let unsigned = r#"[{"txid":"y","net_sats":-500,"confirmed":true,"block_time":1}]"#;
        let rows = normalize_chain_history("bitcoin", unsigned);
        assert_eq!(rows[0].amount, 0.000005);
        assert_eq!(
            rows[0].kind, "send",
            "no is_incoming flag: the sign decides"
        );
    }

    /// An unknown chain, unparsable JSON, or a JSON document that is not an
    /// array all yield nothing rather than a panic.
    #[test]
    fn malformed_input_yields_no_rows() {
        assert!(normalize_chain_history("not-a-chain", "[]").is_empty());
        assert!(normalize_chain_history("bitcoin", "not json").is_empty());
        assert!(normalize_chain_history("bitcoin", r#"{"txid":"a"}"#).is_empty());
        assert!(normalize_chain_history("bitcoin", r#"[{"no_txid":1}]"#).is_empty());
    }

    /// Stellar reports ISO-8601 where every other chain reports a number.
    #[test]
    fn iso8601_timestamps_parse() {
        let rows = normalize_chain_history(
            "stellar",
            r#"[{"txid":"e","timestamp":"2023-11-14T22:13:20Z","amount_stroops":1,"is_incoming":true}]"#,
        );
        assert_eq!(rows[0].timestamp, 1_700_000_000.0);
    }
}

/// A provider's timestamp is a string from the network, so the parser has to
/// treat it as one: it may be malformed, it may carry a zone, and it may not
/// be ASCII at all.
#[cfg(test)]
mod iso8601_tests {
    use super::parse_iso8601_timestamp;

    /// `2023-11-14T22:13:20Z` is Unix 1700000000, and every spelling of that
    /// instant has to reach the same number. The parser this replaces read
    /// the wall clock and dropped the offset, so the last two of these landed
    /// eight and five hours away from the first.
    #[test]
    fn one_instant_spelled_five_ways_is_one_number() {
        for spelling in [
            "2023-11-14T22:13:20Z",
            "2023-11-14T22:13:20",
            "2023-11-14t22:13:20z",
            "2023-11-14 22:13:20",
            "2023-11-14T22:13:20.123456Z",
            "2023-11-15T06:13:20+08:00",
            "2023-11-15T06:13:20+0800",
            "2023-11-15T06:13:20+08",
            "2023-11-14T17:13:20-05:00",
            "  2023-11-14T22:13:20Z  ",
        ] {
            assert_eq!(
                parse_iso8601_timestamp(spelling),
                Some(1_700_000_000.0),
                "{spelling}"
            );
        }
    }

    /// The regression: byte-indexing a `&str` at 4, 7, 10, 13 and 16 panics
    /// when one of those offsets falls inside a multi-byte character. These
    /// are all at least nineteen bytes, so the old length guard let every one
    /// of them through and core aborted on a provider's response.
    #[test]
    fn a_non_ascii_response_is_refused_and_does_not_panic() {
        for hostile in [
            "２０２３-11-14T22:13:20Z",
            "2023-11-14T22:13:20Z…………",
            "日本語日本語日本語日本語日本語日本語",
            "2023-11-14T22:13:2\u{0660}Z",
        ] {
            assert_eq!(parse_iso8601_timestamp(hostile), None, "{hostile}");
        }
    }

    /// Refusal, not the epoch. `0.0` is 1 January 1970 — a real date that a
    /// history row renders as one and sorts by.
    #[test]
    fn what_is_not_a_timestamp_is_none_rather_than_1970() {
        for malformed in [
            "",
            "2023-11-14",
            "not a timestamp at all",
            "2023/11/14T22:13:20Z",
            "2023-11-14X22:13:20Z",
            "2023-13-14T22:13:20Z", // month 13
            "2023-11-32T22:13:20Z", // day 32
            "2023-11-14T24:13:20Z", // hour 24
            "2023-11-14T22:60:20Z", // minute 60
            "2023-11-14T22:13:61Z", // second 61
            "20a3-11-14T22:13:20Z",
            "+023-11-14T22:13:20Z",
            "2023-11-14T22:13:20+24:00", // offset out of range
            "2023-11-14T22:13:20+8",     // offset too short to read
            "2023-11-14T22:13:20 UTC",
        ] {
            assert_eq!(parse_iso8601_timestamp(malformed), None, "{malformed}");
        }
    }

    /// A leap second is a real reading at :60, and the epoch itself is a real
    /// timestamp rather than the failure value it used to share.
    #[test]
    fn leap_seconds_and_the_epoch_itself_parse() {
        assert_eq!(
            parse_iso8601_timestamp("2016-12-31T23:59:60Z"),
            Some(1_483_228_800.0)
        );
        assert_eq!(
            parse_iso8601_timestamp("1970-01-01T00:00:00Z"),
            Some(0.0),
            "the epoch parses; it is no longer also the error value"
        );
    }
}

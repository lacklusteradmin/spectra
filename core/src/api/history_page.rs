//! Provider pagination is part of the wire contract, before transfer filtering.
use crate::api::error::ApiError;

#[derive(Debug, Clone)]
pub struct HistoryPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> HistoryPage<T> {
    /// The same page with each item converted, dropping the ones `convert`
    /// refuses; the cursor still follows the provider's page.
    pub fn filter_map<U>(self, convert: impl FnMut(T) -> Option<U>) -> HistoryPage<U> {
        HistoryPage {
            items: self.items.into_iter().filter_map(convert).collect(),
            next_cursor: self.next_cursor,
        }
    }
}

/// One transfer in an address's history, as every history adapter reports it.
///
/// Each adapter converts its own answer into this, so the units it reads,
/// the scale of its timestamps and where it says which way a transfer went
/// stay with the code that parsed them. The service gives the entry an asset
/// identity and stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub txid: String,
    /// The token's contract, mint or issue; `None` for the network's own coin.
    pub contract: Option<String>,
    pub amount: HistoryAmount,
    pub incoming: bool,
    pub outcome: HistoryOutcome,
    pub block_height: Option<i64>,
    /// Unix seconds; `None` while the chain has not dated the transaction.
    pub timestamp: Option<f64>,
    /// The other party, where the source names one; empty otherwise.
    pub counterparty: String,
}

/// How much moved, as a magnitude; [`HistoryEntry::incoming`] says which way.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryAmount {
    /// Whole base units of the network's own coin: satoshis, wei, drops.
    Units(u128),
    /// An exact decimal already at its asset's precision.
    Decimal(String),
}

impl HistoryAmount {
    /// A signed integer amount, as the UTXO family and a few account chains
    /// report an address's net change.
    pub(crate) fn signed_units(net: i64) -> Self {
        Self::Units(net.unsigned_abs().into())
    }

    /// A whole number of base units written as digits, as the sources whose
    /// amounts overflow a JSON number send them. `None` for anything else.
    pub(crate) fn unit_digits(digits: &str) -> Option<Self> {
        let digits = digits.strip_prefix('-').unwrap_or(digits);
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse().ok().map(Self::Units))
            .flatten()
    }
}

/// What the chain says became of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOutcome {
    Pending,
    Confirmed,
    Failed,
}

pub(crate) fn page_number(cursor: Option<&str>) -> Result<u32, ApiError> {
    cursor.map_or(Ok(1), |value| {
        value
            .parse::<u32>()
            .ok()
            .filter(|page| *page > 0 && *page < u32::MAX)
            .ok_or_else(|| ApiError::InvalidInput("invalid history page cursor".into()))
    })
}

pub(crate) fn query_value(value: &str) -> String {
    reqwest::Url::parse_with_params("https://cursor.invalid", &[("cursor", value)])
        .expect("static cursor URL")
        .query()
        .expect("one cursor parameter")
        .trim_start_matches("cursor=")
        .to_string()
}

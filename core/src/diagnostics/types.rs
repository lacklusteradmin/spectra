//! Diagnostic record types. The serde names below are the field names of the
//! exported diagnostics bundle.

use serde::{Deserialize, Serialize};

/// What one backend answered when a history run asked it.
///
/// Present only for chains that ask more than one — the EVM family asks
/// four, Tron asks TronScan for transactions and TRC-20 transfers
/// separately. A chain with a single backend leaves this empty and
/// `source_used` says which one it was.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryDiagnosticsSource {
    pub name: String,
    pub count: i32,
    pub error: Option<String>,
}

/// One wallet's history-run result on one chain.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryDiagnostics {
    #[serde(rename = "walletID")]
    pub wallet_id: String,
    /// The address or xpub the run looked at.
    pub identifier: String,
    /// Which backend the reported count came from.
    #[serde(rename = "sourceUsed")]
    pub source_used: String,
    /// Rows the run ended up with.
    #[serde(rename = "transactionCount")]
    pub transaction_count: i32,
    /// Rows seen before decoding, where the chain decodes and can see more
    /// than it can use. `None` where the scan and the count are the same
    /// thing, which is every chain but the EVM family.
    #[serde(rename = "scannedCount")]
    pub scanned_count: Option<i32>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
    pub error: Option<String>,
    /// One entry per backend asked, where the chain asks more than one.
    #[serde(rename = "perSource")]
    pub per_source: Vec<HistoryDiagnosticsSource>,
}

impl HistoryDiagnostics {
    /// Rows the scan saw but could not decode. Derived, because storing it
    /// alongside the two numbers it comes from is a third number that can
    /// disagree with them.
    pub fn undecoded_count(&self) -> i32 {
        self.scanned_count
            .map(|scanned| (scanned - self.transaction_count).max(0))
            .unwrap_or(0)
    }

    /// How much of what the scan saw it could decode, in `0.0..=1.0`. `1.0`
    /// when there was nothing to decode or the chain does not decode.
    pub fn decoding_completeness(&self) -> f64 {
        match self.scanned_count {
            Some(scanned) if scanned > 0 => {
                (f64::from(self.transaction_count) / f64::from(scanned)).clamp(0.0, 1.0)
            }
            _ => 1.0,
        }
    }
}

#[derive(uniffi::Record, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsEnvironmentMetadata {
    pub app_version: String,
    pub build_number: String,
    pub os_version: String,
    pub locale_identifier: String,
    pub time_zone_identifier: String,
    pub selected_fiat_currency: String,
    pub wallet_count: i64,
    pub transaction_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two numbers derived from the counts cannot disagree with them.
    #[test]
    fn undecoded_and_completeness_are_derived() {
        let row = |scanned: Option<i32>, decoded: i32| HistoryDiagnostics {
            wallet_id: "w".into(),
            identifier: "a".into(),
            source_used: "rust".into(),
            transaction_count: decoded,
            scanned_count: scanned,
            next_cursor: None,
            error: None,
            per_source: Vec::new(),
        };
        let partial = row(Some(10), 9);
        assert_eq!(partial.undecoded_count(), 1);
        assert!((partial.decoding_completeness() - 0.9).abs() < 1e-9);

        // Nothing scanned, or a chain that does not decode: complete by
        // definition, not zero percent.
        assert_eq!(row(None, 5).undecoded_count(), 0);
        assert_eq!(row(None, 5).decoding_completeness(), 1.0);
        assert_eq!(row(Some(0), 0).decoding_completeness(), 1.0);

        // A decode that somehow saw fewer than it produced is clamped rather
        // than reported as a negative count or a ratio above one.
        assert_eq!(row(Some(2), 5).undecoded_count(), 0);
        assert_eq!(row(Some(2), 5).decoding_completeness(), 1.0);
    }
}

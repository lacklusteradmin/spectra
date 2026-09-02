//! Rust-owned diagnostic record types exposed to Swift via UniFFI.
//!
//! The JSON serialization shape (serde rename attributes below) must stay
//! byte-identical to the Swift dictionary layouts — changing field names here
//! breaks the exported diagnostics-bundle format.

use serde::{Deserialize, Serialize};

macro_rules! diagnostics_record {
    (
        $(#[$meta:meta])*
        $name:ident { $( $(#[$fmeta:meta])* $field:ident : $ty:ty ),* $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(uniffi::Record, Serialize, Deserialize, Clone, Debug, PartialEq)]
        pub struct $name {
            $( $(#[$fmeta])* pub $field : $ty, )*
        }
    };
}

diagnostics_record! {
    /// What one backend answered when a history run asked it.
    ///
    /// Present only for chains that ask more than one — the EVM family asks
    /// four, Tron asks TronScan for transactions and TRC-20 transfers
    /// separately. A chain with a single backend leaves this empty and
    /// `source_used` says which one it was.
    HistoryDiagnosticsSource {
        name: String,
        count: i32,
        error: Option<String>,
    }
}

diagnostics_record! {
    /// One wallet's history-run result on one chain.
    ///
    /// There were five of these — `UtxoHistoryDiagnostics`,
    /// `SimpleHistoryDiagnostics`, `SolanaHistoryDiagnostics`,
    /// `TronHistoryDiagnostics` and
    /// `EthereumTokenTransferHistoryDiagnostics`. Three were the same four
    /// fields: Solana's was Simple with `transaction_count` spelled
    /// `rpc_count`, and Tron's was Simple plus a second count. UTXO's added a
    /// wallet id and a cursor. Only the EVM one had a different shape, and its
    /// difference was four `(count, error)` pairs flattened into eight fields
    /// plus two numbers derivable from the other two.
    ///
    /// The split cost five registries, five JSON builders, five export wrapper
    /// types, a `DiagnosticsShape` enum threaded through all of them, and a
    /// Swift call site that had to know which shape its chain used.
    HistoryDiagnostics {
        #[serde(rename = "walletID")]
        wallet_id: String,
        /// The address or xpub the run looked at.
        identifier: String,
        /// Which backend the reported count came from.
        #[serde(rename = "sourceUsed")]
        source_used: String,
        /// Rows the run ended up with.
        #[serde(rename = "transactionCount")]
        transaction_count: i32,
        /// Rows seen before decoding, where the chain decodes and can see more
        /// than it can use. `None` where the scan and the count are the same
        /// thing, which is every chain but the EVM family.
        #[serde(rename = "scannedCount")]
        scanned_count: Option<i32>,
        #[serde(rename = "nextCursor")]
        next_cursor: Option<String>,
        error: Option<String>,
        /// One entry per backend asked, where the chain asks more than one.
        #[serde(rename = "perSource")]
        per_source: Vec<HistoryDiagnosticsSource>,
    }
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

    fn roundtrip<T>(json: &str)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let decoded: T = serde_json::from_str(json).expect("decode");
        let reencoded = serde_json::to_string(&decoded).expect("encode");
        let redecoded: T = serde_json::from_str(&reencoded).expect("redecode");
        assert_eq!(decoded, redecoded, "roundtrip mismatch");
        // Re-encoding the re-decoded must match the first re-encoding byte-for-byte.
        let reencoded2 = serde_json::to_string(&redecoded).expect("encode2");
        assert_eq!(reencoded, reencoded2);
    }

    /// One record, one round-trip. There were five, and the ten `Simple*`
    /// aliases before them.
    #[test]
    fn history_roundtrip() {
        roundtrip::<HistoryDiagnostics>(
            r#"{"walletID":"w1","identifier":"addr","sourceUsed":"rust","transactionCount":5,"scannedCount":null,"nextCursor":"c","error":null,"perSource":[]}"#,
        );
    }

    /// A chain that asks several backends carries one entry per backend rather
    /// than a field per backend.
    #[test]
    fn history_with_sources_roundtrip() {
        roundtrip::<HistoryDiagnostics>(
            r#"{"walletID":"w1","identifier":"0xabc","sourceUsed":"rust","transactionCount":9,"scannedCount":10,"nextCursor":null,"error":"boom","perSource":[{"name":"rpc","count":1,"error":null},{"name":"blockscout","count":2,"error":"boom"}]}"#,
        );
    }

    /// The two numbers the old EVM record stored beside the ones they came
    /// from are derived, so they cannot disagree with them.
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

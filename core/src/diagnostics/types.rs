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
    UtxoHistoryDiagnostics {
        #[serde(rename = "walletID")]
        wallet_id: String,
        identifier: String,
        #[serde(rename = "sourceUsed")]
        source_used: String,
        #[serde(rename = "transactionCount")]
        transaction_count: i32,
        #[serde(rename = "nextCursor")]
        next_cursor: Option<String>,
        error: Option<String>,
    }
}

diagnostics_record! {
    EthereumTokenTransferHistoryDiagnostics {
        address: String,
        #[serde(rename = "rpcTransferCount")]
        rpc_transfer_count: i32,
        #[serde(rename = "rpcError")]
        rpc_error: Option<String>,
        #[serde(rename = "blockscoutTransferCount")]
        blockscout_transfer_count: i32,
        #[serde(rename = "blockscoutError")]
        blockscout_error: Option<String>,
        #[serde(rename = "etherscanTransferCount")]
        etherscan_transfer_count: i32,
        #[serde(rename = "etherscanError")]
        etherscan_error: Option<String>,
        #[serde(rename = "ethplorerTransferCount")]
        ethplorer_transfer_count: i32,
        #[serde(rename = "ethplorerError")]
        ethplorer_error: Option<String>,
        #[serde(rename = "sourceUsed")]
        source_used: String,
        #[serde(rename = "transferScanCount")]
        transfer_scan_count: i32,
        #[serde(rename = "decodedTransferCount")]
        decoded_transfer_count: i32,
        #[serde(rename = "unsupportedTransferDropCount")]
        unsupported_transfer_drop_count: i32,
        #[serde(rename = "decodingCompletenessRatio")]
        decoding_completeness_ratio: f64,
    }
}

diagnostics_record! {
    TronHistoryDiagnostics {
        address: String,
        #[serde(rename = "tronScanTxCount")]
        tron_scan_tx_count: i32,
        #[serde(rename = "tronScanTRC20Count")]
        tron_scan_trc20_count: i32,
        #[serde(rename = "sourceUsed")]
        source_used: String,
        error: Option<String>,
    }
}

diagnostics_record! {
    SolanaHistoryDiagnostics {
        address: String,
        #[serde(rename = "rpcCount")]
        rpc_count: i32,
        #[serde(rename = "sourceUsed")]
        source_used: String,
        error: Option<String>,
    }
}

// Simple address/source/count/error shape shared by ten chains.
diagnostics_record! {
    /// History diagnostics for a chain that reports one address, one source and
    /// a count.
    ///
    /// There were ten of these — `SimpleHistoryDiagnostics`,
    /// `SimpleHistoryDiagnostics`, and eight more — stamped out by a macro
    /// from one field list. Ten names for one record: the macro was core
    /// already admitting they were identical, and the cost landed on the
    /// callers, which needed ten differently-typed slots, ten dictionaries and
    /// a Swift protocol whose only job was to treat them as one type again.
    SimpleHistoryDiagnostics {
        address: String,
        #[serde(rename = "sourceUsed")]
        source_used: String,
        #[serde(rename = "transactionCount")]
        transaction_count: i32,
        error: Option<String>,
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
    pub pricing_provider: String,
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

    #[test]
    fn bitcoin_roundtrip() {
        roundtrip::<UtxoHistoryDiagnostics>(
            r#"{"walletID":"w1","identifier":"addr","sourceUsed":"rust","transactionCount":5,"nextCursor":"c","error":null}"#,
        );
    }

    #[test]
    fn ethereum_roundtrip() {
        roundtrip::<EthereumTokenTransferHistoryDiagnostics>(
            r#"{"address":"0xabc","rpcTransferCount":1,"rpcError":null,"blockscoutTransferCount":2,"blockscoutError":"boom","etherscanTransferCount":3,"etherscanError":null,"ethplorerTransferCount":4,"ethplorerError":null,"sourceUsed":"rust","transferScanCount":10,"decodedTransferCount":9,"unsupportedTransferDropCount":1,"decodingCompletenessRatio":0.9}"#,
        );
    }

    #[test]
    fn tron_roundtrip() {
        roundtrip::<TronHistoryDiagnostics>(
            r#"{"address":"T","tronScanTxCount":4,"tronScanTRC20Count":2,"sourceUsed":"tronscan","error":null}"#,
        );
    }

    #[test]
    fn solana_roundtrip() {
        roundtrip::<SolanaHistoryDiagnostics>(
            r#"{"address":"S","rpcCount":7,"sourceUsed":"rpc","error":null}"#,
        );
    }

    macro_rules! simple_test {
        ($fn_name:ident, $ty:ident) => {
            #[test]
            fn $fn_name() {
                roundtrip::<$ty>(
                    r#"{"address":"x","sourceUsed":"rust","transactionCount":3,"error":null}"#,
                );
            }
        };
    }
    // Ten identical round-trips became one when the ten records did.
    simple_test!(simple_history_roundtrip, SimpleHistoryDiagnostics);
}

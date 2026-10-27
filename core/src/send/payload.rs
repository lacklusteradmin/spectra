//! Exact fee units and durable signed-submission journaling.

/// Interpret the shortest decimal representation of a UI fee exactly. Floats
/// remain at the existing boundary; no rounding or saturating cast crosses it.
pub fn fee_units(amount: &str, decimals: u32) -> Result<u64, crate::error::SpectraBridgeError> {
    let invalid = || crate::error::SpectraBridgeError::InvalidInput {
        message: "fee must be a positive decimal that fits its precision and integer range".into(),
    };
    crate::amount::decimal::to_units(amount, decimals)
        .filter(|&units| units > 0)
        .and_then(|units| u64::try_from(units).ok())
        .ok_or_else(invalid)
}

#[cfg(test)]
mod fee_tests {
    use super::*;
    #[test]
    fn fees_never_round_or_saturate() {
        assert_eq!(fee_units("0.17", 6).unwrap(), 170000);
        assert_eq!(fee_units("0.01", 9).unwrap(), 10000000);
        assert_eq!(fee_units("0.000000001", 9).unwrap(), 1);
        for value in [
            "",
            "NaN",
            "-1",
            "0",
            "0.0000000001",
            "18446744073.709551616",
        ] {
            assert!(fee_units(value, 9).is_err(), "{value}");
        }
        assert!(fee_units("18446744073710", 6).is_err());
    }
}

/// A signed transaction ready to submit, with the hash and nonce it will
/// land under, as its send record keeps it for a rebroadcast. Never include
/// the wallet seed or wallet signing keys.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedSubmission {
    pub payload: SignedPayload,
    pub transaction_hash: Option<String>,
    pub nonce: Option<u64>,
}

/// A signed transaction in the form its network's nodes take it. The
/// endpoint's API names the form; a broadcast refuses any other.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SignedPayload {
    /// The transaction's bytes in hex, `0x`-prefixed where the network
    /// writes them so (EVM, Substrate). An Aptos transaction in this form
    /// is its BCS.
    Hex(String),
    /// The transaction's bytes in standard base64.
    Base64(String),
    /// A document the node takes whole: a Tron transaction, an Aptos signed
    /// body, an ICP or Kaspa submission.
    Json(String),
    /// A Sui transaction's bytes and its signature, both base64.
    Sui {
        transaction: String,
        signature: String,
    },
    /// ICP staking's signed ingress calls, submitted in turn.
    IcpStaking(Vec<super::icp_staking::SignedIcpStakingCall>),
}

impl SignedPayload {
    /// The raw transaction a hex payload carries.
    pub(crate) fn raw_hex(&self) -> Result<&str, super::error::SendError> {
        match self {
            Self::Hex(raw) => Ok(raw),
            _ => Err(super::error::SendError::invalid(
                "Signed transaction is not raw bytes",
            )),
        }
    }
}

/// What a person is shown, and what a sign-only send hands out: the signed
/// transaction as its network's tools take it.
impl std::fmt::Display for SignedPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hex(text) | Self::Base64(text) | Self::Json(text) => f.write_str(text),
            // As `sui_executeTransactionBlock` names its two parts.
            Self::Sui {
                transaction,
                signature,
            } => write!(
                f,
                "{}",
                serde_json::json!({ "tx_bytes": transaction, "signatures": [signature] })
            ),
            Self::IcpStaking(calls) => {
                f.write_str(&serde_json::to_string(calls).map_err(|_| std::fmt::Error)?)
            }
        }
    }
}

pub(crate) fn bitcoin_transaction_id(raw: &str) -> Option<String> {
    let bytes = hex::decode(raw).ok()?;
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&bytes).ok()?;
    Some(tx.compute_txid().to_string())
}

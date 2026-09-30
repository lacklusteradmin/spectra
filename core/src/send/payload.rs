//! Exact fee units and durable signed-submission journaling.

/// Interpret the shortest decimal representation of a UI fee exactly. Floats
/// remain at the existing boundary; no rounding or saturating cast crosses it.
pub fn fee_units(amount: &str, decimals: u32) -> Result<u64, crate::SpectraBridgeError> {
    let invalid = || crate::SpectraBridgeError::InvalidInput {
        message: "fee must be a positive decimal that fits its precision and integer range".into(),
    };
    crate::decimal::to_units(amount, decimals)
        .filter(|&units| units > 0)
        .and_then(|units| u64::try_from(units).ok())
        .ok_or_else(invalid)
}

pub(crate) fn dogecoin_fee(rate: &str) -> Result<u64, crate::SpectraBridgeError> {
    let raw_per_kb = fee_units(rate, 8)?;
    // The existing 350-byte estimate, rounded UP once in integer units.
    Ok((u128::from(raw_per_kb) * 350).div_ceil(1000) as u64)
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
        assert_eq!(dogecoin_fee("0.01").unwrap(), 350000);
        assert_eq!(dogecoin_fee("0.00000001").unwrap(), 1);
    }
}

/// Exact input for submitting the same signed transaction again. Never include
/// the wallet seed or wallet signing keys.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedSubmission {
    pub payload: String,
    pub result_field: String,
    pub transaction_hash: Option<String>,
    pub nonce: Option<u64>,
}

pub(crate) fn bitcoin_transaction_id(raw: &str) -> Option<String> {
    let bytes = hex::decode(raw).ok()?;
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&bytes).ok()?;
    Some(tx.compute_txid().to_string())
}

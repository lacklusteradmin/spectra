// Per-chain send-payload JSON builders.
//
// Each function takes a human-scale decimal amount plus addresses/keys, performs
// the raw-unit conversion, and returns the exact JSON body the Rust signer
// expects. Consolidates ~17 scattered `UInt64(amount * 1eN)` + sendPayload()
// call sites from Swift into one place.
//
// Shared broadcast-result classification also lives here .

/// Interpret the shortest decimal representation of a UI fee exactly. Floats
/// remain at the existing boundary; no rounding or saturating cast crosses it.
pub fn fee_units(amount: f64, decimals: u32) -> Result<u64, crate::SpectraBridgeError> {
    let invalid = || crate::SpectraBridgeError::InvalidInput {
        message: "fee must be positive, finite and fit its precision and integer range".into(),
    };
    if !amount.is_finite() || amount <= 0.0 {
        return Err(invalid());
    }
    let raw = crate::send::amount_input::parse_raw_amount(&amount.to_string(), decimals)
        .map_err(|_| invalid())?;
    u64::try_from(raw).map_err(|_| invalid())
}

pub(crate) fn dogecoin_fee(rate: f64) -> Result<u64, crate::SpectraBridgeError> {
    let raw_per_kb = fee_units(rate, 8)?;
    // The existing 350-byte estimate, rounded UP once in integer units.
    Ok((u128::from(raw_per_kb) * 350).div_ceil(1000) as u64)
}

// --- Broadcast-result classification ---

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum SendChain {
    Bitcoin,
    BitcoinCash,
    BitcoinSV,
    Litecoin,
    Dogecoin,
    Ethereum,
    Tron,
    Solana,
    Xrp,
    Stellar,
    Monero,
    Cardano,
    Sui,
    Aptos,
    Ton,
    Icp,
    Near,
    Polkadot,
    Zcash,
    BitcoinGold,
    Decred,
    Kaspa,
    Dash,
    Bittensor,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct SendBroadcastOutcome {
    pub transaction_hash: String,
    pub payload_format: String,
}

fn hash_field_for(chain: SendChain) -> &'static str {
    match chain {
        SendChain::Sui => "digest",
        SendChain::Solana => "signature",
        _ => "txid",
    }
}

fn format_key_for(chain: SendChain) -> &'static str {
    match chain {
        SendChain::Bitcoin => "bitcoin.rust_json",
        SendChain::BitcoinCash => "bitcoin_cash.rust_json",
        SendChain::BitcoinSV => "bitcoin_sv.rust_json",
        SendChain::Litecoin => "litecoin.rust_json",
        SendChain::Dogecoin => "dogecoin.rust_json",
        SendChain::Ethereum => "ethereum.rust_json",
        SendChain::Tron => "tron.rust_json",
        SendChain::Solana => "solana.rust_json",
        SendChain::Xrp => "xrp.rust_json",
        SendChain::Stellar => "stellar.rust_json",
        SendChain::Monero => "monero.rust_json",
        SendChain::Cardano => "cardano.rust_json",
        SendChain::Sui => "sui.rust_json",
        SendChain::Aptos => "aptos.rust_json",
        SendChain::Ton => "ton.rust_json",
        SendChain::Icp => "icp.rust_json",
        SendChain::Near => "near.rust_json",
        SendChain::Polkadot => "polkadot.rust_json",
        SendChain::Zcash => "zcash.rust_json",
        SendChain::BitcoinGold => "bitcoin_gold.rust_json",
        SendChain::Decred => "decred.rust_json",
        SendChain::Kaspa => "kaspa.rust_json",
        SendChain::Dash => "dash.rust_json",
        SendChain::Bittensor => "bittensor.rust_json",
    }
}

pub fn classify_send_broadcast_result(
    chain: SendChain,
    result_json: String,
) -> SendBroadcastOutcome {
    let field = hash_field_for(chain);
    let hash = crate::send::preview_decode::extract_json_string_field(
        result_json.clone(),
        field.to_string(),
    );
    SendBroadcastOutcome {
        transaction_hash: hash,
        payload_format: format_key_for(chain).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_sui_digest() {
        let o = classify_send_broadcast_result(SendChain::Sui, r#"{"digest":"abc"}"#.into());
        assert_eq!(o.transaction_hash, "abc");
        assert_eq!(o.payload_format, "sui.rust_json");
    }

    #[test]
    fn classify_icp_transaction_hash() {
        let o = classify_send_broadcast_result(SendChain::Icp, r#"{"txid":"abc"}"#.into());
        assert_eq!(o.transaction_hash, "abc");
    }
}

#[cfg(test)]
mod fee_tests {
    use super::*;
    #[test]
    fn fees_never_round_or_saturate() {
        assert_eq!(fee_units(0.17, 6).unwrap(), 170000);
        assert_eq!(fee_units(0.01, 9).unwrap(), 10000000);
        assert_eq!(fee_units(0.000000001, 9).unwrap(), 1);
        for value in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            0.0000000001,
            u64::MAX as f64,
        ] {
            assert!(fee_units(value, 9).is_err(), "{value}");
        }
        assert!(fee_units(18446744073710.0, 6).is_err());
        assert_eq!(dogecoin_fee(0.01).unwrap(), 350000);
        assert_eq!(dogecoin_fee(0.00000001).unwrap(), 1);
    }
}

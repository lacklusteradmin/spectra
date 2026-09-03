// Per-chain send-payload JSON builders.
//
// Each function takes a human-scale decimal amount plus addresses/keys, performs
// the raw-unit conversion, and returns the exact JSON body the Rust signer
// expects. Consolidates ~17 scattered `UInt64(amount * 1eN)` + sendPayload()
// call sites from Swift into one place.
//
// Shared broadcast-result classification also lives here .

pub(crate) fn amount_u64(amount: f64, scale: f64) -> u64 {
    // Round to nearest to avoid sub-unit rounding errors from floating-point drift.
    (amount * scale).round() as u64
}

pub(crate) fn amount_i64(amount: f64, scale: f64) -> i64 {
    (amount * scale).round() as i64
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
        SendChain::Icp => "block_index",
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
    let mut hash = crate::send::preview_decode::extract_json_string_field(
        result_json.clone(),
        field.to_string(),
    );
    // ICP: fallback to raw JSON when block_index is absent (matches Swift behavior).
    if matches!(chain, SendChain::Icp) && hash.is_empty() {
        hash = result_json.clone();
    }
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
    fn classify_icp_fallback_when_no_block_index() {
        let o = classify_send_broadcast_result(SendChain::Icp, r#"{"other":1}"#.into());
        assert_eq!(o.transaction_hash, r#"{"other":1}"#);
    }
}

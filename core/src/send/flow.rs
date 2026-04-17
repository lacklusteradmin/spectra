// Phase 4a pure-logic lifts from swift/store/WalletStore+SendFlow.swift.
//
// Each function is a pure transform with no @Published or iOS dependencies.
// Swift call sites collapse to one-liners delegating here.

use crate::addressing::{validate_address, AddressValidationRequest};
use crate::wallet_core::*;

#[uniffi::export]
pub fn compute_background_maintenance_interval(
    base_interval_sec: f64,
    is_constrained_network: bool,
    is_expensive_network: bool,
    is_low_power_mode: bool,
    battery_level: f32,
) -> f64 {
    let mut interval = base_interval_sec;
    if is_constrained_network || is_expensive_network {
        interval = interval.max(30.0 * 60.0);
    }
    if is_low_power_mode {
        interval = interval.max(45.0 * 60.0);
    }
    if battery_level < 0.20 {
        interval = interval.max(60.0 * 60.0);
    }
    interval
}

#[uniffi::export]
pub fn evaluate_heavy_refresh_gate(
    background_sync_profile: String,
    is_network_reachable: bool,
    is_constrained_network: bool,
    is_expensive_network: bool,
    is_low_power_mode: bool,
    battery_level: f32,
) -> bool {
    if !is_network_reachable { return false; }
    match background_sync_profile.as_str() {
        "conservative" => !is_constrained_network && !is_expensive_network && !is_low_power_mode && battery_level >= 0.30,
        "balanced" => !is_constrained_network && !is_low_power_mode && battery_level >= 0.20,
        _ => {
            if is_low_power_mode && battery_level < 0.15 { return false; }
            battery_level >= 0.15
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmReceiptClassification {
    pub is_confirmed: bool,
    pub is_failed: bool,
    pub block_number: Option<i64>,
}

#[uniffi::export]
pub fn classify_evm_receipt_json(json: String) -> Option<EvmReceiptClassification> {
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let block_number = v.get("block_number").and_then(|x| x.as_i64());
    let status = v.get("status").and_then(|x| x.as_str());
    let is_confirmed = block_number.is_some();
    let is_failed = matches!(status, Some("0x0"));
    Some(EvmReceiptClassification { is_confirmed, is_failed, block_number })
}

pub(crate) fn chain_kind(chain_name: &str) -> Option<&'static str> {
    Some(match chain_name {
        "Bitcoin" => "bitcoin",
        "Bitcoin Cash" => "bitcoinCash",
        "Bitcoin SV" => "bitcoinSV",
        "Litecoin" => "litecoin",
        "Dogecoin" => "dogecoin",
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche" | "Hyperliquid" => "evm",
        "Tron" => "tron",
        "Solana" => "solana",
        "Cardano" => "cardano",
        "XRP Ledger" => "xrp",
        "Stellar" => "stellar",
        "Monero" => "monero",
        "Sui" => "sui",
        "Aptos" => "aptos",
        "TON" => "ton",
        "Internet Computer" => "internetComputer",
        "NEAR" => "near",
        "Polkadot" => "polkadot",
        _ => return None,
    })
}

#[uniffi::export]
pub fn is_valid_send_address(
    chain_name: String,
    address: String,
    network_mode: Option<String>,
) -> bool {
    let Some(kind) = chain_kind(&chain_name) else { return false };
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: address,
        network_mode,
    })
    .is_valid
}

pub(crate) fn normalize_address(chain_name: &str, address: &str) -> String {
    let t = address.trim();
    match chain_name {
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism"
        | "BNB Chain" | "Avalanche" | "Hyperliquid" => t.to_lowercase(),
        "Sui" | "Aptos" => {
            let l = t.to_lowercase();
            if l.starts_with("0x") { l } else { format!("0x{}", l) }
        }
        "Internet Computer" | "NEAR" => t.to_lowercase(),
        _ => t.to_string(),
    }
}

#[uniffi::export]
pub fn normalized_send_address(chain_name: String, address: String) -> String {
    normalize_address(&chain_name, &address)
}

/// Heuristic: does the trimmed input look like an ENS name (`foo.eth`, no spaces,
/// not an 0x-prefixed hex address)? Mirrors Swift's `isENSNameCandidate`.
#[uniffi::export]
pub fn is_ens_name_candidate(value: String) -> bool {
    let normalized = value.trim().to_lowercase();
    normalized.ends_with(".eth")
        && !normalized.contains(' ')
        && !normalized.starts_with("0x")
}

/// Snapshot of every chain's current send-preview. Swift passes the full set;
/// Rust picks the one matching `chain_name` and flattens it to `SendPreviewDetails`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SendPreviewsInput {
    pub bitcoin: Option<BitcoinSendPreview>,
    pub bitcoin_cash: Option<BitcoinSendPreview>,
    pub bitcoin_sv: Option<BitcoinSendPreview>,
    pub litecoin: Option<BitcoinSendPreview>,
    pub dogecoin: Option<DogecoinSendPreview>,
    pub ethereum: Option<EthereumSendPreview>,
    pub tron: Option<TronSendPreview>,
    pub solana: Option<SolanaSendPreview>,
    pub xrp: Option<XRPSendPreview>,
    pub stellar: Option<StellarSendPreview>,
    pub monero: Option<MoneroSendPreview>,
    pub cardano: Option<CardanoSendPreview>,
    pub sui: Option<SuiSendPreview>,
    pub aptos: Option<AptosSendPreview>,
    pub ton: Option<TONSendPreview>,
    pub icp: Option<ICPSendPreview>,
    pub near: Option<NearSendPreview>,
    pub polkadot: Option<PolkadotSendPreview>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, uniffi::Record)]
pub struct SendPreviewDetailsCore {
    pub spendableBalance: Option<f64>,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: Option<f64>,
}

// Per-chain extracted tuple: (spendable, feeRateDesc, txBytes, inputCount, usesChange, maxSendable, estFee)
// `estFee` (the 7th slot) is only non-None for UTXO chains with a distinct network-fee
// field; it drives the `fallback = coin_amount - est_fee` calc Swift does at the tail.
type Extracted = (
    Option<f64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<bool>,
    Option<f64>,
    Option<f64>,
);

fn extract(input: &SendPreviewsInput, chain: &str) -> Option<Extracted> {
    match chain {
        "Bitcoin" => input.bitcoin.as_ref().map(|p| (
            p.spendableBalance, p.feeRateDescription.clone(), p.estimatedTransactionBytes,
            p.selectedInputCount, p.usesChangeOutput, p.maxSendable, Some(p.estimatedNetworkFeeBtc),
        )),
        "Bitcoin Cash" => input.bitcoin_cash.as_ref().map(|p| (
            p.spendableBalance, p.feeRateDescription.clone(), p.estimatedTransactionBytes,
            p.selectedInputCount, p.usesChangeOutput, p.maxSendable, Some(p.estimatedNetworkFeeBtc),
        )),
        "Bitcoin SV" => input.bitcoin_sv.as_ref().map(|p| (
            p.spendableBalance, p.feeRateDescription.clone(), p.estimatedTransactionBytes,
            p.selectedInputCount, p.usesChangeOutput, p.maxSendable, Some(p.estimatedNetworkFeeBtc),
        )),
        "Litecoin" => input.litecoin.as_ref().map(|p| (
            p.spendableBalance, p.feeRateDescription.clone(), p.estimatedTransactionBytes,
            p.selectedInputCount, p.usesChangeOutput, p.maxSendable, Some(p.estimatedNetworkFeeBtc),
        )),
        "Dogecoin" => input.dogecoin.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), Some(p.estimatedTransactionBytes),
            Some(p.selectedInputCount), Some(p.usesChangeOutput), Some(p.maxSendable), None,
        )),
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche" | "Hyperliquid" => {
            input.ethereum.as_ref().map(|p| (
                p.spendableBalance, p.feeRateDescription.clone(), None, None, None, p.maxSendable, None,
            ))
        }
        "Tron" => input.tron.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Solana" => input.solana.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "XRP Ledger" => input.xrp.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Stellar" => input.stellar.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Monero" => input.monero.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Cardano" => input.cardano.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Sui" => input.sui.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Aptos" => input.aptos.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "TON" => input.ton.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Internet Computer" => input.icp.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "NEAR" => input.near.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), None, None, None, Some(p.maxSendable), None,
        )),
        "Polkadot" => input.polkadot.as_ref().map(|p| (
            Some(p.spendableBalance), p.feeRateDescription.clone(), p.estimatedTransactionBytes,
            None, None, Some(p.maxSendable), None,
        )),
        _ => None,
    }
}

#[uniffi::export]
pub fn compute_send_preview_details(
    input: SendPreviewsInput,
    chain_name: String,
    coin_amount: f64,
) -> Option<SendPreviewDetailsCore> {
    let d = extract(&input, &chain_name)?;
    let fallback = d.6.map(|fee| (coin_amount - fee).max(0.0));
    Some(SendPreviewDetailsCore {
        spendableBalance: d.0.or(fallback),
        feeRateDescription: d.1,
        estimatedTransactionBytes: d.2,
        selectedInputCount: d.3,
        usesChangeOutput: d.4,
        maxSendable: d.5.or(fallback),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ens_candidate_positive() {
        assert!(is_ens_name_candidate("vitalik.eth".into()));
        assert!(is_ens_name_candidate("  Foo.ETH  ".into()));
    }

    #[test]
    fn ens_candidate_negative() {
        assert!(!is_ens_name_candidate("0xabc.eth".into()));
        assert!(!is_ens_name_candidate("foo .eth".into()));
        assert!(!is_ens_name_candidate("foo.com".into()));
    }
}

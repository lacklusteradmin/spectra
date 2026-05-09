// Pure-logic helpers backing the send flow.
//
// Each function is a pure transform with no @Published or iOS dependencies.
// Swift call sites collapse to one-liners delegating here.

use crate::derivation::addressing::{validate_address, AddressValidationRequest};
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
pub fn active_pending_refresh_interval_for_profile(
    background_sync_profile: String,
    balanced_interval: f64,
) -> f64 {
    match background_sync_profile.as_str() {
        "conservative" => 30.0,
        "aggressive" => 10.0,
        _ => balanced_interval,
    }
}

#[uniffi::export]
pub fn portfolio_composition_signature(holding_keys: Vec<String>) -> String {
    let mut sorted = holding_keys;
    sorted.sort();
    sorted.join("|")
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
        // Mainnets
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
        // Testnets — each maps to the testnet-flavored validator kind.
        "Bitcoin Testnet" => "bitcoinTestnet",
        "Bitcoin Testnet4" => "bitcoinTestnet4",
        "Bitcoin Signet" => "bitcoinSignet",
        "Bitcoin Cash Testnet" => "bitcoinCashTestnet",
        "Bitcoin SV Testnet" => "bitcoinSVTestnet",
        "Litecoin Testnet" => "litecoinTestnet",
        "Dogecoin Testnet" => "dogecoinTestnet",
        "Ethereum Sepolia"
        | "Ethereum Hoodi"
        | "Arbitrum Sepolia"
        | "Optimism Sepolia"
        | "Base Sepolia"
        | "BNB Chain Testnet"
        | "Avalanche Fuji"
        | "Polygon Amoy"
        | "Hyperliquid Testnet"
        | "Ethereum Classic Mordor" => "evmTestnet",
        "Tron Nile" => "tronTestnet",
        "Solana Devnet" => "solanaDevnet",
        "Cardano Preprod" => "cardanoTestnet",
        "XRP Ledger Testnet" => "xrpTestnet",
        "Stellar Testnet" => "stellarTestnet",
        "Monero Stagenet" => "moneroStagenet",
        "Sui Testnet" => "suiTestnet",
        "Aptos Testnet" => "aptosTestnet",
        "TON Testnet" => "tonTestnet",
        "NEAR Testnet" => "nearTestnet",
        "Polkadot Westend" => "polkadotTestnet",
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
        // EVM mainnets + testnets — same lowercase normalization.
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism"
        | "BNB Chain" | "Avalanche" | "Hyperliquid"
        | "Ethereum Sepolia" | "Ethereum Hoodi" | "Arbitrum Sepolia"
        | "Optimism Sepolia" | "Base Sepolia" | "BNB Chain Testnet"
        | "Avalanche Fuji" | "Polygon Amoy" | "Hyperliquid Testnet"
        | "Ethereum Classic Mordor" => t.to_lowercase(),
        "Sui" | "Aptos" | "Sui Testnet" | "Aptos Testnet" => {
            let l = t.to_lowercase();
            if l.starts_with("0x") { l } else { format!("0x{}", l) }
        }
        "Internet Computer" | "NEAR" | "NEAR Testnet" => t.to_lowercase(),
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

// ── FFI: high-risk send evaluation (relocated from ffi.rs) ───────────────

/// A chain_name + address pair used in the high-risk send evaluation.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HighRiskChainAddress {
    pub chain_name: String,
    pub address: String,
}

/// Typed input for high-risk send evaluation — replaces the JSON dict that
/// Swift previously assembled via `JSONSerialization`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HighRiskSendRequest {
    pub chain_name: String,
    pub symbol: String,
    pub amount: f64,
    pub holding_amount: f64,
    pub destination_address: String,
    pub destination_input: String,
    pub used_ens_resolution: bool,
    pub wallet_selected_chain: String,
    pub address_book_entries: Vec<HighRiskChainAddress>,
    pub tx_addresses: Vec<HighRiskChainAddress>,
}

/// A single high-risk warning with a code and optional metadata fields.
/// Swift maps these to localized user-facing strings.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HighRiskSendWarning {
    pub code: String,
    pub chain: Option<String>,
    pub name: Option<String>,
    pub address: Option<String>,
    pub percent: Option<u64>,
    pub symbol: Option<String>,
}

/// Typed high-risk send evaluation — replaces `core_evaluate_high_risk_send_reasons_json`.
#[uniffi::export]
pub fn core_evaluate_high_risk_send_reasons(
    request: HighRiskSendRequest,
) -> Vec<HighRiskSendWarning> {
    use crate::derivation::validation::{validate_address, AddressValidationRequest};

    let chain_name = &request.chain_name;
    let mut warnings: Vec<HighRiskSendWarning> = Vec::new();

    let make = |code: &str| HighRiskSendWarning {
        code: code.to_string(),
        chain: None,
        name: None,
        address: None,
        percent: None,
        symbol: None,
    };

    let hrsr_validate = |chain_name: &str, address: &str| -> bool {
        let Some(kind) = chain_kind(chain_name) else { return false };
        validate_address(AddressValidationRequest {
            kind: kind.to_string(),
            value: address.to_string(),
            network_mode: None,
        })
        .is_valid
    };

    // 1. Address format validation.
    if !hrsr_validate(chain_name, &request.destination_address) {
        warnings.push(HighRiskSendWarning {
            chain: Some(chain_name.clone()),
            ..make("invalid_format")
        });
    }

    // Normalize destination for case-insensitive comparison.
    let norm_dest =
        normalize_address(chain_name, &request.destination_address).to_lowercase();

    // 2. New address detection.
    let has_address_book = request.address_book_entries.iter().any(|e| {
        e.chain_name == *chain_name
            && normalize_address(chain_name, &e.address).to_lowercase() == norm_dest
    });
    let has_tx_history = request.tx_addresses.iter().any(|e| {
        e.chain_name == *chain_name
            && normalize_address(chain_name, &e.address).to_lowercase() == norm_dest
    });
    if !has_address_book && !has_tx_history {
        warnings.push(make("new_address"));
    }

    // 3. ENS resolution warning.
    if request.used_ens_resolution {
        warnings.push(HighRiskSendWarning {
            name: Some(request.destination_input.clone()),
            address: Some(request.destination_address.clone()),
            ..make("ens_resolved")
        });
    }

    // 4. Large send percentage (≥25 % of holding balance).
    if request.holding_amount > 0.0 {
        let ratio = request.amount / request.holding_amount;
        if ratio >= 0.25 {
            let pct = (ratio * 100.0).round() as u64;
            warnings.push(HighRiskSendWarning {
                percent: Some(pct),
                symbol: Some(request.symbol.clone()),
                ..make("large_send")
            });
        }
    }

    // 5-10. Cross-chain prefix mismatch checks.
    let lowered = request.destination_input.to_lowercase();
    let is_evm = matches!(
        chain_name.as_str(),
        "Ethereum"
            | "Ethereum Classic"
            | "Arbitrum"
            | "Optimism"
            | "BNB Chain"
            | "Avalanche"
            | "Hyperliquid"
    );
    let is_l2 = matches!(
        chain_name.as_str(),
        "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche" | "Hyperliquid"
    );
    let is_ens_candidate =
        lowered.ends_with(".eth") && !lowered.contains(' ') && !lowered.starts_with("0x");

    if is_evm {
        let looks_non_evm = lowered.starts_with("bc1")
            || lowered.starts_with("tb1")
            || lowered.starts_with("ltc1")
            || lowered.starts_with("bnb1")
            || lowered.starts_with('t')
            || lowered.starts_with('d')
            || lowered.starts_with('a');
        if looks_non_evm {
            warnings.push(HighRiskSendWarning {
                chain: Some(chain_name.clone()),
                ..make("non_evm_on_evm")
            });
        }
        if is_l2 && is_ens_candidate {
            warnings.push(HighRiskSendWarning {
                chain: Some(chain_name.clone()),
                ..make("ens_on_l2")
            });
        }
    } else if crate::registry::Chain::from_display_name(chain_name)
        .is_some_and(|c| c.flags_evm_address_as_wrong_chain())
    {
        if lowered.starts_with("0x") || is_ens_candidate {
            warnings.push(HighRiskSendWarning {
                chain: Some(chain_name.clone()),
                ..make("eth_on_utxo")
            });
        }
    } else if chain_name == "Tron" {
        if lowered.starts_with("0x") || lowered.starts_with("bc1") {
            warnings.push(make("non_tron"));
        }
    } else if chain_name == "Solana" {
        if lowered.starts_with("0x")
            || lowered.starts_with("bc1")
            || lowered.starts_with("ltc1")
            || lowered.starts_with('t')
        {
            warnings.push(make("non_solana"));
        }
    } else if chain_name == "XRP Ledger" {
        if lowered.starts_with("0x") || lowered.starts_with("bc1") || lowered.starts_with('t') {
            warnings.push(make("non_xrp"));
        }
    } else if chain_name == "Monero"
        && (lowered.starts_with("0x") || lowered.starts_with("bc1") || lowered.starts_with('r'))
    {
        warnings.push(make("non_monero"));
    }

    // 11. Wallet-chain context mismatch.
    if !request.wallet_selected_chain.is_empty() && request.wallet_selected_chain != *chain_name {
        warnings.push(make("chain_mismatch"));
    }

    warnings
}

// ── Merged from flow_helpers.rs ───────────────────────────────────

use crate::SpectraBridgeError;

// ─── EVM chain context string mapping ────────────────────────────────────────
// Returns a tag like "ethereum", "ethereum_sepolia", "ethereum_hoodi",
// "ethereum_classic", "arbitrum", "optimism", "bnb", "avalanche", "hyperliquid",
// or empty string for non-EVM. Each testnet is its own chain row, so the
// `ethereum_network_mode` argument is retained only for FFI back-compat
// with stored wallets — the chain_name alone now uniquely identifies the
// network.

#[uniffi::export]
pub fn core_evm_chain_context_tag(chain_name: String, ethereum_network_mode: String) -> String {
    let _ = ethereum_network_mode; // legacy argument, ignored
    match chain_name.as_str() {
        "Ethereum" => "ethereum".to_string(),
        "Ethereum Sepolia" => "ethereum_sepolia".to_string(),
        "Ethereum Hoodi" => "ethereum_hoodi".to_string(),
        "Ethereum Classic" => "ethereum_classic".to_string(),
        "Ethereum Classic Mordor" => "ethereum_classic_mordor".to_string(),
        "Arbitrum" => "arbitrum".to_string(),
        "Arbitrum Sepolia" => "arbitrum_sepolia".to_string(),
        "Optimism" => "optimism".to_string(),
        "Optimism Sepolia" => "optimism_sepolia".to_string(),
        "Base Sepolia" => "base_sepolia".to_string(),
        "BNB Chain" => "bnb".to_string(),
        "BNB Chain Testnet" => "bnb_testnet".to_string(),
        "Avalanche" => "avalanche".to_string(),
        "Avalanche Fuji" => "avalanche_fuji".to_string(),
        "Polygon Amoy" => "polygon_amoy".to_string(),
        "Hyperliquid" => "hyperliquid".to_string(),
        "Hyperliquid Testnet" => "hyperliquid_testnet".to_string(),
        _ => String::new(),
    }
}

#[uniffi::export]
pub fn core_is_evm_chain(chain_name: String) -> bool {
    !core_evm_chain_context_tag(chain_name, "mainnet".to_string()).is_empty()
}

// ─── Dogecoin derivation index parser ─────────────────────────────────────────

#[uniffi::export]
pub fn core_parse_dogecoin_derivation_index(path: Option<String>, expected_prefix: String) -> Option<i32> {
    let path = path?;
    if !path.starts_with(&expected_prefix) {
        return None;
    }
    let suffix = &path[expected_prefix.len()..];
    suffix.parse::<i32>().ok()
}

// ─── Simple chain risk probe config ──────────────────────────────────────────
// Per-chain static config for the Litecoin/Dogecoin/Solana/XRP/Monero/Sui/Aptos
// branch of Swift's destination-risk probe: display chain name and balance
// label for messages.

#[derive(Debug, Clone, uniffi::Record)]
pub struct SimpleChainRiskProbeConfig {
    pub display_chain_name: String,
    pub balance_label: String,
}

#[uniffi::export]
pub fn core_simple_chain_risk_probe_config(
    chain_name: String,
    symbol: String,
) -> Option<SimpleChainRiskProbeConfig> {
    let (display_chain_name, balance_label) = match chain_name.as_str() {
        "Litecoin" => ("Litecoin", "balance"),
        "Dogecoin" if symbol == "DOGE" => ("Dogecoin", "balance"),
        "Solana" => ("Solana", "SOL balance"),
        "XRP Ledger" => ("XRP", "XRP balance"),
        "Monero" => ("Monero", "XMR balance"),
        "Sui" => ("Sui", "SUI balance"),
        "Aptos" => ("Aptos", "APT balance"),
        _ => return None,
    };
    Some(SimpleChainRiskProbeConfig {
        display_chain_name: display_chain_name.to_string(),
        balance_label: balance_label.to_string(),
    })
}

// ─── Broadcast rebroadcast dispatch table ─────────────────────────────────────
// Maps Swift's BroadcastEntry payload format → (chain_id, result_field, wrap_key,
// extract_field). Returns an error for unknown formats.

#[derive(Debug, Clone, uniffi::Record)]
pub struct RebroadcastDispatch {
    pub chain_id: String,
    pub result_field: String,
    pub wrap_key: Option<String>,
    pub extract_field: Option<String>,
}

#[uniffi::export]
pub fn core_rebroadcast_dispatch_for_format(
    format: String,
) -> Result<RebroadcastDispatch, SpectraBridgeError> {
    // Keep chain IDs aligned with SpectraChainID in Swift.
    // 0 bitcoin, 1 bitcoin_cash, 2 bitcoin_sv, 3 litecoin, 4 dogecoin,
    // 5 ethereum, 6 tron, 7 solana, 8 xrp, 9 stellar, 10 monero,
    // 11 cardano, 12 sui, 13 aptos, 14 ton, 15 icp, 16 near, 17 polkadot
    let entry: Option<RebroadcastDispatch> = match format.as_str() {
        "bitcoin.raw_hex" => Some(RebroadcastDispatch { chain_id: "bitcoin".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "bitcoin_cash.raw_hex" => Some(RebroadcastDispatch { chain_id: "bitcoin-cash".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "bitcoin_sv.raw_hex" => Some(RebroadcastDispatch { chain_id: "bitcoin-sv".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "litecoin.raw_hex" => Some(RebroadcastDispatch { chain_id: "litecoin".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "dogecoin.raw_hex" => Some(RebroadcastDispatch { chain_id: "dogecoin".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "tron.signed_json" => Some(RebroadcastDispatch { chain_id: "tron".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "solana.base64" => Some(RebroadcastDispatch { chain_id: "solana".into(), result_field: "signature".into(), wrap_key: None, extract_field: None }),
        "xrp.blob_hex" => Some(RebroadcastDispatch { chain_id: "xrp".into(), result_field: "txid".into(), wrap_key: Some("tx_blob_hex".into()), extract_field: None }),
        "stellar.xdr" => Some(RebroadcastDispatch { chain_id: "stellar".into(), result_field: "txid".into(), wrap_key: Some("signed_xdr_b64".into()), extract_field: None }),
        "cardano.cbor_hex" => Some(RebroadcastDispatch { chain_id: "cardano".into(), result_field: "txid".into(), wrap_key: Some("cbor_hex".into()), extract_field: None }),
        "near.base64" => Some(RebroadcastDispatch { chain_id: "near".into(), result_field: "txid".into(), wrap_key: Some("signed_tx_b64".into()), extract_field: None }),
        "polkadot.extrinsic_hex" => Some(RebroadcastDispatch { chain_id: "polkadot".into(), result_field: "txid".into(), wrap_key: Some("extrinsic_hex".into()), extract_field: None }),
        "aptos.signed_json" => Some(RebroadcastDispatch { chain_id: "aptos".into(), result_field: "txid".into(), wrap_key: Some("signed_body_json".into()), extract_field: None }),
        "ton.boc" => Some(RebroadcastDispatch { chain_id: "ton".into(), result_field: "message_hash".into(), wrap_key: Some("boc_b64".into()), extract_field: None }),
        "bitcoin.rust_json" => Some(RebroadcastDispatch { chain_id: "bitcoin".into(), result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "bitcoin_cash.rust_json" => Some(RebroadcastDispatch { chain_id: "bitcoin-cash".into(), result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "bitcoin_sv.rust_json" => Some(RebroadcastDispatch { chain_id: "bitcoin-sv".into(), result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "litecoin.rust_json" => Some(RebroadcastDispatch { chain_id: "litecoin".into(), result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "dogecoin.rust_json" => Some(RebroadcastDispatch { chain_id: "dogecoin".into(), result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "solana.rust_json" => Some(RebroadcastDispatch { chain_id: "solana".into(), result_field: "signature".into(), wrap_key: None, extract_field: Some("signed_tx_base64".into()) }),
        "tron.rust_json" => Some(RebroadcastDispatch { chain_id: "tron".into(), result_field: "txid".into(), wrap_key: None, extract_field: Some("signed_tx_json".into()) }),
        "xrp.rust_json" => Some(RebroadcastDispatch { chain_id: "xrp".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "stellar.rust_json" => Some(RebroadcastDispatch { chain_id: "stellar".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "cardano.rust_json" => Some(RebroadcastDispatch { chain_id: "cardano".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "polkadot.rust_json" => Some(RebroadcastDispatch { chain_id: "polkadot".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "sui.rust_json" => Some(RebroadcastDispatch { chain_id: "sui".into(), result_field: "digest".into(), wrap_key: None, extract_field: None }),
        "aptos.rust_json" => Some(RebroadcastDispatch { chain_id: "aptos".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "ton.rust_json" => Some(RebroadcastDispatch { chain_id: "ton".into(), result_field: "message_hash".into(), wrap_key: None, extract_field: None }),
        "near.rust_json" => Some(RebroadcastDispatch { chain_id: "near".into(), result_field: "txid".into(), wrap_key: None, extract_field: None }),
        _ => None,
    };
    entry.ok_or_else(|| SpectraBridgeError::from("Rebroadcast is not supported for this transaction format yet."))
}

// ─── Rebroadcast prepared payload ────────────────────────────────────────────
// Fuses the dispatch-table lookup with the payload shape transformation so Swift
// never has to build JSON objects or scrape fields for rebroadcast. Handles:
//   • sui.signed_json — remap {txBytesBase64, signatureBase64} → {tx_bytes_b64, sig_b64}
//   • extract_field branch — pull named field value out of a wallet-produced JSON
//   • wrap_key branch — wrap raw payload string under a single JSON key
//   • otherwise — pass payload through unchanged

#[derive(Debug, Clone, uniffi::Record)]
pub struct PreparedBroadcastPayload {
    pub chain_id: String,
    pub broadcast_payload: String,
    pub result_field: String,
}

#[uniffi::export]
pub fn core_rebroadcast_prepare_payload(
    format: String,
    raw_payload: String,
) -> Result<PreparedBroadcastPayload, SpectraBridgeError> {
    if format == "sui.signed_json" {
        let remapped = sui_signed_json_remap(&raw_payload).unwrap_or_else(|| raw_payload.clone());
        return Ok(PreparedBroadcastPayload {
            chain_id: "sui".into(),
            broadcast_payload: remapped,
            result_field: "digest".to_string(),
        });
    }
    let dispatch = core_rebroadcast_dispatch_for_format(format)?;
    let broadcast_payload = if let Some(extract_field) = dispatch.extract_field.as_ref() {
        crate::send::preview_decode::extract_json_string_field(raw_payload.clone(), extract_field.clone())
    } else if let Some(wrap_key) = dispatch.wrap_key.as_ref() {
        let mut map = serde_json::Map::new();
        map.insert(wrap_key.clone(), serde_json::Value::String(raw_payload.clone()));
        serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or(raw_payload)
    } else {
        raw_payload
    };
    Ok(PreparedBroadcastPayload {
        chain_id: dispatch.chain_id,
        broadcast_payload,
        result_field: dispatch.result_field,
    })
}

fn sui_signed_json_remap(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = v.as_object()?;
    let tx = obj.get("txBytesBase64")?.as_str()?;
    let sig = obj.get("signatureBase64")?.as_str()?;
    let remapped = serde_json::json!({ "tx_bytes_b64": tx, "sig_b64": sig });
    serde_json::to_string(&remapped).ok()
}

// ─── Seed derivation chain raw lookup ────────────────────────────────────────

/// Returns the canonical "raw" derivation-chain name for a given chain row.
/// Testnets share their mainnet counterpart's derivation engine, so e.g.
/// `"Ethereum Sepolia"` returns `"Ethereum"`. The Chain enum is the source
/// of truth for that mapping.
#[uniffi::export]
pub fn core_seed_derivation_chain_raw(chain_name: String) -> Option<String> {
    let chain = crate::registry::Chain::from_display_name(&chain_name)?;
    let mainnet = chain.mainnet_counterpart();
    // Some EVM L1/L2/sidechains (BNB Chain, Optimism, etc.) reuse Ethereum's
    // derivation path; the historical raw-name table preserved that
    // collapsing. Mirror it here.
    let raw = match mainnet {
        crate::registry::Chain::BnbChain => "Ethereum",
        c => c.chain_display_name(),
    };
    Some(raw.to_string())
}

#[uniffi::export]
pub fn core_supports_deep_utxo_discovery(chain_name: String) -> bool {
    crate::registry::Chain::from_display_name(&chain_name)
        .is_some_and(|c| c.supports_deep_utxo_discovery())
}

// ─── Receive address resolver dispatch ───────────────────────────────────────
// Centralizes the `(symbol, chain_name, is_evm_chain)` → resolver routing that
// `receiveAddress()` in Swift previously encoded as nested switches.

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ReceiveAddressResolverKind {
    BitcoinLegacy,
    BitcoinCash,
    BitcoinSv,
    Litecoin,
    DogecoinNone,
    Evm,
    Tron,
    Solana,
    Cardano,
    Xrp,
    Stellar,
    Monero,
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
    None,
}

#[uniffi::export]
pub fn core_plan_receive_address_resolver(
    symbol: String,
    chain_name: String,
    is_evm_chain: bool,
) -> ReceiveAddressResolverKind {
    // Collapse testnets onto their mainnet counterpart so the resolver
    // dispatch table stays mainnet-only. Both share the same derivation
    // engine + address shape; only the network parameter differs.
    let dispatch_name: String = crate::registry::Chain::from_display_name(&chain_name)
        .map(|c| c.mainnet_counterpart().chain_display_name().to_string())
        .unwrap_or(chain_name.clone());
    match (symbol.as_str(), dispatch_name.as_str()) {
        ("BTC", _) => ReceiveAddressResolverKind::BitcoinLegacy,
        ("BCH", "Bitcoin Cash") => ReceiveAddressResolverKind::BitcoinCash,
        ("BSV", "Bitcoin SV") => ReceiveAddressResolverKind::BitcoinSv,
        ("LTC", "Litecoin") => ReceiveAddressResolverKind::Litecoin,
        ("DOGE", "Dogecoin") => ReceiveAddressResolverKind::DogecoinNone,
        _ if is_evm_chain => ReceiveAddressResolverKind::Evm,
        (_, "Tron") => ReceiveAddressResolverKind::Tron,
        (_, "Solana") => ReceiveAddressResolverKind::Solana,
        (_, "Cardano") => ReceiveAddressResolverKind::Cardano,
        (_, "XRP Ledger") => ReceiveAddressResolverKind::Xrp,
        (_, "Stellar") => ReceiveAddressResolverKind::Stellar,
        (_, "Monero") => ReceiveAddressResolverKind::Monero,
        (_, "Sui") => ReceiveAddressResolverKind::Sui,
        (_, "Aptos") => ReceiveAddressResolverKind::Aptos,
        (_, "TON") => ReceiveAddressResolverKind::Ton,
        (_, "Internet Computer") => ReceiveAddressResolverKind::Icp,
        (_, "NEAR") => ReceiveAddressResolverKind::Near,
        (_, "Polkadot") => ReceiveAddressResolverKind::Polkadot,
        ("ZEC", "Zcash") => ReceiveAddressResolverKind::Zcash,
        ("BTG", "Bitcoin Gold") => ReceiveAddressResolverKind::BitcoinGold,
        ("DCR", "Decred") => ReceiveAddressResolverKind::Decred,
        ("KAS", "Kaspa") => ReceiveAddressResolverKind::Kaspa,
        ("DASH", "Dash") => ReceiveAddressResolverKind::Dash,
        ("TAO", "Bittensor") => ReceiveAddressResolverKind::Bittensor,
        _ => ReceiveAddressResolverKind::None,
    }
}

// ─── EVM contract-code detection ─────────────────────────────────────────────
// Lifted from Swift `evmHasContractCode`: a nonempty `eth_getCode` result
// (anything other than "0x" or "0x0") indicates deployed bytecode.

pub fn core_evm_has_contract_code(code: String) -> bool {
    let trimmed = code.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("0x")
        && !trimmed.eq_ignore_ascii_case("0x0")
}

// ─── EVM replacement fee bump calculator ─────────────────────────────────────
// When preparing a speed-up / cancel replacement, Swift bumps existing custom
// fees by 20% with a 0.1 gwei floor (or falls back to defaults if either input
// is missing / blank). Returns formatted strings (3 decimals) the way Swift
// renders them into the composer fields.

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmReplacementFeeBump {
    pub max_fee_gwei: String,
    pub priority_fee_gwei: String,
}

#[uniffi::export]
pub fn core_evm_replacement_fee_bump(
    existing_max_fee_gwei: Option<String>,
    existing_priority_fee_gwei: Option<String>,
    default_max_fee_gwei: f64,
    default_priority_fee_gwei: f64,
) -> EvmReplacementFeeBump {
    let parse = |s: Option<&str>| -> Option<f64> {
        s.and_then(|v| {
            let trimmed = v.trim();
            if trimmed.is_empty() { None } else { trimmed.parse::<f64>().ok() }
        })
    };
    let have_max = parse(existing_max_fee_gwei.as_deref());
    let have_pri = parse(existing_priority_fee_gwei.as_deref());
    if have_max.is_none() || have_pri.is_none() {
        return EvmReplacementFeeBump {
            max_fee_gwei: format!("{:.1}", default_max_fee_gwei),
            priority_fee_gwei: format!("{:.1}", default_priority_fee_gwei),
        };
    }
    let bumped_max = (have_max.unwrap() * 1.2).max(0.1);
    let bumped_pri = (have_pri.unwrap() * 1.2).max(0.1);
    EvmReplacementFeeBump {
        max_fee_gwei: format!("{:.3}", bumped_max),
        priority_fee_gwei: format!("{:.3}", bumped_pri),
    }
}

#[cfg(test)]
mod flow_helpers_tests {
    use super::*;

    #[test]
    fn evm_chain_context_ethereum_sepolia() {
        // After the testnet-as-separate-chain migration, Sepolia is its own
        // chain row. The legacy `network_mode` argument is preserved for
        // FFI back-compat but no longer used.
        assert_eq!(
            core_evm_chain_context_tag("Ethereum Sepolia".to_string(), String::new()),
            "ethereum_sepolia"
        );
        assert_eq!(
            core_evm_chain_context_tag("Ethereum".to_string(), "ignored".to_string()),
            "ethereum"
        );
    }

    #[test]
    fn evm_chain_context_non_evm() {
        assert_eq!(
            core_evm_chain_context_tag("Bitcoin".to_string(), "mainnet".to_string()),
            ""
        );
    }

    #[test]
    fn parse_dogecoin_index() {
        assert_eq!(
            core_parse_dogecoin_derivation_index(Some("m/44'/3'/0'/0/7".to_string()), "m/44'/3'/0'/0/".to_string()),
            Some(7)
        );
        assert_eq!(
            core_parse_dogecoin_derivation_index(Some("other".to_string()), "m/44'/3'/0'/0/".to_string()),
            None
        );
    }

    #[test]
    fn rebroadcast_dispatch_btc() {
        let d = core_rebroadcast_dispatch_for_format("bitcoin.raw_hex".to_string()).unwrap();
        assert_eq!(d.chain_id, "bitcoin");
        assert_eq!(d.result_field, "txid");
    }

    #[test]
    fn rebroadcast_dispatch_unknown_errors() {
        assert!(core_rebroadcast_dispatch_for_format("nope".to_string()).is_err());
    }

    #[test]
    fn evm_has_contract_code_variants() {
        assert!(!core_evm_has_contract_code("0x".to_string()));
        assert!(!core_evm_has_contract_code("0X0".to_string()));
        assert!(!core_evm_has_contract_code("   0x ".to_string()));
        assert!(!core_evm_has_contract_code(String::new()));
        assert!(core_evm_has_contract_code("0x60806040".to_string()));
    }

    #[test]
    fn evm_bump_defaults_when_blank() {
        let r = core_evm_replacement_fee_bump(None, Some(" ".to_string()), 4.0, 2.0);
        assert_eq!(r.max_fee_gwei, "4.0");
        assert_eq!(r.priority_fee_gwei, "2.0");
    }

    #[test]
    fn evm_bump_scales_existing() {
        let r = core_evm_replacement_fee_bump(
            Some("5.0".to_string()), Some("2.5".to_string()), 4.0, 2.0,
        );
        assert_eq!(r.max_fee_gwei, "6.000");
        assert_eq!(r.priority_fee_gwei, "3.000");
    }

    #[test]
    fn prepare_payload_sui_signed_json_remap() {
        let raw = r#"{"txBytesBase64":"AAAA","signatureBase64":"BBBB"}"#;
        let p = core_rebroadcast_prepare_payload("sui.signed_json".into(), raw.into()).unwrap();
        assert_eq!(p.chain_id, "sui");
        assert_eq!(p.result_field, "digest");
        let parsed: serde_json::Value = serde_json::from_str(&p.broadcast_payload).unwrap();
        assert_eq!(parsed["tx_bytes_b64"], "AAAA");
        assert_eq!(parsed["sig_b64"], "BBBB");
    }

    #[test]
    fn prepare_payload_sui_malformed_passthrough() {
        let raw = "not json";
        let p = core_rebroadcast_prepare_payload("sui.signed_json".into(), raw.into()).unwrap();
        assert_eq!(p.broadcast_payload, raw);
    }

    #[test]
    fn prepare_payload_wrap_key() {
        let p = core_rebroadcast_prepare_payload("xrp.blob_hex".into(), "deadbeef".into()).unwrap();
        assert_eq!(p.chain_id, "xrp");
        assert_eq!(p.result_field, "txid");
        let parsed: serde_json::Value = serde_json::from_str(&p.broadcast_payload).unwrap();
        assert_eq!(parsed["tx_blob_hex"], "deadbeef");
    }

    #[test]
    fn prepare_payload_extract_field() {
        let raw = r#"{"raw_tx_hex":"ff00","other":"x"}"#;
        let p = core_rebroadcast_prepare_payload("bitcoin.rust_json".into(), raw.into()).unwrap();
        assert_eq!(p.chain_id, "bitcoin");
        assert_eq!(p.broadcast_payload, "ff00");
    }

    #[test]
    fn prepare_payload_passthrough() {
        let p = core_rebroadcast_prepare_payload("bitcoin.raw_hex".into(), "abcd".into()).unwrap();
        assert_eq!(p.broadcast_payload, "abcd");
    }

    #[test]
    fn prepare_payload_unknown_errors() {
        assert!(core_rebroadcast_prepare_payload("nope".into(), "x".into()).is_err());
    }

    #[test]
    fn evm_bump_respects_floor() {
        let r = core_evm_replacement_fee_bump(
            Some("0.01".to_string()), Some("0.01".to_string()), 4.0, 2.0,
        );
        assert_eq!(r.max_fee_gwei, "0.100");
        assert_eq!(r.priority_fee_gwei, "0.100");
    }

}

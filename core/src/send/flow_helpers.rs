// Pure logic lifts from Swift AppState+SendFlow.swift.
// No IO, no SwiftUI, no Keychain — just mappings, validators, and small parsers.

use crate::SpectraBridgeError;

// ─── Ethereum send error mapping ─────────────────────────────────────────────

#[uniffi::export]
pub fn core_map_ethereum_send_error(message: String) -> String {
    let lower = message.to_lowercase();
    if lower.contains("nonce too low") {
        return "Nonce too low. A newer transaction from this wallet is already known. Refresh and retry.".to_string();
    }
    if lower.contains("replacement transaction underpriced") {
        return "Replacement transaction underpriced. Increase fees and retry.".to_string();
    }
    if lower.contains("already known") {
        return "This transaction is already in the mempool.".to_string();
    }
    if lower.contains("insufficient funds") {
        return "Insufficient ETH to cover value plus network fee.".to_string();
    }
    if lower.contains("max fee per gas less than block base fee") {
        return "Max fee is below current base fee. Increase Max Fee and retry.".to_string();
    }
    if lower.contains("intrinsic gas too low") {
        return "Gas limit is too low for this transaction.".to_string();
    }
    message
}

// ─── Tron send error user-facing mapping ─────────────────────────────────────

#[uniffi::export]
pub fn core_user_facing_tron_send_error(message: String) -> String {
    let lower = message.to_lowercase();
    if lower.contains("timed out") {
        return "Tron network request timed out. Please try again.".to_string();
    }
    if lower.contains("not connected") || lower.contains("offline") {
        return "No network connection. Check your internet and retry.".to_string();
    }
    message
}

// ─── Address book validation message ─────────────────────────────────────────

#[uniffi::export]
pub fn core_address_book_validation_message(
    chain_name: String,
    is_empty: bool,
    is_valid: bool,
) -> String {
    if is_empty {
        return match chain_name.as_str() {
            "Bitcoin" => "Enter a Bitcoin address valid for the selected Bitcoin network mode.".to_string(),
            "Dogecoin" => "Dogecoin addresses usually start with D, A, or 9.".to_string(),
            "Ethereum" => "Ethereum addresses must start with 0x and include 40 hex characters.".to_string(),
            "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche" | "Hyperliquid" =>
                format!("{} addresses use EVM format (0x + 40 hex characters).", chain_name),
            "Tron" => "Tron addresses usually start with T and are Base58 encoded.".to_string(),
            "Solana" => "Solana addresses are Base58 encoded and typically 32-44 characters.".to_string(),
            "Cardano" => "Cardano addresses typically start with addr1 and use bech32 format.".to_string(),
            "XRP Ledger" => "XRP Ledger addresses start with r and are Base58 encoded.".to_string(),
            "Stellar" => "Stellar addresses start with G and are StrKey encoded.".to_string(),
            "Monero" => "Monero addresses are Base58 encoded and usually start with 4 or 8.".to_string(),
            "Sui" | "Aptos" => format!("{} addresses are hex and typically start with 0x.", chain_name),
            "TON" => "TON addresses are usually user-friendly strings like UQ... or raw 0:<hex> addresses.".to_string(),
            "NEAR" => "NEAR addresses can be named accounts or 64-character implicit account IDs.".to_string(),
            "Polkadot" => "Polkadot addresses use SS58 encoding and usually start with 1.".to_string(),
            _ => "Enter an address for the selected chain.".to_string(),
        };
    }
    if is_valid {
        return format!("Valid {} address.", chain_name);
    }
    match chain_name.as_str() {
        "Bitcoin" => "Enter a valid Bitcoin address for the selected Bitcoin network mode.".to_string(),
        "Dogecoin" => "Enter a valid Dogecoin address beginning with D, A, or 9.".to_string(),
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche" | "Hyperliquid" =>
            format!("Enter a valid {} address (0x + 40 hex characters).", chain_name),
        "Tron" => "Enter a valid Tron address (starts with T).".to_string(),
        "Solana" => "Enter a valid Solana address (Base58 format).".to_string(),
        "Cardano" => "Enter a valid Cardano address (starts with addr1).".to_string(),
        "XRP Ledger" => "Enter a valid XRP address (starts with r).".to_string(),
        "Stellar" => "Enter a valid Stellar address (starts with G).".to_string(),
        "Monero" => "Enter a valid Monero address (starts with 4 or 8).".to_string(),
        "Sui" | "Aptos" => format!("Enter a valid {} address (starts with 0x).", chain_name),
        "TON" => "Enter a valid TON address.".to_string(),
        "NEAR" => "Enter a valid NEAR account ID or implicit address.".to_string(),
        "Polkadot" => "Enter a valid Polkadot SS58 address.".to_string(),
        _ => format!("Enter a valid {} address.", chain_name),
    }
}

// ─── EVM chain context string mapping ────────────────────────────────────────
// Returns a tag like "ethereum", "ethereum_sepolia", "ethereum_hoodi",
// "ethereum_classic", "arbitrum", "optimism", "bnb", "avalanche", "hyperliquid",
// or empty string for non-EVM.

#[uniffi::export]
pub fn core_evm_chain_context_tag(chain_name: String, ethereum_network_mode: String) -> String {
    match chain_name.as_str() {
        "Ethereum" => match ethereum_network_mode.as_str() {
            "sepolia" => "ethereum_sepolia".to_string(),
            "hoodi" => "ethereum_hoodi".to_string(),
            _ => "ethereum".to_string(),
        },
        "Ethereum Classic" => "ethereum_classic".to_string(),
        "Arbitrum" => "arbitrum".to_string(),
        "Optimism" => "optimism".to_string(),
        "BNB Chain" => "bnb".to_string(),
        "Avalanche" => "avalanche".to_string(),
        "Hyperliquid" => "hyperliquid".to_string(),
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

// ─── Display network name / chain title helpers ──────────────────────────────

#[uniffi::export]
pub fn core_display_network_name_for_chain(
    chain_name: String,
    bitcoin_display: String,
    ethereum_display: String,
    dogecoin_display: String,
) -> String {
    match chain_name.as_str() {
        "Bitcoin" => bitcoin_display,
        "Ethereum" => ethereum_display,
        "Dogecoin" => dogecoin_display,
        _ => chain_name,
    }
}

#[uniffi::export]
pub fn core_display_chain_title(chain_name: String, network_name: String) -> String {
    if network_name == chain_name || network_name == "Mainnet" {
        return chain_name;
    }
    format!("{} {}", chain_name, network_name)
}

// ─── Chain destination risk probe message formatter ──────────────────────────
// Given balance/history signals, produce the (warning, info) message strings.

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChainRiskProbeMessages {
    pub warning: Option<String>,
    pub info: Option<String>,
}

#[uniffi::export]
pub fn core_chain_risk_probe_messages(
    chain_name: String,
    balance_label: String,
    balance_non_positive: bool,
    has_history: bool,
) -> ChainRiskProbeMessages {
    let warning = if balance_non_positive && !has_history {
        Some(format!(
            "Warning: this {} address has zero balance and no transaction history. Double-check recipient details.",
            chain_name
        ))
    } else {
        None
    };
    let info = if balance_non_positive && has_history {
        Some(format!(
            "Note: this {} address has transaction history but currently zero {}.",
            chain_name, balance_label
        ))
    } else {
        None
    };
    ChainRiskProbeMessages { warning, info }
}

// ─── Simple chain risk probe config ──────────────────────────────────────────
// Per-chain static config for the Litecoin/Dogecoin/Solana/XRP/Monero/Sui/Aptos
// branch of Swift's destination-risk probe: balance JSON field, divisor to
// reach the display unit, display chain name, and balance label for messages.

#[derive(Debug, Clone, uniffi::Record)]
pub struct SimpleChainRiskProbeConfig {
    pub balance_field: String,
    pub divisor: f64,
    pub display_chain_name: String,
    pub balance_label: String,
}

#[uniffi::export]
pub fn core_simple_chain_risk_probe_config(
    chain_name: String,
    symbol: String,
) -> Option<SimpleChainRiskProbeConfig> {
    let (balance_field, divisor, display_chain_name, balance_label) = match chain_name.as_str() {
        "Litecoin" => ("balance_sat", 1e8, "Litecoin", "balance"),
        "Dogecoin" if symbol == "DOGE" => ("balance_koin", 1e8, "Dogecoin", "balance"),
        "Solana" => ("lamports", 1e9, "Solana", "SOL balance"),
        "XRP Ledger" => ("drops", 1e6, "XRP", "XRP balance"),
        "Monero" => ("piconeros", 1e12, "Monero", "XMR balance"),
        "Sui" => ("mist", 1e9, "Sui", "SUI balance"),
        "Aptos" => ("octas", 1e8, "Aptos", "APT balance"),
        _ => return None,
    };
    Some(SimpleChainRiskProbeConfig {
        balance_field: balance_field.to_string(),
        divisor,
        display_chain_name: display_chain_name.to_string(),
        balance_label: balance_label.to_string(),
    })
}

// ─── Fiat currency display names ─────────────────────────────────────────────

#[uniffi::export]
pub fn core_fiat_currency_display_name(code: String) -> String {
    match code.as_str() {
        "USD" => "US Dollar (USD)",
        "EUR" => "Euro (EUR)",
        "GBP" => "British Pound (GBP)",
        "JPY" => "Japanese Yen (JPY)",
        "CNY" => "Chinese Yuan (CNY)",
        "INR" => "Indian Rupee (INR)",
        "CAD" => "Canadian Dollar (CAD)",
        "AUD" => "Australian Dollar (AUD)",
        "CHF" => "Swiss Franc (CHF)",
        "BRL" => "Brazilian Real (BRL)",
        "SGD" => "Singapore Dollar (SGD)",
        "AED" => "UAE Dirham (AED)",
        _ => return code,
    }
    .to_string()
}

// ─── Broadcast rebroadcast dispatch table ─────────────────────────────────────
// Maps Swift's BroadcastEntry payload format → (chain_id, result_field, wrap_key,
// extract_field). Returns an error for unknown formats.

#[derive(Debug, Clone, uniffi::Record)]
pub struct RebroadcastDispatch {
    pub chain_id: u32,
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
        "bitcoin.raw_hex" => Some(RebroadcastDispatch { chain_id: 0, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "bitcoin_cash.raw_hex" => Some(RebroadcastDispatch { chain_id: 1, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "bitcoin_sv.raw_hex" => Some(RebroadcastDispatch { chain_id: 2, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "litecoin.raw_hex" => Some(RebroadcastDispatch { chain_id: 3, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "dogecoin.raw_hex" => Some(RebroadcastDispatch { chain_id: 4, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "tron.signed_json" => Some(RebroadcastDispatch { chain_id: 6, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "solana.base64" => Some(RebroadcastDispatch { chain_id: 7, result_field: "signature".into(), wrap_key: None, extract_field: None }),
        "xrp.blob_hex" => Some(RebroadcastDispatch { chain_id: 8, result_field: "txid".into(), wrap_key: Some("tx_blob_hex".into()), extract_field: None }),
        "stellar.xdr" => Some(RebroadcastDispatch { chain_id: 9, result_field: "txid".into(), wrap_key: Some("signed_xdr_b64".into()), extract_field: None }),
        "cardano.cbor_hex" => Some(RebroadcastDispatch { chain_id: 11, result_field: "txid".into(), wrap_key: Some("cbor_hex".into()), extract_field: None }),
        "near.base64" => Some(RebroadcastDispatch { chain_id: 16, result_field: "txid".into(), wrap_key: Some("signed_tx_b64".into()), extract_field: None }),
        "polkadot.extrinsic_hex" => Some(RebroadcastDispatch { chain_id: 17, result_field: "txid".into(), wrap_key: Some("extrinsic_hex".into()), extract_field: None }),
        "aptos.signed_json" => Some(RebroadcastDispatch { chain_id: 13, result_field: "txid".into(), wrap_key: Some("signed_body_json".into()), extract_field: None }),
        "ton.boc" => Some(RebroadcastDispatch { chain_id: 14, result_field: "message_hash".into(), wrap_key: Some("boc_b64".into()), extract_field: None }),
        "bitcoin.rust_json" => Some(RebroadcastDispatch { chain_id: 0, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "bitcoin_cash.rust_json" => Some(RebroadcastDispatch { chain_id: 1, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "bitcoin_sv.rust_json" => Some(RebroadcastDispatch { chain_id: 2, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "litecoin.rust_json" => Some(RebroadcastDispatch { chain_id: 3, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "dogecoin.rust_json" => Some(RebroadcastDispatch { chain_id: 4, result_field: "txid".into(), wrap_key: None, extract_field: Some("raw_tx_hex".into()) }),
        "solana.rust_json" => Some(RebroadcastDispatch { chain_id: 7, result_field: "signature".into(), wrap_key: None, extract_field: Some("signed_tx_base64".into()) }),
        "tron.rust_json" => Some(RebroadcastDispatch { chain_id: 6, result_field: "txid".into(), wrap_key: None, extract_field: Some("signed_tx_json".into()) }),
        "xrp.rust_json" => Some(RebroadcastDispatch { chain_id: 8, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "stellar.rust_json" => Some(RebroadcastDispatch { chain_id: 9, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "cardano.rust_json" => Some(RebroadcastDispatch { chain_id: 11, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "polkadot.rust_json" => Some(RebroadcastDispatch { chain_id: 17, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "sui.rust_json" => Some(RebroadcastDispatch { chain_id: 12, result_field: "digest".into(), wrap_key: None, extract_field: None }),
        "aptos.rust_json" => Some(RebroadcastDispatch { chain_id: 13, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        "ton.rust_json" => Some(RebroadcastDispatch { chain_id: 14, result_field: "message_hash".into(), wrap_key: None, extract_field: None }),
        "near.rust_json" => Some(RebroadcastDispatch { chain_id: 16, result_field: "txid".into(), wrap_key: None, extract_field: None }),
        _ => None,
    };
    entry.ok_or_else(|| SpectraBridgeError::from("Rebroadcast is not supported for this transaction format yet."))
}

// ─── Seed derivation chain raw lookup ────────────────────────────────────────

#[uniffi::export]
pub fn core_seed_derivation_chain_raw(chain_name: String) -> Option<String> {
    let raw = match chain_name.as_str() {
        "Bitcoin" => "Bitcoin",
        "Bitcoin Cash" => "Bitcoin Cash",
        "Bitcoin SV" => "Bitcoin SV",
        "Litecoin" => "Litecoin",
        "Dogecoin" => "Dogecoin",
        "Ethereum" | "BNB Chain" => "Ethereum",
        "Ethereum Classic" => "Ethereum Classic",
        "Arbitrum" => "Arbitrum",
        "Optimism" => "Optimism",
        "Avalanche" => "Avalanche",
        "Hyperliquid" => "Hyperliquid",
        "Tron" => "Tron",
        "Solana" => "Solana",
        "Stellar" => "Stellar",
        "XRP Ledger" => "XRP Ledger",
        "Cardano" => "Cardano",
        "Sui" => "Sui",
        "Aptos" => "Aptos",
        "TON" => "TON",
        "Internet Computer" => "Internet Computer",
        "NEAR" => "NEAR",
        "Polkadot" => "Polkadot",
        _ => return None,
    };
    Some(raw.to_string())
}

#[uniffi::export]
pub fn core_supports_deep_utxo_discovery(chain_name: String) -> bool {
    matches!(chain_name.as_str(), "Bitcoin" | "Bitcoin Cash" | "Bitcoin SV" | "Litecoin" | "Dogecoin")
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
    None,
}

#[uniffi::export]
pub fn core_plan_receive_address_resolver(
    symbol: String,
    chain_name: String,
    is_evm_chain: bool,
) -> ReceiveAddressResolverKind {
    match (symbol.as_str(), chain_name.as_str()) {
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
        _ => ReceiveAddressResolverKind::None,
    }
}

// ─── EVM contract-code detection ─────────────────────────────────────────────
// Lifted from Swift `evmHasContractCode`: a nonempty `eth_getCode` result
// (anything other than "0x" or "0x0") indicates deployed bytecode.

#[uniffi::export]
pub fn core_evm_has_contract_code(code: String) -> bool {
    let trimmed = code.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("0x")
        && !trimmed.eq_ignore_ascii_case("0x0")
}

// ─── Send balance validation ────────────────────────────────────────────────
// Consolidates the per-chain "can we afford amount + fee?" check that was
// duplicated ~10 times across Swift's `submitSend()`.

#[derive(Debug, Clone, uniffi::Record)]
pub struct SendBalanceValidationRequest {
    /// Amount the user wants to send.
    pub amount: f64,
    /// Estimated network fee (in the fee-paying asset).
    pub network_fee: f64,
    /// Balance of the asset being sent.
    pub holding_balance: f64,
    /// Whether this is a native asset send (fee paid from same balance).
    pub is_native_asset: bool,
    /// Symbol of the asset being sent (e.g. "SOL", "USDT").
    pub symbol: String,
    /// For token sends: symbol of the native asset that pays fees (e.g. "SOL", "TRX").
    pub native_symbol: Option<String>,
    /// For token sends: balance of the native asset that pays fees.
    pub native_balance: Option<f64>,
    /// Decimal precision for formatting (6 for most, 8 for BTC, 7 for XLM).
    pub fee_decimals: u32,
    /// Optional chain label for the fee message (e.g. "Tron", "Solana").
    pub chain_label: Option<String>,
}

/// Validate that the wallet has sufficient balance for amount + fee.
/// Returns `Ok(())` on success, `Err(user-facing error message)` on failure.
#[uniffi::export]
pub fn core_validate_send_balance(
    request: SendBalanceValidationRequest,
) -> Result<(), SpectraBridgeError> {
    if request.is_native_asset {
        let total = request.amount + request.network_fee;
        if total > request.holding_balance {
            return Err(SpectraBridgeError::from(
                core_insufficient_funds_for_amount_plus_fee_message(
                    request.symbol,
                    total,
                    request.fee_decimals,
                ),
            ));
        }
    } else {
        // Token send: check token balance for amount, native balance for fee
        if request.amount > request.holding_balance {
            return Err(SpectraBridgeError::from(format!(
                "Insufficient {} balance for this transfer.",
                request.symbol
            )));
        }
        if let (Some(native_sym), Some(native_bal)) =
            (request.native_symbol, request.native_balance)
        {
            if request.network_fee > native_bal {
                return Err(SpectraBridgeError::from(
                    core_insufficient_native_for_fee_message(
                        native_sym,
                        request.network_fee,
                        request.fee_decimals,
                        request.chain_label,
                    ),
                ));
            }
        }
    }
    Ok(())
}

// ─── Insufficient-funds message formatter ────────────────────────────────────
// Produces the "Insufficient X for amount plus network fee (needs ~N.NNNNNN X)."
// string used across every chain's submit path. `decimals` selects precision
// (typically 6 for most chains, 8 for BTC-style, 7 for Stellar).

#[uniffi::export]
pub fn core_insufficient_funds_for_amount_plus_fee_message(
    symbol: String,
    total_needed: f64,
    decimals: u32,
) -> String {
    let d = decimals as usize;
    format!(
        "Insufficient {} for amount plus network fee (needs ~{:.*} {}).",
        symbol, d, total_needed, symbol
    )
}

// Variant for the "cover network fee only" case (token sends where
// fee is paid in native asset).
#[uniffi::export]
pub fn core_insufficient_native_for_fee_message(
    native_symbol: String,
    fee: f64,
    decimals: u32,
    chain_label: Option<String>,
) -> String {
    let d = decimals as usize;
    match chain_label {
        Some(chain) => format!(
            "Insufficient {} to cover {} network fee (~{:.*} {}).",
            native_symbol, chain, d, fee, native_symbol
        ),
        None => format!(
            "Insufficient {} to cover the network fee (~{:.*} {}).",
            native_symbol, d, fee, native_symbol
        ),
    }
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

// ─── EVM preflight reason codes → localized-ready strings ────────────────────
// Given the results of HTTP contract-code probes (recipient and optional token
// contract), produce the set of risk-reason strings Swift previously assembled
// inline. Swift still drives the HTTP calls (they need per-wallet chain config)
// but hands Rust the raw `code` strings plus context.

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmPreflightContractInput {
    pub chain_name: String,
    pub symbol: String,
    pub recipient_code: Option<String>,     // None = probe failed
    pub token_symbol: Option<String>,       // None = native send (no token check)
    pub token_code: Option<String>,         // None = probe failed (only meaningful if token_symbol is Some)
    pub token_probed: bool,                 // did Swift actually attempt the token probe?
}

#[uniffi::export]
pub fn core_evm_preflight_contract_reasons(input: EvmPreflightContractInput) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match &input.recipient_code {
        Some(code) if core_evm_has_contract_code(code.clone()) => {
            out.push(format!(
                "Recipient is a smart contract on {}. Confirm it can receive {} safely.",
                input.chain_name, input.symbol
            ));
        }
        None => {
            out.push(format!(
                "Could not verify recipient contract state on {}. Review destination carefully.",
                input.chain_name
            ));
        }
        _ => {}
    }
    if input.token_probed {
        if let Some(token_sym) = input.token_symbol {
            match &input.token_code {
                Some(code) if !core_evm_has_contract_code(code.clone()) => {
                    out.push(format!(
                        "Token contract {} appears missing on {}. This may be a wrong-network token selection.",
                        token_sym, input.chain_name
                    ));
                }
                None => {
                    out.push(format!(
                        "Could not verify {} contract bytecode on {}.",
                        token_sym, input.chain_name
                    ));
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_eth_nonce_too_low() {
        let out = core_map_ethereum_send_error("Error: Nonce Too Low in pool".to_string());
        assert!(out.starts_with("Nonce too low"));
    }

    #[test]
    fn passes_unknown_eth_error_through() {
        let out = core_map_ethereum_send_error("some weird failure".to_string());
        assert_eq!(out, "some weird failure");
    }

    #[test]
    fn tron_timeout_mapping() {
        assert_eq!(
            core_user_facing_tron_send_error("Request timed out".to_string()),
            "Tron network request timed out. Please try again."
        );
    }

    #[test]
    fn address_book_empty_bitcoin() {
        let msg = core_address_book_validation_message("Bitcoin".to_string(), true, false);
        assert!(msg.contains("Bitcoin address"));
    }

    #[test]
    fn address_book_valid() {
        let msg = core_address_book_validation_message("Ethereum".to_string(), false, true);
        assert_eq!(msg, "Valid Ethereum address.");
    }

    #[test]
    fn evm_chain_context_ethereum_sepolia() {
        assert_eq!(
            core_evm_chain_context_tag("Ethereum".to_string(), "sepolia".to_string()),
            "ethereum_sepolia"
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
        assert_eq!(d.chain_id, 0);
        assert_eq!(d.result_field, "txid");
    }

    #[test]
    fn rebroadcast_dispatch_unknown_errors() {
        assert!(core_rebroadcast_dispatch_for_format("nope".to_string()).is_err());
    }

    #[test]
    fn display_chain_title_mainnet_collapses() {
        assert_eq!(
            core_display_chain_title("Bitcoin".to_string(), "Mainnet".to_string()),
            "Bitcoin"
        );
    }

    #[test]
    fn display_chain_title_with_network() {
        assert_eq!(
            core_display_chain_title("Bitcoin".to_string(), "Testnet".to_string()),
            "Bitcoin Testnet"
        );
    }

    #[test]
    fn risk_probe_warning_path() {
        let m = core_chain_risk_probe_messages("Bitcoin".to_string(), "balance".to_string(), true, false);
        assert!(m.warning.is_some());
        assert!(m.info.is_none());
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
    fn insufficient_funds_message_formats() {
        let m = core_insufficient_funds_for_amount_plus_fee_message(
            "BTC".to_string(), 0.12345678, 8,
        );
        assert_eq!(m, "Insufficient BTC for amount plus network fee (needs ~0.12345678 BTC).");
        let m6 = core_insufficient_funds_for_amount_plus_fee_message(
            "TRX".to_string(), 12.5, 6,
        );
        assert_eq!(m6, "Insufficient TRX for amount plus network fee (needs ~12.500000 TRX).");
    }

    #[test]
    fn insufficient_native_for_fee_message_with_and_without_chain() {
        let with = core_insufficient_native_for_fee_message(
            "TRX".to_string(), 1.2345, 6, Some("Tron".to_string()),
        );
        assert!(with.contains("Tron network fee"));
        assert!(with.contains("1.234500 TRX"));
        let without = core_insufficient_native_for_fee_message(
            "SOL".to_string(), 0.000005, 6, None,
        );
        assert!(without.starts_with("Insufficient SOL to cover the network fee"));
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
    fn evm_bump_respects_floor() {
        let r = core_evm_replacement_fee_bump(
            Some("0.01".to_string()), Some("0.01".to_string()), 4.0, 2.0,
        );
        assert_eq!(r.max_fee_gwei, "0.100");
        assert_eq!(r.priority_fee_gwei, "0.100");
    }

    #[test]
    fn evm_preflight_reports_contract_recipient() {
        let reasons = core_evm_preflight_contract_reasons(EvmPreflightContractInput {
            chain_name: "Ethereum".into(), symbol: "ETH".into(),
            recipient_code: Some("0xabc123".into()),
            token_symbol: None, token_code: None, token_probed: false,
        });
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].contains("smart contract on Ethereum"));
    }

    #[test]
    fn evm_preflight_reports_probe_failure() {
        let reasons = core_evm_preflight_contract_reasons(EvmPreflightContractInput {
            chain_name: "Ethereum".into(), symbol: "ETH".into(),
            recipient_code: None,
            token_symbol: None, token_code: None, token_probed: false,
        });
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].contains("Could not verify recipient"));
    }

    #[test]
    fn evm_preflight_reports_missing_token() {
        let reasons = core_evm_preflight_contract_reasons(EvmPreflightContractInput {
            chain_name: "Ethereum".into(), symbol: "USDC".into(),
            recipient_code: Some("0x".into()),
            token_symbol: Some("USDC".into()),
            token_code: Some("0x".into()),
            token_probed: true,
        });
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].contains("Token contract USDC appears missing"));
    }

    #[test]
    fn risk_probe_info_path() {
        let m = core_chain_risk_probe_messages("Bitcoin".to_string(), "balance".to_string(), true, true);
        assert!(m.warning.is_none());
        assert!(m.info.is_some());
    }

    #[test]
    fn validate_send_balance_native_ok() {
        let r = core_validate_send_balance(SendBalanceValidationRequest {
            amount: 1.0, network_fee: 0.001, holding_balance: 2.0,
            is_native_asset: true, symbol: "SOL".into(),
            native_symbol: None, native_balance: None, fee_decimals: 6,
            chain_label: None,
        });
        assert!(r.is_ok());
    }

    #[test]
    fn validate_send_balance_native_insufficient() {
        let r = core_validate_send_balance(SendBalanceValidationRequest {
            amount: 1.5, network_fee: 0.6, holding_balance: 2.0,
            is_native_asset: true, symbol: "SOL".into(),
            native_symbol: None, native_balance: None, fee_decimals: 6,
            chain_label: None,
        });
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("Insufficient SOL"));
    }

    #[test]
    fn validate_send_balance_token_ok() {
        let r = core_validate_send_balance(SendBalanceValidationRequest {
            amount: 100.0, network_fee: 5.0, holding_balance: 200.0,
            is_native_asset: false, symbol: "USDT".into(),
            native_symbol: Some("TRX".into()), native_balance: Some(50.0),
            fee_decimals: 6, chain_label: Some("Tron".into()),
        });
        assert!(r.is_ok());
    }

    #[test]
    fn validate_send_balance_token_insufficient_native() {
        let r = core_validate_send_balance(SendBalanceValidationRequest {
            amount: 100.0, network_fee: 5.0, holding_balance: 200.0,
            is_native_asset: false, symbol: "USDT".into(),
            native_symbol: Some("TRX".into()), native_balance: Some(1.0),
            fee_decimals: 6, chain_label: Some("Tron".into()),
        });
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("Insufficient TRX"));
    }

    #[test]
    fn validate_send_balance_token_insufficient_amount() {
        let r = core_validate_send_balance(SendBalanceValidationRequest {
            amount: 300.0, network_fee: 5.0, holding_balance: 200.0,
            is_native_asset: false, symbol: "USDT".into(),
            native_symbol: Some("TRX".into()), native_balance: Some(50.0),
            fee_decimals: 6, chain_label: Some("Tron".into()),
        });
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("Insufficient USDT"));
    }
}

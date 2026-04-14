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
        "Bitcoin" => "bitcoin",
        "Bitcoin Cash" => "bitcoinCash",
        "Bitcoin SV" => "bitcoinSV",
        "Litecoin" => "litecoin",
        "Dogecoin" => "dogecoin",
        "Ethereum" | "BNB Chain" => "ethereum",
        "Ethereum Classic" => "ethereumClassic",
        "Arbitrum" => "arbitrum",
        "Optimism" => "optimism",
        "Avalanche" => "avalanche",
        "Hyperliquid" => "hyperliquid",
        "Tron" => "tron",
        "Solana" => "solana",
        "Stellar" => "stellar",
        "XRP Ledger" => "xrp",
        "Cardano" => "cardano",
        "Sui" => "sui",
        "Aptos" => "aptos",
        "TON" => "ton",
        "Internet Computer" => "internetComputer",
        "NEAR" => "near",
        "Polkadot" => "polkadot",
        _ => return None,
    };
    Some(raw.to_string())
}

#[uniffi::export]
pub fn core_supports_deep_utxo_discovery(chain_name: String) -> bool {
    matches!(chain_name.as_str(), "Bitcoin Cash" | "Bitcoin SV" | "Litecoin")
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
    fn risk_probe_info_path() {
        let m = core_chain_risk_probe_messages("Bitcoin".to_string(), "balance".to_string(), true, true);
        assert!(m.warning.is_none());
        assert!(m.info.is_some());
    }
}

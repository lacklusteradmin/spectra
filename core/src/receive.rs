// Pure receive-address message builder. Swift previously owned a ~45-line
// chain-by-chain switch that produced user-facing strings describing the
// receive state. This module lifts that logic into Rust so Swift can become
// a thin forwarder.

/// Inputs needed to render the user-facing receive-address string for the
/// current wallet + chain selection.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReceiveAddressMessageInput {
    /// The selected coin's chain name (e.g. "Bitcoin", "Ethereum").
    pub chain_name: String,
    /// The selected coin's symbol (e.g. "BTC", "ETH").
    pub symbol: String,
    /// True if `chain_name` is an EVM chain per Swift's catalog.
    pub is_evm_chain: bool,
    /// Address resolved live (via refresh). Empty if not yet resolved.
    pub resolved_address: String,
    /// Persisted / derived address for the chain on this wallet. `None` if
    /// no such address is available (watch-only not configured, seed absent).
    pub chain_address: Option<String>,
    /// True if a seed phrase is stored for the wallet.
    pub has_seed: bool,
    /// True if a watch-only address was typed / imported for the chain.
    pub has_watch_address: bool,
    /// True while a live resolve request is in flight.
    pub is_resolving: bool,
}

#[uniffi::export]
pub fn receive_address_message(input: ReceiveAddressMessageInput) -> String {
    let ReceiveAddressMessageInput {
        chain_name,
        symbol,
        is_evm_chain,
        resolved_address,
        chain_address,
        has_seed,
        has_watch_address,
        is_resolving,
    } = input;

    // Helper: build the UTXO-style message (BTC, BCH, BSV, LTC) which share a
    // common pattern: resolved-first, then fallback address, then missing-seed
    // message, then loading/tap message.
    let utxo_missing_seed_msg = |name: &str, symbol_label: &str| -> String {
        format!(
            "{name} receive unavailable. Open Edit Name and add the seed phrase or {symbol_label} watch address."
        )
    };
    let utxo_loading_msg = |name: &str| -> String {
        if is_resolving {
            format!("Loading {name} receive address...")
        } else {
            format!("Tap Refresh or reopen Receive to resolve a {name} address.")
        }
    };

    let chain_address_trimmed = chain_address
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Bitcoin.
    if symbol == "BTC" {
        if !resolved_address.is_empty() {
            return resolved_address;
        }
        if let Some(addr) = chain_address_trimmed {
            return addr;
        }
        if !has_seed {
            return utxo_missing_seed_msg("Bitcoin", "BTC");
        }
        return utxo_loading_msg("Bitcoin");
    }

    // BCH / BSV / LTC share the same template.
    let utxo_match: Option<(&str, &str)> = match (symbol.as_str(), chain_name.as_str()) {
        ("BCH", "Bitcoin Cash") => Some(("Bitcoin Cash", "BCH")),
        ("BSV", "Bitcoin SV") => Some(("Bitcoin SV", "BSV")),
        ("LTC", "Litecoin") => Some(("Litecoin", "LTC")),
        _ => None,
    };
    if let Some((name, sym)) = utxo_match {
        if !resolved_address.is_empty() {
            return resolved_address;
        }
        if let Some(addr) = chain_address_trimmed {
            return addr;
        }
        if !has_seed {
            return utxo_missing_seed_msg(name, sym);
        }
        return utxo_loading_msg(name);
    }

    // Dogecoin has a slightly different guard (seed OR watch address).
    if symbol == "DOGE" && chain_name == "Dogecoin" {
        if !resolved_address.is_empty() {
            return resolved_address;
        }
        if !has_seed && !has_watch_address {
            return "Dogecoin receive unavailable. Open Edit Name and add a seed phrase or DOGE watch address.".to_string();
        }
        return utxo_loading_msg("Dogecoin");
    }

    // EVM: `chain_address` carries the resolved EVM address if derivable; if
    // absent we return the missing-seed prompt.
    if is_evm_chain {
        match chain_address {
            None => {
                return format!(
                    "{chain_name} receive unavailable. Open Edit Name and add the seed phrase."
                );
            }
            Some(evm) => {
                return if resolved_address.is_empty() { evm } else { resolved_address };
            }
        }
    }

    // "Simple" chains: Swift passes a resolver hint string; we embed it here
    // per chain.
    let simple: Option<&str> = match chain_name.as_str() {
        "Tron" => Some("seed phrase or TRON watch address"),
        "Solana" => Some("seed phrase or SOL watch address"),
        "Cardano" => Some("seed phrase"),
        "XRP Ledger" => Some("seed phrase or XRP watch address"),
        "Stellar" => Some("seed phrase or Stellar watch address"),
        "Monero" => Some("a Monero address"),
        "Sui" => Some("seed phrase or Sui watch address"),
        "Aptos" => Some("seed phrase or Aptos watch address"),
        "TON" => Some("seed phrase or TON watch address"),
        "Internet Computer" => Some("seed phrase or ICP watch address"),
        "NEAR" => Some("seed phrase or NEAR watch address"),
        "Polkadot" => Some("seed phrase or Polkadot watch address"),
        _ => None,
    };
    if let Some(hint) = simple {
        match chain_address {
            None => {
                return format!(
                    "{chain_name} receive unavailable. Open Edit Name and add the {hint}."
                );
            }
            Some(addr) => {
                return if resolved_address.is_empty() { addr } else { resolved_address };
            }
        }
    }

    "Receive is not enabled for this chain.".to_string()
}

/// Picks the user-visible wallet display name given import batch context.
/// Mirrors Swift's `walletDisplayName(baseName:batchPosition:defaultWalletIndex:selectedChainCount:)`.
#[uniffi::export]
pub fn wallet_display_name(
    base_name: String,
    batch_position: i32,
    default_wallet_index: i32,
    selected_chain_count: i32,
) -> String {
    let trimmed = base_name.trim();
    if trimmed.is_empty() {
        return format!("Wallet {default_wallet_index}");
    }
    if selected_chain_count > 1 {
        format!("{trimmed} {batch_position}")
    } else {
        trimmed.to_string()
    }
}

/// Returns the next integer to use for a default "Wallet N" name, given the
/// set of existing wallet names. Mirrors Swift's `nextDefaultWalletNameIndex()`.
#[uniffi::export]
pub fn next_default_wallet_name_index(existing_wallet_names: Vec<String>) -> i32 {
    let mut highest = 0i32;
    for name in existing_wallet_names {
        if let Some(rest) = name.strip_prefix("Wallet ") {
            if let Ok(v) = rest.parse::<i32>() {
                if v > highest {
                    highest = v;
                }
            }
        }
    }
    highest + 1
}

/// Returns true when the chain supports the bip39-style private-key import
/// flow. Mirrors Swift's `importWallet` inclusion check.
#[uniffi::export]
pub fn chain_supports_private_key_import(chain_name: String) -> bool {
    matches!(
        chain_name.as_str(),
        "Bitcoin"
            | "Bitcoin Cash"
            | "Bitcoin SV"
            | "Litecoin"
            | "Dogecoin"
            | "Ethereum"
            | "Ethereum Classic"
            | "Arbitrum"
            | "Optimism"
            | "BNB Chain"
            | "Avalanche"
            | "Hyperliquid"
            | "Tron"
            | "Solana"
            | "Cardano"
            | "XRP Ledger"
            | "Stellar"
            | "Sui"
            | "Aptos"
            | "TON"
            | "Internet Computer"
            | "NEAR"
            | "Polkadot"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ReceiveAddressMessageInput {
        ReceiveAddressMessageInput {
            chain_name: "Bitcoin".into(),
            symbol: "BTC".into(),
            is_evm_chain: false,
            resolved_address: String::new(),
            chain_address: None,
            has_seed: false,
            has_watch_address: false,
            is_resolving: false,
        }
    }

    #[test]
    fn btc_resolved_wins() {
        let mut i = base();
        i.resolved_address = "bc1qabc".into();
        assert_eq!(receive_address_message(i), "bc1qabc");
    }

    #[test]
    fn btc_fallback_to_wallet_address() {
        let mut i = base();
        i.chain_address = Some("  bc1qxyz  ".into());
        assert_eq!(receive_address_message(i), "bc1qxyz");
    }

    #[test]
    fn btc_missing_seed_message() {
        let msg = receive_address_message(base());
        assert!(msg.starts_with("Bitcoin receive unavailable"));
    }

    #[test]
    fn btc_loading_when_seed_present() {
        let mut i = base();
        i.has_seed = true;
        i.is_resolving = true;
        assert_eq!(receive_address_message(i), "Loading Bitcoin receive address...");
    }

    #[test]
    fn bch_template() {
        let mut i = base();
        i.symbol = "BCH".into();
        i.chain_name = "Bitcoin Cash".into();
        i.has_seed = true;
        let msg = receive_address_message(i);
        assert_eq!(msg, "Tap Refresh or reopen Receive to resolve a Bitcoin Cash address.");
    }

    #[test]
    fn doge_requires_seed_or_watch() {
        let mut i = base();
        i.symbol = "DOGE".into();
        i.chain_name = "Dogecoin".into();
        let msg = receive_address_message(i);
        assert!(msg.contains("Dogecoin receive unavailable"));
    }

    #[test]
    fn doge_watch_only_is_enough() {
        let mut i = base();
        i.symbol = "DOGE".into();
        i.chain_name = "Dogecoin".into();
        i.has_watch_address = true;
        assert_eq!(
            receive_address_message(i),
            "Tap Refresh or reopen Receive to resolve a Dogecoin address."
        );
    }

    #[test]
    fn evm_returns_derived_address_when_resolved_empty() {
        let mut i = base();
        i.chain_name = "Ethereum".into();
        i.symbol = "ETH".into();
        i.is_evm_chain = true;
        i.chain_address = Some("0xabc".into());
        assert_eq!(receive_address_message(i), "0xabc");
    }

    #[test]
    fn evm_unresolvable() {
        let mut i = base();
        i.chain_name = "Arbitrum".into();
        i.symbol = "ETH".into();
        i.is_evm_chain = true;
        let msg = receive_address_message(i);
        assert!(msg.starts_with("Arbitrum receive unavailable"));
    }

    #[test]
    fn simple_chain_tron_unresolvable() {
        let mut i = base();
        i.chain_name = "Tron".into();
        i.symbol = "TRX".into();
        let msg = receive_address_message(i);
        assert!(msg.contains("seed phrase or TRON watch address"));
    }

    #[test]
    fn simple_chain_tron_resolved() {
        let mut i = base();
        i.chain_name = "Tron".into();
        i.symbol = "TRX".into();
        i.chain_address = Some("TXYZ".into());
        assert_eq!(receive_address_message(i), "TXYZ");
    }

    #[test]
    fn wallet_display_name_variants() {
        assert_eq!(wallet_display_name("".into(), 2, 5, 3), "Wallet 5");
        assert_eq!(wallet_display_name("   ".into(), 2, 7, 3), "Wallet 7");
        assert_eq!(wallet_display_name("Main".into(), 2, 1, 3), "Main 2");
        assert_eq!(wallet_display_name("Main".into(), 2, 1, 1), "Main");
    }

    #[test]
    fn next_default_wallet_name_index_finds_highest() {
        let names = vec![
            "Wallet 1".into(),
            "Wallet 5".into(),
            "Ugly".into(),
            "Wallet xx".into(),
        ];
        assert_eq!(next_default_wallet_name_index(names), 6);
        assert_eq!(next_default_wallet_name_index(vec![]), 1);
    }

    #[test]
    fn private_key_import_support_table() {
        assert!(chain_supports_private_key_import("Bitcoin".into()));
        assert!(chain_supports_private_key_import("Ethereum".into()));
        assert!(chain_supports_private_key_import("Polkadot".into()));
        assert!(!chain_supports_private_key_import("Monero".into()));
        assert!(!chain_supports_private_key_import("Unknown".into()));
    }

    #[test]
    fn unknown_chain_disabled() {
        let mut i = base();
        i.chain_name = "Nothing".into();
        i.symbol = "XYZ".into();
        assert_eq!(receive_address_message(i), "Receive is not enabled for this chain.");
    }
}

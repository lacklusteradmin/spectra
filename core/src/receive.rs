// Builds the user-facing string shown on the receive screen — the address
// itself, or an explanation of why there isn't one yet.

use crate::registry::Chain;

/// Said for a chain the registry does not know and for one with no receive
/// path — the screen has nothing to show either way.
const NOT_ENABLED: &str = "Receive is not enabled for this chain.";

/// Inputs needed to render the user-facing receive-address string for the
/// current wallet + chain selection.
///
/// It also carried the coin's symbol and an `is_evm_chain` flag Swift computed
/// from its own catalog — a chain fact answered off-registry, and the reason
/// the branches below matched on `("BCH", "Bitcoin Cash")` string pairs while
/// the enum that knows both sat one call away.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReceiveAddressMessageInput {
    /// The selected coin's chain name (e.g. "Bitcoin", "Ethereum").
    pub chain_name: String,
    /// Address resolved live (via refresh). Empty if not yet resolved.
    pub resolved_address: String,
    /// Persisted / derived address for the chain on this wallet. `None` if
    /// no such address is available (watch-only not configured, seed absent).
    pub chain_address: Option<String>,
    /// True if a seed phrase is stored for the wallet.
    pub has_seed: bool,
    /// True if a watch-only address was typed / imported for this chain.
    pub has_watch_address: bool,
    /// True while a live resolve request is in flight.
    pub is_resolving: bool,
}

/// What the receive screen tells a user to add when a chain has no address.
///
/// Copy rather than a chain fact, which is why it is a table and not a catalog
/// column — but keyed by the chain, so renaming one in the catalog cannot
/// silently drop it to "Receive is not enabled for this chain" the way a table
/// of display-name strings did.
fn receive_watch_hint(chain: Chain) -> Option<&'static str> {
    Some(match chain {
        Chain::Tron => "seed phrase or TRON watch address",
        Chain::Solana => "seed phrase or SOL watch address",
        Chain::Cardano => "seed phrase",
        Chain::Xrp => "seed phrase or XRP watch address",
        Chain::Stellar => "seed phrase or Stellar watch address",
        Chain::Monero => "a Monero address",
        Chain::Sui => "seed phrase or Sui watch address",
        Chain::Aptos => "seed phrase or Aptos watch address",
        Chain::Ton => "seed phrase or TON watch address",
        Chain::Icp => "seed phrase or ICP watch address",
        Chain::Near => "seed phrase or NEAR watch address",
        Chain::Polkadot => "seed phrase or Polkadot watch address",
        _ => return None,
    })
}

#[uniffi::export]
pub fn receive_address_message(input: ReceiveAddressMessageInput) -> String {
    let ReceiveAddressMessageInput {
        chain_name,
        resolved_address,
        chain_address,
        has_seed,
        has_watch_address,
        is_resolving,
    } = input;

    let Some(chain) = Chain::from_display_name(&chain_name) else {
        return NOT_ENABLED.to_string();
    };
    let name = chain.chain_display_name();
    let loading = || {
        if is_resolving {
            format!("Loading {name} receive address...")
        } else {
            format!("Tap Refresh or reopen Receive to resolve a {name} address.")
        }
    };
    let missing =
        |what: &str| format!("{name} receive unavailable. Open Edit Name and add the {what}.");

    // An address of blanks is not an address. The EVM and "simple" arms used
    // to return the untrimmed value, so a stored `" "` rendered as the receive
    // address itself.
    let chain_address = chain_address
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // The UTXO family — one branch, from the same flag that decides whether a
    // receive index is reserved at all. Four `(symbol, chain name)` pairs used
    // to spell it, so the testnets of three of them fell past every arm to
    // "Receive is not enabled for this chain".
    if chain.supports_deep_utxo_discovery() {
        if !resolved_address.is_empty() {
            return resolved_address;
        }
        if let Some(addr) = chain_address {
            return addr;
        }
        // Dogecoin resolves no address of its own, so a typed watch address is
        // the only thing it can show and the seed is not required.
        let can_resolve = if chain.mainnet_counterpart() == Chain::Dogecoin {
            has_seed || has_watch_address
        } else {
            has_seed
        };
        if !can_resolve {
            return missing(&format!(
                "seed phrase or {} watch address",
                chain.coin_symbol()
            ));
        }
        return loading();
    }

    // EVM: one derived address serves every chain in the family.
    if chain.is_evm() {
        let Some(evm) = chain_address else {
            return missing("seed phrase");
        };
        return if resolved_address.is_empty() {
            evm
        } else {
            resolved_address
        };
    }

    // Everything else resolves one address, and differs only in what a user
    // has to add for it to exist. A testnet asks for its mainnet's.
    let Some(hint) = receive_watch_hint(chain.mainnet_counterpart()) else {
        return NOT_ENABLED.to_string();
    };
    let Some(addr) = chain_address else {
        return missing(hint);
    };
    if resolved_address.is_empty() {
        addr
    } else {
        resolved_address
    }
}

/// Returns the next integer to use for a default "Wallet N" name, given the
/// set of existing wallet names.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ReceiveAddressMessageInput {
        ReceiveAddressMessageInput {
            chain_name: "Bitcoin".into(),
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
        assert_eq!(
            receive_address_message(i),
            "Loading Bitcoin receive address..."
        );
    }

    #[test]
    fn bch_template() {
        let mut i = base();
        i.chain_name = "Bitcoin Cash".into();
        i.has_seed = true;
        let msg = receive_address_message(i);
        assert_eq!(
            msg,
            "Tap Refresh or reopen Receive to resolve a Bitcoin Cash address."
        );
    }

    #[test]
    fn doge_requires_seed_or_watch() {
        let mut i = base();
        i.chain_name = "Dogecoin".into();
        let msg = receive_address_message(i);
        assert!(msg.contains("Dogecoin receive unavailable"));
    }

    #[test]
    fn doge_watch_only_is_enough() {
        let mut i = base();
        i.chain_name = "Dogecoin".into();
        i.has_watch_address = true;
        assert_eq!(
            receive_address_message(i),
            "Tap Refresh or reopen Receive to resolve a Dogecoin address."
        );
    }

    /// Four `(symbol, chain name)` pairs spelled this family, so Litecoin's
    /// testnet matched none of them and was told receive is not enabled.
    #[test]
    fn a_utxo_testnet_gets_its_family_message() {
        let mut i = base();
        i.chain_name = "Litecoin Testnet".into();
        assert_eq!(
            receive_address_message(i),
            "Litecoin Testnet receive unavailable. Open Edit Name and add the seed phrase or LTC watch address."
        );
    }

    /// A chain the registry does not know, and one it knows with no receive
    /// path, say the same thing.
    #[test]
    fn an_unknown_chain_and_a_pathless_one_both_refuse() {
        let mut unknown = base();
        unknown.chain_name = "Nowhere".into();
        assert_eq!(receive_address_message(unknown), NOT_ENABLED);
        let mut kaspa = base();
        kaspa.chain_name = "Kaspa".into();
        assert_eq!(receive_address_message(kaspa), NOT_ENABLED);
    }

    /// An address of blanks used to be returned as the receive address.
    #[test]
    fn a_blank_stored_address_is_not_an_address() {
        let mut i = base();
        i.chain_name = "Ethereum".into();
        i.chain_address = Some("   ".into());
        assert_eq!(
            receive_address_message(i),
            "Ethereum receive unavailable. Open Edit Name and add the seed phrase."
        );
    }

    #[test]
    fn evm_returns_derived_address_when_resolved_empty() {
        let mut i = base();
        i.chain_name = "Ethereum".into();
        i.chain_address = Some("0xabc".into());
        assert_eq!(receive_address_message(i), "0xabc");
    }

    #[test]
    fn evm_unresolvable() {
        let mut i = base();
        i.chain_name = "Arbitrum".into();
        let msg = receive_address_message(i);
        assert!(msg.starts_with("Arbitrum receive unavailable"));
    }

    #[test]
    fn simple_chain_tron_unresolvable() {
        let mut i = base();
        i.chain_name = "Tron".into();
        let msg = receive_address_message(i);
        assert!(msg.contains("seed phrase or TRON watch address"));
    }

    #[test]
    fn simple_chain_tron_resolved() {
        let mut i = base();
        i.chain_name = "Tron".into();
        i.chain_address = Some("TXYZ".into());
        assert_eq!(receive_address_message(i), "TXYZ");
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
    fn unknown_chain_disabled() {
        let mut i = base();
        i.chain_name = "Nothing".into();
        assert_eq!(
            receive_address_message(i),
            "Receive is not enabled for this chain."
        );
    }
}

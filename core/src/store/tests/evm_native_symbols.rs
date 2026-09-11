use crate::registry::Chain;

#[test]
fn native_symbols_match_the_hand_written_pairs_they_replaced() {
    for (display_name, symbol) in [
        ("Ethereum", "ETH"),
        ("Ethereum Classic", "ETC"),
        ("Optimism", "ETH"),
        ("BNB Chain", "BNB"),
        ("Avalanche", "AVAX"),
        ("Hyperliquid", "HYPE"),
    ] {
        let chain = Chain::from_display_name(display_name)
            .unwrap_or_else(|| panic!("unknown chain {display_name}"));
        assert_eq!(chain.coin_symbol(), symbol, "{display_name}");
    }
}

#[test]
fn every_evm_chain_has_a_native_symbol() {
    for chain in Chain::all().filter(|c| c.is_evm()) {
        assert!(
            !chain.coin_symbol().is_empty(),
            "{} has no native symbol, so its native asset would be treated as a token",
            chain.str_id()
        );
    }
}

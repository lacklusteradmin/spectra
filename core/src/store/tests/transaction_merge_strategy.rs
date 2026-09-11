use crate::fetch::transactions::TransactionMergeStrategy as S;
use crate::registry::Chain;

fn strategy(display_name: &str) -> S {
    Chain::from_display_name(display_name)
        .unwrap_or_else(|| panic!("unknown chain {display_name}"))
        .transaction_merge_strategy()
}

#[test]
fn matches_the_swift_wrappers_it_replaced() {
    for name in ["Bitcoin", "Bitcoin Cash", "Bitcoin SV", "Litecoin"] {
        assert_eq!(strategy(name), S::StandardUtxo, "{name}");
    }
    assert_eq!(strategy("Dogecoin"), S::Dogecoin);
    for name in [
        "Tron",
        "Solana",
        "Cardano",
        "XRP Ledger",
        "Stellar",
        "Monero",
        "Sui",
        "Aptos",
        "TON",
        "Internet Computer",
        "NEAR",
        "Polkadot",
    ] {
        assert_eq!(strategy(name), S::AccountBased, "{name}");
    }
    for name in ["Ethereum", "Arbitrum", "Base", "Polygon"] {
        assert_eq!(strategy(name), S::Evm, "{name}");
    }
}

#[test]
fn only_tron_keys_its_merge_identity_on_symbol() {
    for chain in Chain::all() {
        let expected = matches!(chain.str_id(), "tron" | "tron-nile");
        assert_eq!(
            chain.merge_identity_includes_symbol(),
            expected,
            "{}",
            chain.str_id()
        );
    }
}

#[test]
fn a_testnet_merges_the_same_way_as_its_mainnet() {
    for chain in Chain::all() {
        let mainnet = chain.mainnet_counterpart();
        if mainnet == chain {
            continue;
        }
        assert_eq!(
            chain.transaction_merge_strategy(),
            mainnet.transaction_merge_strategy(),
            "{} diverges from {}",
            chain.str_id(),
            mainnet.str_id()
        );
    }
}

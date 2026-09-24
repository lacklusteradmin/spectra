use crate::fetch::transactions::TransactionMergeStrategy as S;
use crate::registry::Chain;

fn strategy(chain_id: &str) -> S {
    Chain::from_str_id(chain_id)
        .unwrap_or_else(|| panic!("unknown chain {chain_id}"))
        .transaction_merge_strategy()
}

#[test]
fn history_protocols_select_their_merge_semantics() {
    for name in ["bitcoin", "bitcoin-cash", "bitcoin-sv", "litecoin"] {
        assert_eq!(strategy(name), S::StandardUtxo, "{name}");
    }
    assert_eq!(strategy("dogecoin"), S::Dogecoin);
    for name in [
        "tron",
        "solana",
        "cardano",
        "xrp",
        "stellar",
        "monero",
        "sui",
        "aptos",
        "ton",
        "internet-computer",
        "near",
        "polkadot",
    ] {
        assert_eq!(strategy(name), S::AccountBased, "{name}");
    }
    for name in ["ethereum", "arbitrum", "base", "polygon"] {
        assert_eq!(strategy(name), S::Evm, "{name}");
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

use crate::fetch::transactions::TransactionMergeStrategy as S;
use crate::registry::Chain;

fn strategy(chain_id: crate::registry::Chain) -> S {
    chain_id.transaction_merge_strategy()
}

#[test]
fn history_protocols_select_their_merge_semantics() {
    for name in [
        crate::registry::Chain::Bitcoin,
        crate::registry::Chain::BitcoinCash,
        crate::registry::Chain::BitcoinSV,
        crate::registry::Chain::Litecoin,
    ] {
        assert_eq!(strategy(name), S::StandardUtxo, "{name}");
    }
    assert_eq!(strategy(crate::registry::Chain::Dogecoin), S::Dogecoin);
    for name in [
        crate::registry::Chain::Tron,
        crate::registry::Chain::Solana,
        crate::registry::Chain::Cardano,
        crate::registry::Chain::Xrp,
        crate::registry::Chain::Stellar,
        crate::registry::Chain::Monero,
        crate::registry::Chain::Sui,
        crate::registry::Chain::Aptos,
        crate::registry::Chain::Ton,
        crate::registry::Chain::Icp,
        crate::registry::Chain::Near,
        crate::registry::Chain::Polkadot,
    ] {
        assert_eq!(strategy(name), S::AccountBased, "{name}");
    }
    for name in [
        crate::registry::Chain::Ethereum,
        crate::registry::Chain::Arbitrum,
        crate::registry::Chain::Base,
        crate::registry::Chain::Polygon,
    ] {
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

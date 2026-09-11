use crate::registry::Chain;

/// The non-EVM chains the Swift switch named, minus the four that keep
/// bespoke resolvers (Bitcoin, Dogecoin, Cardano, Monero).
const SWITCH_CHAINS: &[&str] = &[
    "Bitcoin Cash",
    "Bitcoin SV",
    "Litecoin",
    "Tron",
    "Solana",
    "Stellar",
    "XRP Ledger",
    "Sui",
    "Aptos",
    "TON",
    "Internet Computer",
    "NEAR",
    "Polkadot",
    "Zcash",
    "Bitcoin Gold",
    "Decred",
    "Kaspa",
    "Dash",
    "Bittensor",
];

#[test]
fn every_chain_the_switch_named_still_resolves() {
    for name in SWITCH_CHAINS {
        assert!(
            Chain::from_display_name(name).is_some(),
            "{name} is no longer a known chain"
        );
        assert!(
            Chain::from_display_name(name)
                .and_then(crate::send::flow::seed_derivation_chain_raw)
                .is_some(),
            "{name} has no seed derivation chain, so its address would resolve to nil"
        );
    }
}

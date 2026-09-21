use crate::registry::Chain;
use crate::store::wallet_domain::{
    AssetHolding, CoreSeedDerivationPaths, CoreSeedDerivationPreset, CoreWalletDerivationOverrides,
    WalletView,
};
use std::collections::HashMap;

fn bitcoin_wallet() -> WalletView {
    let mut paths = CoreSeedDerivationPaths::default();
    // The full table every wallet carries today, of which one entry applies.
    paths.set_path_for(Chain::Bitcoin, "m/84'/0'/0'/0/0");
    paths.set_path_for(Chain::BitcoinTestnet4, "m/84'/1'/0'/0/0");
    paths.set_path_for(Chain::Ethereum, "m/44'/60'/0'/0/0");
    paths.set_path_for(Chain::Solana, "m/44'/501'/0'");

    WalletView {
        id: "w1".to_string(),
        name: "Cold".to_string(),
        chain_id: "bitcoin-testnet-4".to_string(),
        addresses: HashMap::from([("bitcoin".to_string(), "bc1qexample".to_string())]),
        bitcoin_xpub: Some("zpub123".to_string()),
        seed_derivation_preset: CoreSeedDerivationPreset::Account2,
        seed_derivation_paths: paths,
        derivation_overrides: CoreWalletDerivationOverrides {
            passphrase: Some("secret".to_string()),
            ..Default::default()
        },
        family_name: "Bitcoin".to_string(),
        holdings: vec![AssetHolding {
            name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            coingecko_id: "bitcoin".to_string(),
            chain_name: "Bitcoin".to_string(),
            token_standard: "Native".to_string(),
            contract_address: None,
            amount: 1.5,
            price_usd: 60000.0,
        }],
        include_in_portfolio_total: true,
    }
}

#[test]
fn keeps_the_path_the_wallet_uses_and_drops_the_rest() {
    let summary = bitcoin_wallet().to_wallet_state(false).unwrap();
    assert_eq!(summary.derivation_path.as_deref(), Some("m/84'/1'/0'/0/0"));
    // The Ethereum and Solana entries were global defaults, not this
    // wallet's data, and do not survive into the model core computes with.
    assert_eq!(summary.chain_name, "Bitcoin");
}

#[test]
fn keeps_only_the_network_that_applies_to_this_wallets_family() {
    let summary = bitcoin_wallet().to_wallet_state(false).unwrap();
    assert_eq!(summary.chain_id.as_str(), "bitcoin-testnet-4");

    let mut inconsistent = bitcoin_wallet();
    inconsistent.family_name = "Solana".to_string();
    assert!(inconsistent.to_wallet_state(false).is_err());
}

#[test]
fn carries_overrides_xpub_preset_and_holdings() {
    let summary = bitcoin_wallet().to_wallet_state(false).unwrap();
    assert_eq!(
        summary.derivation_overrides.passphrase.as_deref(),
        Some("secret")
    );
    assert_eq!(summary.xpub.as_deref(), Some("zpub123"));
    assert_eq!(
        summary.derivation_preset,
        crate::store::wallet_domain::CoreSeedDerivationPreset::Account2
    );
    assert_eq!(summary.holdings.len(), 1);
    assert_eq!(summary.holdings[0].amount, 1.5);
    assert_eq!(summary.holdings[0].symbol, "BTC");
}

/// The address becomes a typed entry with its chain and derivation path,
/// rather than a bare string in a slot-keyed map.
#[test]
fn the_address_gains_its_chain_and_path() {
    let summary = bitcoin_wallet().to_wallet_state(false).unwrap();
    assert_eq!(summary.addresses.len(), 1);
    assert_eq!(summary.addresses[0].address, "bc1qexample");
    assert_eq!(summary.addresses[0].chain_name, "Bitcoin");
    assert_eq!(
        summary.addresses[0].derivation_path.as_deref(),
        Some("m/84'/0'/0'/0/0")
    );
    assert_eq!(summary.primary_address(), Some("bc1qexample"));
}

/// Watch-only is a Keychain fact on iOS, not something the record holds,
/// so the caller states it.
#[test]
fn watch_only_comes_from_the_caller() {
    assert!(
        !bitcoin_wallet()
            .to_wallet_state(false)
            .unwrap()
            .is_watch_only
    );
    assert!(
        bitcoin_wallet()
            .to_wallet_state(true)
            .unwrap()
            .is_watch_only
    );
}

#[test]
fn unknown_wallet_networks_and_families_are_refused() {
    let mut wallet = bitcoin_wallet();
    wallet.family_name = "Nonexistent Chain".to_string();
    assert!(wallet.to_wallet_state(false).is_err());
    wallet.family_name = "Bitcoin".to_string();
    wallet.chain_id = "unknown-network".to_string();
    assert!(wallet.to_wallet_state(false).is_err());
}

#[test]
fn network_identity_selects_the_primary_address_in_both_models() {
    let mut wallet = bitcoin_wallet();
    wallet.addresses.insert(
        Chain::BitcoinTestnet4.address_slot().to_string(),
        "tb1qexample".to_string(),
    );
    assert_eq!(wallet.primary_address(), Some("tb1qexample"));
    let state = wallet.to_wallet_state(false).unwrap();
    assert_eq!(state.addresses[0].address, "tb1qexample");
    assert_eq!(state.primary_address(), Some("tb1qexample"));
    wallet.chain_id = Chain::Bitcoin.str_id().to_string();
    assert_eq!(wallet.primary_address(), Some("bc1qexample"));
    assert_eq!(
        wallet.to_wallet_state(false).unwrap().primary_address(),
        Some("bc1qexample")
    );
}

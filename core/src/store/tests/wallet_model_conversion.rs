use crate::registry::Chain;
use crate::store::wallet_domain::{
    AssetHolding, CoreImportedWallet, CoreSeedDerivationPaths, CoreSeedDerivationPreset,
    CoreWalletDerivationOverrides,
};
use std::collections::HashMap;

fn bitcoin_wallet() -> CoreImportedWallet {
    let mut paths = CoreSeedDerivationPaths::default();
    // The full table every wallet carries today, of which one entry applies.
    paths.set_path_for(Chain::Bitcoin, "m/84'/0'/0'/0/0");
    paths.set_path_for(Chain::Ethereum, "m/44'/60'/0'/0/0");
    paths.set_path_for(Chain::Solana, "m/44'/501'/0'");

    CoreImportedWallet {
        id: "w1".to_string(),
        name: "Cold".to_string(),
        network_chain_id: Some("bitcoin-testnet-4".to_string()),
        addresses: HashMap::from([("bitcoin".to_string(), "bc1qexample".to_string())]),
        bitcoin_xpub: Some("zpub123".to_string()),
        seed_derivation_preset: CoreSeedDerivationPreset::Account2,
        seed_derivation_paths: paths,
        derivation_overrides: CoreWalletDerivationOverrides {
            passphrase: Some("secret".to_string()),
            ..Default::default()
        },
        selected_chain: "Bitcoin".to_string(),
        holdings: vec![AssetHolding {
            name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            coin_gecko_id: "bitcoin".to_string(),
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
    let summary = bitcoin_wallet().to_summary(false);
    assert_eq!(summary.derivation_path.as_deref(), Some("m/84'/0'/0'/0/0"));
    // The Ethereum and Solana entries were global defaults, not this
    // wallet's data, and do not survive into the model core computes with.
    assert_eq!(summary.chain_name, "Bitcoin");
}

#[test]
fn keeps_only_the_network_that_applies_to_this_wallets_family() {
    let summary = bitcoin_wallet().to_summary(false);
    assert_eq!(summary.network_mode.as_deref(), Some("bitcoin-testnet-4"));

    // A wallet on a chain with no network variants reports none, rather
    // than the meaningless mainnet default the record always holds.
    let mut solana = bitcoin_wallet();
    solana.selected_chain = "Solana".to_string();
    assert_eq!(solana.to_summary(false).network_mode, None);
}

#[test]
fn carries_overrides_xpub_preset_and_holdings() {
    let summary = bitcoin_wallet().to_summary(false);
    assert_eq!(
        summary.derivation_overrides.passphrase.as_deref(),
        Some("secret")
    );
    assert_eq!(summary.xpub.as_deref(), Some("zpub123"));
    assert_eq!(summary.derivation_preset, "account2");
    assert_eq!(summary.holdings.len(), 1);
    assert_eq!(summary.holdings[0].amount, 1.5);
    assert_eq!(summary.holdings[0].symbol, "BTC");
}

/// The address becomes a typed entry with its chain and derivation path,
/// rather than a bare string in a slot-keyed map.
#[test]
fn the_address_gains_its_chain_and_path() {
    let summary = bitcoin_wallet().to_summary(false);
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
    assert!(!bitcoin_wallet().to_summary(false).is_watch_only);
    assert!(bitcoin_wallet().to_summary(true).is_watch_only);
}

/// A wallet whose chain the registry does not know converts without
/// inventing anything.
///
/// It used to convert with no addresses at all, because the conversion
/// emitted one entry — the selected chain's. It emits every slot the record
/// holds now, since a wallet carries one address per network of its family,
/// and each entry names the chain whose slot it came from. An unknown chain
/// owns no slot, so it contributes neither an address nor a path.
#[test]
fn an_unknown_chain_yields_no_address_of_its_own() {
    let mut wallet = bitcoin_wallet();
    wallet.selected_chain = "Nonexistent Chain".to_string();
    let summary = wallet.to_summary(false);
    assert_eq!(summary.derivation_path, None);
    assert!(summary
        .addresses
        .iter()
        .all(|entry| entry.chain_name != "Nonexistent Chain"));
    assert_eq!(summary.addresses.len(), 1);
    assert_eq!(summary.addresses[0].chain_name, "Bitcoin");
}

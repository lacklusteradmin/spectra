use crate::registry::Chain;
use crate::store::state::{WalletAddress, WalletSummary};
use crate::store::wallet_domain::AssetHolding;
use crate::store::wallet_domain::CoreSeedDerivationPaths;

fn defaults() -> CoreSeedDerivationPaths {
    crate::app_core_derivation_paths_for_preset(0).expect("defaults")
}

fn summary() -> WalletSummary {
    WalletSummary {
        id: "w1".to_string(),
        name: "Cold".to_string(),
        is_watch_only: false,
        chain_name: "Bitcoin".to_string(),
        include_in_portfolio_total: true,
        network_mode: Some("bitcoin-testnet-4".to_string()),
        xpub: Some("zpub123".to_string()),
        derivation_preset: "account2".to_string(),
        derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
        derivation_overrides: Default::default(),
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
        addresses: vec![WalletAddress {
            chain_name: "Bitcoin".to_string(),
            address: "bc1qexample".to_string(),
            kind: "receive".to_string(),
            derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
        }],
    }
}

/// Everything the app renders survives the trip out to the view model.
#[test]
fn the_view_model_carries_what_the_app_shows() {
    let view = summary().to_imported_wallet(&defaults());
    assert_eq!(view.id, "w1");
    assert_eq!(view.selected_chain, "Bitcoin");
    assert_eq!(view.bitcoin_xpub.as_deref(), Some("zpub123"));
    assert_eq!(view.address_for(Chain::Bitcoin), Some("bc1qexample"));
    assert_eq!(view.holdings.len(), 1);
    assert_eq!(view.holdings[0].amount, 1.5);
    // The wallet's own path overrides the default for its chain.
    assert_eq!(
        view.seed_derivation_paths.path_for(Chain::Bitcoin),
        Some("m/84'/0'/2'/0/0")
    );
    // Other chains keep the catalog defaults, which is all they ever were.
    assert!(view.seed_derivation_paths.path_for(Chain::Solana).is_some());
}

/// The network mode goes back into the field for the wallet's own chain and
/// The wallet's network is one chain id, so there is nothing to leave
/// alone — a wallet on Bitcoin testnet4 says exactly that, rather than
/// carrying a Bitcoin mode and a Dogecoin mode and a rule for reading them.
#[test]
fn the_wallets_own_network_survives_the_round_trip() {
    let view = summary().to_imported_wallet(&defaults());
    assert_eq!(view.network_chain_id.as_deref(), Some("bitcoin-testnet-4"));
}

/// Holding ids are derived from what identifies the asset, so rebuilding
/// the view model does not make SwiftUI think every row is new.
#[test]
fn holding_ids_are_stable_across_rebuilds() {
    use crate::store::wallet_domain::holding_identity;
    let first = summary().to_imported_wallet(&defaults());
    let second = summary().to_imported_wallet(&defaults());
    assert_eq!(
        holding_identity(&first.holdings[0]),
        holding_identity(&second.holdings[0])
    );
    assert!(!holding_identity(&first.holdings[0]).is_empty());

    // Two different assets do not collide.
    let mut other = summary();
    other.holdings[0].symbol = "USDT".to_string();
    other.holdings[0].contract_address = Some("0xdac1".to_string());
    let third = other.to_imported_wallet(&defaults());
    assert_ne!(
        holding_identity(&first.holdings[0]),
        holding_identity(&third.holdings[0])
    );
}

/// Round trip through both conversions preserves everything the summary
/// holds — the authority is unchanged by being rendered.
#[test]
fn summary_survives_a_round_trip_through_the_view_model() {
    let original = summary();
    let round_tripped = original
        .to_imported_wallet(&defaults())
        .to_summary(original.is_watch_only);
    assert_eq!(round_tripped, original);
}

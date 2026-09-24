use crate::registry::Chain;
use crate::store::state::{WalletAddress, WalletState};
use crate::store::wallet_domain::AssetHolding;
use crate::store::wallet_domain::CoreSeedDerivationPaths;

fn defaults() -> CoreSeedDerivationPaths {
    crate::derivation_paths_for_preset(Default::default()).expect("defaults")
}

fn summary() -> WalletState {
    WalletState {
        id: "w1".to_string(),
        name: "Cold".to_string(),
        signing: crate::store::state::WalletSigning::SeedPhrase {
            password_protected: false,
        },
        include_in_portfolio_total: true,
        chain_id: "bitcoin-testnet-4".to_string(),
        xpub: Some("zpub123".to_string()),
        derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Account2,
        derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
        derivation_overrides: Default::default(),
        holdings: vec![AssetHolding {
            id: "bitcoin:native".into(),
            name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            coingecko_id: "bitcoin".to_string(),
            chain_id: "bitcoin".to_string(),
            token_standard: "Native".to_string(),
            contract_address: None,
            amount: "1.5".into(),
        }],
        addresses: vec![WalletAddress {
            chain_id: "bitcoin".to_string(),
            address: "bc1qexample".to_string(),
            kind: "receive".to_string(),
            derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
        }],
    }
}

/// Everything the app renders survives the trip out to the view model.
#[test]
fn the_view_model_carries_what_the_app_shows() {
    let view = summary().to_wallet_view(&defaults());
    assert_eq!(view.id, "w1");
    assert_eq!(view.chain_id, "bitcoin-testnet-4");
    assert_eq!(view.bitcoin_xpub.as_deref(), Some("zpub123"));
    assert_eq!(view.address_for(Chain::Bitcoin), Some("bc1qexample"));
    assert_eq!(view.holdings.len(), 1);
    assert_eq!(view.holdings[0].amount, "1.5");
    // The wallet's own path overrides the default for its chain.
    assert_eq!(
        view.seed_derivation_paths.path_for(Chain::Bitcoin),
        Some("m/84'/0'/2'/0/0")
    );
    // Other chains keep the catalog defaults, which is all they ever were.
    assert!(view.seed_derivation_paths.path_for(Chain::Solana).is_some());
}

/// Round trip through both conversions preserves everything the summary
/// holds — the authority is unchanged by being rendered.
#[test]
fn summary_survives_a_round_trip_through_the_view_model() {
    let original = summary();
    let round_tripped = original
        .to_wallet_view(&defaults())
        .to_wallet_state()
        .unwrap();
    assert_eq!(round_tripped, original);
}

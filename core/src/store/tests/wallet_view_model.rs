use crate::registry::Chain;
use crate::store::state::{WalletAddress, WalletState};
use crate::store::wallet_domain::AssetHolding;

fn summary() -> WalletState {
    WalletState {
        id: "w1".to_string(),
        name: "Cold".to_string(),
        signing: crate::store::state::WalletSigning::SeedPhrase {
            password_protected: false,
        },
        include_in_portfolio_total: true,
        chain_id: crate::registry::Chain::Bitcoin,
        xpub: Some("zpub123".to_string()),
        derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
        derivation_overrides: Default::default(),
        holdings: vec![AssetHolding {
            id: "bitcoin:native".into(),
            name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            coingecko_id: "bitcoin".to_string(),
            chain_id: crate::registry::Chain::Bitcoin,
            token_standard: "Native".to_string(),
            contract_address: None,
            amount: "1.5".into(),
        }],
        addresses: vec![WalletAddress {
            chain_id: crate::registry::Chain::Bitcoin,
            address: "bc1qexample".to_string(),
            kind: "receive".to_string(),
            derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
        }],
        restore_height: None,
    }
}

/// Everything the app renders survives the trip out to the view model.
#[test]
fn the_view_model_carries_what_the_app_shows() {
    let view = summary().to_wallet_view();
    assert_eq!(view.id, "w1");
    assert_eq!(view.chain_id, crate::registry::Chain::Bitcoin);
    assert_eq!(view.account_xpub.as_deref(), Some("zpub123"));
    assert_eq!(view.address_for(Chain::Bitcoin), Some("bc1qexample"));
    assert_eq!(view.holdings.len(), 1);
    assert_eq!(view.holdings[0].amount, "1.5");
    assert_eq!(view.derivation_path.as_deref(), Some("m/84'/0'/2'/0/0"));
}

/// Round trip through both conversions preserves everything the summary
/// holds — the authority is unchanged by being rendered.
#[test]
fn summary_survives_a_round_trip_through_the_view_model() {
    let original = summary();
    let round_tripped = original.to_wallet_view().to_wallet_state().unwrap();
    assert_eq!(round_tripped, original);
}

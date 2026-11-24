use crate::store::{state::WalletState, wallet_domain::AssetHolding};
use crate::{registry::Chain, service::WalletService};

fn native(chain: Chain) -> AssetHolding {
    AssetHolding {
        id: String::new(),
        name: chain.coin_name().into(),
        symbol: chain.coin_symbol().into(),
        chain_id: chain,
        token_standard: None,
        coingecko_id: chain.coingecko_id().into(),
        contract_address: None,
        amount: "0".into(),
    }
}

#[test]
fn symbols_and_market_ids_never_identify_a_holding() {
    let eth = native(Chain::Ethereum);
    let base = native(Chain::Base);
    let mut lookalike = eth.clone();
    lookalike.token_standard = Some(crate::registry::TokenStandard::Erc20);
    lookalike.contract_address = Some("0x1111111111111111111111111111111111111111".into());
    assert!(!lookalike.is_native());
    assert_ne!(lookalike.deployment_id(), eth.deployment_id());
    assert_ne!(lookalike.token_identity(), eth.token_identity());
    assert_eq!(eth.token_identity(), base.token_identity());
    assert_ne!(eth.deployment_id(), base.deployment_id());
    lookalike.canonicalize().unwrap();
    assert!(lookalike.coingecko_id.is_empty());
}

#[test]
fn every_testnet_has_an_independent_unpriced_native_token() {
    for network in Chain::all().filter(|c| c.is_testnet()) {
        let mut holding = native(network);
        holding.coingecko_id = network.mainnet_counterpart().coingecko_id().into();
        holding.canonicalize().unwrap();
        assert!(holding.coingecko_id.is_empty());
        assert_ne!(
            holding.token_identity(),
            native(network.mainnet_counterpart()).token_identity()
        );
    }
}

#[tokio::test]
async fn invalid_protocol_identity_is_refused_before_storage() {
    let service = WalletService::new(vec![]).unwrap();
    let mut wallet = WalletState::single_address(
        "w",
        "W",
        crate::registry::Chain::Ethereum,
        "0x1111111111111111111111111111111111111111",
        None,
        true,
    );
    let mut holding = native(Chain::Ethereum);
    holding.token_standard = Some(crate::registry::TokenStandard::Erc20);
    wallet.holdings.push(holding);
    assert!(service.upsert_wallet(wallet.clone()).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
    wallet.holdings[0].contract_address = Some("not-an-address".into());
    assert!(service.upsert_wallet(wallet).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
}

#[test]
fn display_precision_is_deployment_specific() {
    use crate::catalog::tokens::token_display_decimals;
    assert_eq!(
        token_display_decimals(Some("ethereum:native".into()), Some(6)),
        18
    );
    let usdc = crate::catalog::tokens::catalog()
        .iter()
        .find(|t| t.chain_id == crate::registry::Chain::Ethereum && t.token_id == "usd-coin")
        .unwrap();
    assert_eq!(
        token_display_decimals(Some(usdc.deployment_id.clone()), None),
        6
    );
    assert_eq!(
        token_display_decimals(Some("custom:unlisted".into()), Some(2)),
        2
    );
    assert_eq!(token_display_decimals(None, None), 18);
}

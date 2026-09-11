use crate::service::WalletService;
use crate::state::StateCommand;
use crate::store::wallet_domain::AssetHolding;

fn coin(symbol: &str, chain: &str, amount: f64) -> AssetHolding {
    AssetHolding {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        coin_gecko_id: symbol.to_lowercase(),
        chain_name: chain.to_string(),
        token_standard: String::new(),
        contract_address: None,
        amount,
        price_usd: 1.0,
    }
}

async fn service_with(
    wallets: Vec<(&str, &str, Vec<AssetHolding>, bool)>,
) -> std::sync::Arc<WalletService> {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    for (id, chain, holdings, included) in wallets {
        let mut summary =
            crate::store::state::WalletSummary::single_address(id, id, chain, "addr", None, false);
        summary.include_in_portfolio_total = included;
        summary.holdings = holdings
            .into_iter()
            .map(|c| crate::store::wallet_domain::AssetHolding {
                name: c.name,
                symbol: c.symbol,
                coin_gecko_id: c.coin_gecko_id,
                chain_name: c.chain_name,
                token_standard: c.token_standard,
                contract_address: c.contract_address,
                amount: c.amount,
                price_usd: c.price_usd,
            })
            .collect();
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet: summary })
            .await
            .expect("upsert");
    }
    service
}

#[tokio::test]
async fn portfolio_sums_the_same_asset_across_wallets() {
    let service = service_with(vec![
        ("w1", "Bitcoin", vec![coin("BTC", "Bitcoin", 1.5)], true),
        ("w2", "Bitcoin", vec![coin("BTC", "Bitcoin", 0.5)], true),
    ])
    .await;
    let derived = service
        .wallet_derived_state(vec![], vec![])
        .await
        .expect("derived");
    assert_eq!(derived.portfolio.len(), 1);
    assert_eq!(derived.portfolio[0].amount, 2.0);
}

#[tokio::test]
async fn wallets_excluded_from_the_total_contribute_nothing() {
    let service = service_with(vec![
        ("w1", "Bitcoin", vec![coin("BTC", "Bitcoin", 1.0)], true),
        ("w2", "Bitcoin", vec![coin("BTC", "Bitcoin", 9.0)], false),
    ])
    .await;
    let derived = service
        .wallet_derived_state(vec![], vec![])
        .await
        .expect("derived");
    assert_eq!(derived.portfolio[0].amount, 1.0);
    assert_eq!(derived.included_portfolio_holdings.len(), 1);
}

/// Every family's testnet is unpriced, not just the two the old rule
/// listed by name — Dogecoin testnet used to be quoted at mainnet prices.
#[tokio::test]
async fn no_testnet_coin_is_quoted_on_any_family() {
    for (chain, testnet_id) in [
        ("Bitcoin", "bitcoin-testnet"),
        ("Ethereum", "ethereum-sepolia"),
        ("Dogecoin", "dogecoin-testnet"),
    ] {
        let service = service_with(vec![("w1", chain, vec![coin("X", chain, 1.0)], true)]).await;
        service
            .apply_state_command(StateCommand::SelectNetworkChain {
                chain_id: testnet_id.into(),
            })
            .await
            .expect("select");
        let derived = service
            .wallet_derived_state(vec![], vec![])
            .await
            .expect("derived");
        assert!(
            derived.unique_price_request_coins.is_empty(),
            "{chain} testnet coins have no price to request"
        );
    }
}

/// Selecting the mainnet clears the entry rather than storing it, so the
/// two ways of saying "mainnet" cannot drift apart.
#[tokio::test]
async fn choosing_mainnet_stores_nothing() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let after_testnet = service
        .apply_state_command(StateCommand::SelectNetworkChain {
            chain_id: "bitcoin-testnet-4".into(),
        })
        .await
        .expect("select");
    assert_eq!(
        after_testnet.state.settings.network_chain_by_family.len(),
        1
    );

    let after_mainnet = service
        .apply_state_command(StateCommand::SelectNetworkChain {
            chain_id: "bitcoin".into(),
        })
        .await
        .expect("select");
    assert!(after_mainnet
        .state
        .settings
        .network_chain_by_family
        .is_empty());
}

#[tokio::test]
async fn sending_needs_signing_material_on_a_live_chain() {
    let service = service_with(vec![(
        "w1",
        "Bitcoin",
        vec![coin("BTC", "Bitcoin", 1.0)],
        true,
    )])
    .await;

    let watch_only = service
        .wallet_derived_state(vec![], vec![])
        .await
        .expect("derived");
    assert!(watch_only.send_enabled_wallet_ids.is_empty());
    // Receiving never needs a key.
    assert_eq!(
        watch_only.receive_enabled_wallet_ids,
        vec!["w1".to_string()]
    );

    let with_key = service
        .wallet_derived_state(vec!["w1".into()], vec![])
        .await
        .expect("derived");
    assert_eq!(with_key.send_enabled_wallet_ids, vec!["w1".to_string()]);
}

#[tokio::test]
async fn an_untracked_token_on_ethereum_cannot_be_sent() {
    let service = service_with(vec![(
        "w1",
        "Ethereum",
        vec![coin("ETH", "Ethereum", 1.0), coin("SHIB", "Ethereum", 1.0)],
        true,
    )])
    .await;
    let derived = service
        .wallet_derived_state(vec!["w1".into()], vec![])
        .await
        .expect("derived");
    let sendable: Vec<&str> = derived.send_coins_by_wallet_id["w1"]
        .iter()
        .map(|c| c.symbol.as_str())
        .collect();
    assert_eq!(sendable, vec!["ETH"], "SHIB is not a known token");
}

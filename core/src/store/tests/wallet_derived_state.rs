use crate::service::WalletService;
use crate::store::state::StateCommand;
use crate::store::wallet_domain::AssetHolding;

fn coin(symbol: &str, chain: &str, amount: f64) -> AssetHolding {
    AssetHolding {
        id: String::new(),
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        coingecko_id: symbol.to_lowercase(),
        chain_id: chain.to_string(),
        token_standard: if crate::registry::Chain::from_str_id(chain)
            .is_some_and(|c| c.coin_symbol() == symbol)
        {
            "Native".into()
        } else {
            "ERC-20".into()
        },
        contract_address: if symbol == "SHIB" {
            Some("0x95ad61b0a150d79219dcf64e1e6cc01f0b64c4ce".into())
        } else {
            None
        },
        amount: crate::decimal::from_f64(amount).unwrap(),
    }
}

async fn service_with(
    wallets: Vec<(&str, &str, Vec<AssetHolding>, bool)>,
) -> std::sync::Arc<WalletService> {
    let service = WalletService::new(Vec::new()).expect("service");
    for (id, chain, holdings, included) in wallets {
        let mut summary =
            crate::store::state::WalletState::single_address(id, id, chain, "addr", None, false);
        summary.include_in_portfolio_total = included;
        summary.holdings = holdings
            .into_iter()
            .map(|c| crate::store::wallet_domain::AssetHolding {
                id: String::new(),
                name: c.name,
                symbol: c.symbol,
                coingecko_id: c.coingecko_id,
                chain_id: c.chain_id,
                token_standard: c.token_standard,
                contract_address: c.contract_address,
                amount: c.amount,
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
        ("w1", "bitcoin", vec![coin("BTC", "bitcoin", 1.5)], true),
        ("w2", "bitcoin", vec![coin("BTC", "bitcoin", 0.5)], true),
    ])
    .await;
    let derived = service.wallet_derived_state().await.expect("derived");
    assert_eq!(derived.portfolio.len(), 1);
    assert_eq!(derived.portfolio[0].amount, "2");
}

#[tokio::test]
async fn wallets_excluded_from_the_total_contribute_nothing() {
    let service = service_with(vec![
        ("w1", "bitcoin", vec![coin("BTC", "bitcoin", 1.0)], true),
        ("w2", "bitcoin", vec![coin("BTC", "bitcoin", 9.0)], false),
    ])
    .await;
    let derived = service.wallet_derived_state().await.expect("derived");
    assert_eq!(derived.portfolio[0].amount, "1");
    assert_eq!(derived.included_portfolio_holdings.len(), 1);
}

/// Every family's testnet is unpriced, not just the two the old rule
/// listed by name — Dogecoin testnet used to be quoted at mainnet prices.
#[tokio::test]
async fn no_testnet_coin_is_quoted_on_any_family() {
    for (chain, testnet_id) in [
        ("bitcoin", "bitcoin-testnet"),
        ("ethereum", "ethereum-sepolia"),
        ("dogecoin", "dogecoin-testnet"),
    ] {
        let network = crate::registry::Chain::from_str_id(testnet_id).unwrap();
        let service = service_with(vec![(
            "w1",
            chain,
            vec![coin(network.coin_symbol(), network.str_id(), 1.0)],
            true,
        )])
        .await;
        service
            .apply_state_command(StateCommand::SelectChainForFamily {
                chain_id: testnet_id.into(),
            })
            .await
            .expect("select");
        let derived = service.wallet_derived_state().await.expect("derived");
        assert!(
            derived.unique_price_request_coins.is_empty(),
            "{chain} testnet coins have no price to request"
        );
    }
}

/// Selecting the mainnet clears the entry rather than storing it, so the
/// two ways of saying "mainnet" cannot drift apart.
#[tokio::test]
async fn choosing_mainnet_stores_its_explicit_id() {
    let service = WalletService::new(Vec::new()).expect("service");
    let after_testnet = service
        .apply_state_command(StateCommand::SelectChainForFamily {
            chain_id: "bitcoin-testnet-4".into(),
        })
        .await
        .expect("select");
    assert_eq!(
        after_testnet.state.settings.selected_chain_by_family.len(),
        1
    );

    let after_mainnet = service
        .apply_state_command(StateCommand::SelectChainForFamily {
            chain_id: "bitcoin".into(),
        })
        .await
        .expect("select");
    assert_eq!(
        after_mainnet
            .state
            .settings
            .selected_chain_by_family
            .get("bitcoin")
            .map(String::as_str),
        Some("bitcoin")
    );
}

#[tokio::test]
async fn sending_needs_signing_material_on_a_live_chain() {
    let service = service_with(vec![(
        "w1",
        "bitcoin",
        vec![coin("BTC", "bitcoin", 1.0)],
        true,
    )])
    .await;

    let with_key = service.wallet_derived_state().await.expect("derived");
    assert_eq!(with_key.send_enabled_wallet_ids, vec!["w1".to_string()]);

    let mut wallet = service.app_state().await.wallets[0].clone();
    wallet.signing = crate::store::state::WalletSigning::WatchOnly;
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .expect("upsert");
    let watch_only = service.wallet_derived_state().await.expect("derived");
    assert!(watch_only.send_enabled_wallet_ids.is_empty());
    // Receiving never needs a key.
    assert_eq!(
        watch_only.receive_enabled_wallet_ids,
        vec!["w1".to_string()]
    );
}

#[tokio::test]
async fn an_untracked_token_on_ethereum_cannot_be_sent() {
    let service = service_with(vec![(
        "w1",
        "ethereum",
        vec![coin("ETH", "ethereum", 1.0), coin("SHIB", "ethereum", 1.0)],
        true,
    )])
    .await;
    let derived = service.wallet_derived_state().await.expect("derived");
    let sendable: Vec<&str> = derived.send_coins_by_wallet_id["w1"]
        .iter()
        .map(|c| c.symbol.as_str())
        .collect();
    assert_eq!(sendable, vec!["ETH"], "SHIB is not a known token");
}

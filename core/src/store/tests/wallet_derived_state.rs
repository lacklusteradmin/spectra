use crate::service::WalletService;
use crate::state::StateCommand;
use crate::store::wallet_domain::AssetHolding;

fn coin(symbol: &str, chain: &str, amount: f64) -> AssetHolding {
    AssetHolding {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        coin_gecko_id: symbol.to_lowercase(),
        chain_name: chain.to_string(),
        token_standard: if crate::registry::Chain::from_display_name(chain)
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
    let derived = service.wallet_derived_state().await.expect("derived");
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
    let derived = service.wallet_derived_state().await.expect("derived");
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
        let network = crate::registry::Chain::from_str_id(testnet_id).unwrap();
        let service = service_with(vec![(
            "w1",
            chain,
            vec![coin(
                network.coin_symbol(),
                network.chain_display_name(),
                1.0,
            )],
            true,
        )])
        .await;
        service
            .apply_state_command(StateCommand::SelectNetworkChain {
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
    assert_eq!(
        after_mainnet
            .state
            .settings
            .network_chain_by_family
            .get("bitcoin")
            .map(String::as_str),
        Some("bitcoin")
    );
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

    let watch_only = service.wallet_derived_state().await.expect("derived");
    assert!(watch_only.send_enabled_wallet_ids.is_empty());
    // Receiving never needs a key.
    assert_eq!(
        watch_only.receive_enabled_wallet_ids,
        vec!["w1".to_string()]
    );

    install_key(&service);
    let with_key = service.wallet_derived_state().await.expect("derived");
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
    install_key(&service);
    let derived = service.wallet_derived_state().await.expect("derived");
    let sendable: Vec<&str> = derived.send_coins_by_wallet_id["w1"]
        .iter()
        .map(|c| c.symbol.as_str())
        .collect();
    assert_eq!(sendable, vec!["ETH"], "SHIB is not a known token");
}

fn install_key(service: &WalletService) {
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service.store_wallet_seed_phrase("w1".into(), "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(), None).unwrap();
}

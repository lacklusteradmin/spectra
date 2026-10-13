use crate::service::WalletService;
use crate::store::state::{StateCommand, WalletState};
use crate::store::wallet_domain::AssetHolding;

fn holding(symbol: &str, chain: crate::registry::Chain, amount: f64) -> AssetHolding {
    AssetHolding {
        id: String::new(),
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        coingecko_id: symbol.to_lowercase(),
        chain_id: chain,
        token_standard: "Native".to_string(),
        contract_address: None,
        amount: crate::decimal::from_f64(amount).unwrap(),
    }
}

async fn service_with(
    wallets: Vec<(&str, crate::registry::Chain, Vec<AssetHolding>)>,
) -> std::sync::Arc<WalletService> {
    let service = WalletService::new(Vec::new()).expect("service");
    for (id, chain, holdings) in wallets {
        let mut wallet = WalletState::single_address(id, id, chain, "addr", None, false);
        wallet.holdings = holdings;
        // The holdings given are balances, read.
        wallet.balances_read_at = Some(1.0);
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .expect("upsert");
    }
    service
}

/// A row is per asset: the same asset on two chains is one row, with the
/// amounts summed and a breakdown of where it is held.
///
/// Every EVM L2 carries Ethereum's coingecko id, which is what makes them one
/// asset.
#[tokio::test]
async fn a_row_is_per_asset_and_breaks_down_by_chain() {
    let service = service_with(vec![
        (
            "w1",
            crate::registry::Chain::Ethereum,
            vec![holding("ETH", crate::registry::Chain::Ethereum, 1.0)],
        ),
        (
            "w2",
            crate::registry::Chain::Ethereum,
            vec![holding("ETH", crate::registry::Chain::Ethereum, 2.0)],
        ),
        (
            "w3",
            crate::registry::Chain::Arbitrum,
            vec![holding("ETH", crate::registry::Chain::Arbitrum, 5.0)],
        ),
    ])
    .await;
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let eth: Vec<_> = groups
        .iter()
        .filter(|g| g.holdings.iter().any(|h| h.coin.symbol == "ETH"))
        .collect();
    assert_eq!(eth.len(), 1, "one row for the asset, not one per chain");

    let row = eth[0];
    let total = row.holdings.iter().fold("0".to_string(), |sum, h| {
        crate::decimal::add(&sum, &h.coin.amount).unwrap()
    });
    assert_eq!(total, "8", "both chains and both wallets summed");
    assert_eq!(row.holdings.len(), 2, "one breakdown entry per chain");

    // The row is presented as the place most of it is.
    assert_eq!(
        row.holdings[0].coin.chain_id,
        crate::registry::Chain::Arbitrum
    );
    assert_eq!(row.holdings[0].coin.amount, "5");
    // And the two wallets on one chain are one entry.
    let ethereum = row
        .holdings
        .iter()
        .find(|h| h.coin.chain_id == crate::registry::Chain::Ethereum)
        .expect("an Ethereum entry");
    assert_eq!(ethereum.coin.amount, "3");
}

/// A holding with no coingecko id is never merged with another by symbol.
///
/// Symbols are not unique and nobody vouches for them. A token the catalog
/// does not vouch for is reported with an empty symbol on purpose — the
/// front end shows its contract, the one string a deployer cannot forge —
/// and grouping by symbol would undo that, showing a real holding and a
/// lookalike on another chain as one balance.
#[tokio::test]
async fn an_unvouched_token_is_never_merged_by_symbol() {
    let mut real = holding("USDX", crate::registry::Chain::Ethereum, 1.0);
    real.token_standard = "ERC-20".into();
    real.contract_address = Some("0x000000000000000000000000000000000000aaaa".into());
    real.coingecko_id = String::new();
    let mut lookalike = holding("USDX", crate::registry::Chain::Tron, 999.0);
    lookalike.token_standard = "TRC-20".into();
    lookalike.contract_address = Some("T9yD14Nj9j7xAB4dbGeiX9h8unkKHxuWwb".into());
    lookalike.coingecko_id = String::new();
    // And a second contract on the same chain, same symbol.
    let mut sibling = holding("USDX", crate::registry::Chain::Ethereum, 2.0);
    sibling.token_standard = "ERC-20".into();
    sibling.contract_address = Some("0x000000000000000000000000000000000000bbbb".into());
    sibling.coingecko_id = String::new();

    let service = service_with(vec![
        ("w1", crate::registry::Chain::Ethereum, vec![real, sibling]),
        ("w2", crate::registry::Chain::Tron, vec![lookalike]),
    ])
    .await;
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let usdx: Vec<_> = groups
        .iter()
        .filter(|g| g.holdings.iter().any(|h| h.coin.symbol == "USDX"))
        .collect();
    assert_eq!(
        usdx.len(),
        3,
        "three unvouched contracts merged by symbol, so a lookalike's \
             balance was added to a real one"
    );
    for g in usdx {
        assert_eq!(g.holdings.len(), 1);
    }
}

/// A row says who holds it: one entry per wallet, summed across the networks
/// that wallet holds it on, largest value first. A wallet left out of the
/// portfolio is left out here too.
#[tokio::test]
async fn a_row_breaks_down_by_wallet() {
    let service = service_with(vec![
        (
            "small",
            crate::registry::Chain::Ethereum,
            vec![holding("ETH", crate::registry::Chain::Ethereum, 1.0)],
        ),
        (
            "large",
            crate::registry::Chain::Ethereum,
            vec![
                holding("ETH", crate::registry::Chain::Ethereum, 2.0),
                holding("ETH", crate::registry::Chain::Arbitrum, 0.5),
            ],
        ),
        (
            "excluded",
            crate::registry::Chain::Ethereum,
            vec![holding("ETH", crate::registry::Chain::Ethereum, 9.0)],
        ),
    ])
    .await;
    {
        let mut state = service.wallet_state.write().await;
        state.quotes.prices.insert("ethereum:native".into(), 1000.0);
        state.quotes.prices.insert("arbitrum:native".into(), 1000.0);
    }
    service
        .apply_state_command(StateCommand::SetWalletPortfolioInclusion {
            wallet_id: "excluded".into(),
            included: false,
        })
        .await
        .expect("exclude");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let eth = groups.iter().find(|g| g.id == "ethereum").expect("a row");
    let wallets: Vec<_> = eth
        .wallets
        .iter()
        .map(|w| (w.wallet_id.as_str(), w.amount.as_str(), w.value))
        .collect();
    assert_eq!(
        wallets,
        [("large", "2.5", Some(2500.0)), ("small", "1", Some(1000.0))]
    );
}

/// A row names itself from its identity, the place most of it is held.
fn row_symbol(g: &crate::store::wallet_domain::DashboardAssetGroup) -> &str {
    g.identity.symbol.as_str()
}

/// A row's value: the sum of its holdings', or none when any is unpriced.
///
/// The values are in the display currency, which is USD in these tests.
fn row_value(g: &crate::store::wallet_domain::DashboardAssetGroup) -> Option<f64> {
    g.holdings
        .iter()
        .map(|h| h.value)
        .try_fold(0.0, |sum, v| v.map(|v| sum + v))
}

/// Only quotes are prices; an old value embedded in a holding is not a fallback.
#[tokio::test]
async fn an_unquoted_holding_stays_unpriced_until_a_quote_arrives() {
    let service = service_with(vec![(
        "w1",
        crate::registry::Chain::Ethereum,
        vec![holding("ETH", crate::registry::Chain::Ethereum, 2.0)],
    )])
    .await;
    let stored = service.portfolio_snapshot().await.expect("snapshot").groups;
    assert_eq!(
        row_value(stored.iter().find(|g| g.id == "ethereum").unwrap()),
        None
    );

    service
        .wallet_state
        .write()
        .await
        .quotes
        .prices
        .insert("ethereum:native".into(), 3000.0);
    let live = service.portfolio_snapshot().await.expect("snapshot").groups;
    assert_eq!(
        row_value(live.iter().find(|g| g.id == "ethereum").unwrap()),
        Some(6000.0)
    );
}

/// A testnet holding has no value, so its row reports none rather than
/// quoting it at mainnet.
#[tokio::test]
async fn a_testnet_row_has_no_value() {
    let service = service_with(vec![(
        "w1",
        crate::registry::Chain::EthereumSepolia,
        vec![holding("ETH", crate::registry::Chain::EthereumSepolia, 2.0)],
    )])
    .await;
    service
        .wallet_state
        .write()
        .await
        .quotes
        .prices
        .insert("ethereum-sepolia:native".into(), 3000.0);
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    assert_eq!(
        row_value(groups.iter().find(|g| g.id == "ethereum-sepolia").unwrap()),
        None
    );
}

/// A pinned asset the user holds nowhere has no row, even with a wallet that
/// has read its network; holding it again brings the row back, still pinned.
#[tokio::test]
async fn a_pinned_asset_held_nowhere_has_no_row() {
    let service = service_with(vec![
        (
            "w1",
            crate::registry::Chain::Ethereum,
            vec![holding("ETH", crate::registry::Chain::Ethereum, 1.0)],
        ),
        (
            "w2",
            crate::registry::Chain::Solana,
            vec![holding("SOL", crate::registry::Chain::Solana, 0.0)],
        ),
    ])
    .await;
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["solana".into(), "ethereum".into(), "bitcoin".into()],
        })
        .await
        .expect("pin");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let ids: Vec<_> = groups.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids, ["ethereum"], "only the pinned asset that is held");
    assert!(groups.iter().all(|g| !g.holdings.is_empty()));

    let mut held = WalletState::single_address(
        "w2",
        "w2",
        crate::registry::Chain::Solana,
        "addr",
        None,
        false,
    );
    held.holdings = vec![holding("SOL", crate::registry::Chain::Solana, 2.0)];
    held.balances_read_at = Some(2.0);
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet: held })
        .await
        .expect("upsert");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let solana = groups.iter().find(|g| g.id == "solana").expect("a row");
    assert!(solana.is_pinned);
    assert_eq!(solana.identity, solana.holdings[0].coin);
}

/// Pinned rows come first, in the order they were pinned, ahead of unpinned
/// rows worth more.
#[tokio::test]
async fn pinned_rows_lead_in_pin_order() {
    let service = service_with(vec![(
        "w1",
        crate::registry::Chain::Ethereum,
        vec![
            holding("ETH", crate::registry::Chain::Ethereum, 1.0),
            holding("BTC", crate::registry::Chain::Bitcoin, 1.0),
            holding("DOGE", crate::registry::Chain::Dogecoin, 1.0),
        ],
    )])
    .await;
    {
        let mut state = service.wallet_state.write().await;
        state.quotes.prices.insert("ethereum:native".into(), 3000.0);
        state.quotes.prices.insert("bitcoin:native".into(), 60000.0);
        state.quotes.prices.insert("dogecoin:native".into(), 0.1);
    }
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["dogecoin".into(), "ethereum".into()],
        })
        .await
        .expect("pin");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let symbols: Vec<_> = groups.iter().map(row_symbol).collect();
    assert_eq!(symbols, ["DOGE", "ETH", "BTC"]);
    assert!(!groups[2].is_pinned);
}

#[tokio::test]
async fn portfolio_snapshot_keeps_wallets_groups_and_valuation_on_one_version() {
    let service = service_with(vec![(
        "w1",
        crate::registry::Chain::Ethereum,
        vec![holding("ETH", crate::registry::Chain::Ethereum, 2.0)],
    )])
    .await;
    service
        .wallet_state
        .write()
        .await
        .quotes
        .prices
        .insert("ethereum:native".into(), 3000.0);
    let before = service.portfolio_snapshot().await.unwrap();
    service
        .apply_state_command(StateCommand::SetWalletPortfolioInclusion {
            wallet_id: "w1".into(),
            included: false,
        })
        .await
        .unwrap();
    let after = service.portfolio_snapshot().await.unwrap();
    assert!(after.revision > before.revision);
    assert_eq!(before.valuation.portfolio.total, 6000.0);
    assert!(before.wallets[0].include_in_portfolio_total);
    assert_eq!(after.valuation.portfolio.total, 0.0);
    assert!(!after.wallets[0].include_in_portfolio_total);
    assert!(after.derived.portfolio.is_empty());
    assert!(after.groups.is_empty());
    assert_eq!(after.valuation.wallets["w1"].total, 6000.0);
}

/// A row worth less than a dollar is small, a pinned or unpriced one never.
#[tokio::test]
async fn rows_under_a_dollar_are_small_unless_pinned_or_unpriced() {
    let service = service_with(vec![(
        "w1",
        crate::registry::Chain::Ethereum,
        vec![
            holding("ETH", crate::registry::Chain::Ethereum, 0.0001),
            holding("BTC", crate::registry::Chain::Bitcoin, 1.0),
            holding("DOGE", crate::registry::Chain::Dogecoin, 3.0),
        ],
    )])
    .await;
    {
        let mut state = service.wallet_state.write().await;
        state.quotes.prices.insert("ethereum:native".into(), 3000.0);
        state.quotes.prices.insert("bitcoin:native".into(), 60000.0);
    }
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: Vec::new(),
        })
        .await
        .expect("pin");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let small = |symbol: &str| {
        groups
            .iter()
            .find(|g| row_symbol(g) == symbol)
            .unwrap()
            .is_small
    };
    assert!(small("ETH"), "0.0001 ETH is $0.30");
    assert!(!small("BTC"));
    assert!(!small("DOGE"), "unpriced, so its worth is unknown");

    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["ethereum".into()],
        })
        .await
        .expect("pin");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    assert!(
        !groups
            .iter()
            .find(|g| row_symbol(g) == "ETH")
            .unwrap()
            .is_small,
        "pinned"
    );
}

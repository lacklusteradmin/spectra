use crate::service::WalletService;
use crate::store::state::{StateCommand, WalletState};
use crate::store::wallet_domain::AssetHolding;

fn holding(symbol: &str, chain: &str, amount: f64, price: f64) -> AssetHolding {
    AssetHolding {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        coingecko_id: symbol.to_lowercase(),
        chain_name: chain.to_string(),
        token_standard: "Native".to_string(),
        contract_address: None,
        amount,
        price_usd: price,
    }
}

async fn service_with(
    wallets: Vec<(&str, &str, Vec<AssetHolding>)>,
) -> std::sync::Arc<WalletService> {
    let service = WalletService::new(Vec::new()).expect("service");
    for (id, chain, holdings) in wallets {
        let mut wallet = WalletState::single_address(id, id, chain, "addr", None, false);
        wallet.holdings = holdings;
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
/// Was per (chain, asset) — ETH on Ethereum and ETH on Arbitrum were two
/// rows. Every EVM L2 carries Ethereum's coingecko id, which is what makes
/// them one asset.
#[tokio::test]
async fn a_row_is_per_asset_and_breaks_down_by_chain() {
    let service = service_with(vec![
        (
            "w1",
            "Ethereum",
            vec![holding("ETH", "Ethereum", 1.0, 2000.0)],
        ),
        (
            "w2",
            "Ethereum",
            vec![holding("ETH", "Ethereum", 2.0, 2000.0)],
        ),
        (
            "w3",
            "Arbitrum",
            vec![holding("ETH", "Arbitrum", 5.0, 2000.0)],
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
    let total: f64 = row.holdings.iter().map(|h| h.coin.amount).sum();
    assert_eq!(total, 8.0, "both chains and both wallets summed");
    assert_eq!(row.holdings.len(), 2, "one breakdown entry per chain");

    // The row is presented as the place most of it is.
    assert_eq!(row.holdings[0].coin.chain_name, "Arbitrum");
    assert_eq!(row.holdings[0].coin.amount, 5.0);
    // And the two wallets on one chain are one entry.
    let ethereum = row
        .holdings
        .iter()
        .find(|h| h.coin.chain_name == "Ethereum")
        .expect("an Ethereum entry");
    assert_eq!(ethereum.coin.amount, 3.0);
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
    let mut real = holding("USDX", "Ethereum", 1.0, 1.0);
    real.token_standard = "ERC-20".into();
    real.contract_address = Some("0x000000000000000000000000000000000000aaaa".into());
    real.coingecko_id = String::new();
    let mut lookalike = holding("USDX", "Tron", 999.0, 1.0);
    lookalike.token_standard = "TRC-20".into();
    lookalike.contract_address = Some("T9yD14Nj9j7xAB4dbGeiX9h8unkKHxuWwb".into());
    lookalike.coingecko_id = String::new();
    // And a second contract on the same chain, same symbol.
    let mut sibling = holding("USDX", "Ethereum", 2.0, 1.0);
    sibling.token_standard = "ERC-20".into();
    sibling.contract_address = Some("0x000000000000000000000000000000000000bbbb".into());
    sibling.coingecko_id = String::new();

    let service = service_with(vec![
        ("w1", "Ethereum", vec![real, sibling]),
        ("w2", "Tron", vec![lookalike]),
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

/// A row names itself from its identity, which is the place most of it is held
/// — or the catalog's entry, for a pinned asset held nowhere.
fn row_symbol(g: &crate::store::wallet_domain::CoreDashboardAssetGroup) -> &str {
    g.identity.symbol.as_str()
}

/// A row's value: the sum of its holdings', or none when any is unpriced.
///
/// Derived rather than stored — the group used to carry a `total_value_usd`
/// beside the list it comes from.
fn row_value(g: &crate::store::wallet_domain::CoreDashboardAssetGroup) -> Option<f64> {
    g.holdings
        .iter()
        .map(|h| h.value_usd)
        .try_fold(0.0, |sum, v| v.map(|v| sum + v))
}

/// Only quotes are prices; an old value embedded in a holding is not a fallback.
#[tokio::test]
async fn an_unquoted_holding_stays_unpriced_until_a_quote_arrives() {
    let service = service_with(vec![(
        "w1",
        "Ethereum",
        vec![holding("ETH", "Ethereum", 2.0, 1000.0)],
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
        "Ethereum",
        vec![holding("ETH", "Ethereum Sepolia", 2.0, 1000.0)],
    )])
    .await;
    service
        .apply_state_command(StateCommand::SelectChainForFamily {
            chain_id: "ethereum-sepolia".into(),
        })
        .await
        .expect("select");
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

/// A pinned asset the user holds nowhere is a row that holds nothing.
///
/// `holdings` used to carry a synthesized entry so the row had a name, which
/// told the reader they held zero of the asset on whichever chain the catalog
/// listed first — a place they had never been shown. The name is `identity`'s
/// job; `holdings` says only where it is actually held.
#[tokio::test]
async fn a_pinned_asset_held_nowhere_holds_nothing() {
    let service = service_with(vec![(
        "w1",
        "Ethereum",
        vec![holding("ETH", "Ethereum", 1.0, 2000.0)],
    )])
    .await;
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["ethereum".into(), "solana".into()],
        })
        .await
        .expect("pin");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;

    let solana = groups.iter().find(|g| g.id == "solana").expect("a row");
    assert!(
        solana.holdings.is_empty(),
        "the user holds no SOL, so the row holds nothing: {:?}",
        solana.holdings
    );
    assert_eq!(solana.identity.symbol, "SOL", "and still names itself");
    assert_eq!(solana.identity.chain_name, "Solana");

    let ethereum = groups.iter().find(|g| g.id == "ethereum").expect("a row");
    assert_eq!(ethereum.holdings.len(), 1, "a held asset keeps its places");
    assert_eq!(
        ethereum.identity, ethereum.holdings[0].coin,
        "and is named by the largest of them"
    );
}

/// Pinned rows come first, in the order they were pinned, and a pinned
/// symbol with no holdings still gets a row.
#[tokio::test]
async fn pinned_rows_lead_in_pin_order() {
    let service = service_with(vec![(
        "w1",
        "Ethereum",
        vec![
            holding("ETH", "Ethereum", 1.0, 2000.0),
            holding("BTC", "Bitcoin", 1.0, 60000.0),
        ],
    )])
    .await;
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["ethereum".into(), "solana".into()],
        })
        .await
        .expect("pin");
    let groups = service.portfolio_snapshot().await.expect("snapshot").groups;
    let symbols: Vec<_> = groups.iter().map(row_symbol).collect();
    // ETH before SOL because that is the pin order, and both before the
    // unpinned BTC even though BTC is worth more.
    assert_eq!(symbols.first(), Some(&"ETH"));
    assert!(
        groups
            .iter()
            .any(|g| row_symbol(g) == "BTC" && !g.is_pinned),
        "BTC is still shown, unpinned"
    );
}

#[tokio::test]
async fn portfolio_snapshot_keeps_wallets_groups_and_valuation_on_one_version() {
    let service = service_with(vec![(
        "w1",
        "Ethereum",
        vec![holding("ETH", "Ethereum", 2.0, 99.0)],
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
    assert!(after.groups.iter().all(|group| group.holdings.is_empty()));
    assert_eq!(after.valuation.wallets["w1"].total, 6000.0);
}

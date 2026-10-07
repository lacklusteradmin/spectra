//! A hidden holding counts in no total and on no dashboard row, stays
//! sendable, and stays hidden across reopening.

use crate::registry::Chain;
use crate::service::WalletService;
use crate::store::state::{StateCommand, WalletState};

fn holding(deployment_id: &str, amount: &str) -> crate::store::wallet_domain::AssetHolding {
    let mut holding = crate::tokens::deployment(deployment_id)
        .unwrap()
        .holding_template();
    holding.amount = amount.into();
    holding
}

async fn service(db: Option<&std::path::Path>) -> std::sync::Arc<WalletService> {
    let service = WalletService::new(Vec::new()).unwrap();
    if let Some(db) = db {
        service
            .open_state(db.to_string_lossy().into_owned())
            .await
            .unwrap();
    }
    service
}

fn hide(deployment_id: &str, hidden: bool) -> StateCommand {
    StateCommand::SetHoldingHidden {
        wallet_id: "w".into(),
        deployment_id: deployment_id.into(),
        hidden,
    }
}

#[tokio::test]
async fn a_hidden_holding_counts_nowhere_but_stays_sendable() {
    let token = crate::store::built_in_token_preferences()
        .into_iter()
        .find(|preference| preference.token.chain_id == Chain::Ethereum)
        .unwrap()
        .token
        .deployment_id;
    let service = service(None).await;
    let mut wallet = WalletState::single_address("w", "W", Chain::Ethereum, "0xabc", None, false);
    wallet.holdings = vec![holding("ethereum:native", "1"), holding(&token, "10")];
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .unwrap();
    {
        let mut state = service.wallet_state.write().await;
        state.quotes.prices.insert("ethereum:native".into(), 3000.0);
        state.quotes.prices.insert(token.clone(), 2.0);
    }
    let totals = |service: std::sync::Arc<WalletService>| async move {
        let snapshot = service.portfolio_snapshot().await.unwrap();
        (
            snapshot.valuation.wallets["w"].total,
            snapshot.valuation.portfolio.total,
            snapshot.derived.included_portfolio_holdings.len(),
            snapshot.derived.send_coins_by_wallet_id["w"].len(),
        )
    };
    let before = totals(service.clone()).await;
    service
        .apply_state_command(hide(&token, true))
        .await
        .unwrap();
    let after = totals(service.clone()).await;
    assert_eq!(after.2, before.2 - 1, "the dashboard rows leave it out");
    assert_eq!(after.3, before.3, "it stays sendable");
    assert_eq!((before.0, before.1), (3020.0, 3020.0));
    assert_eq!((after.0, after.1), (3000.0, 3000.0));
    assert_eq!(
        service.app_state().await.wallets[0].hidden_holdings,
        std::slice::from_ref(&token)
    );
    // Hiding twice changes nothing; showing it again restores the totals.
    service
        .apply_state_command(hide(&token, true))
        .await
        .unwrap();
    assert_eq!(
        service.app_state().await.wallets[0].hidden_holdings.len(),
        1
    );
    service
        .apply_state_command(hide(&token, false))
        .await
        .unwrap();
    assert_eq!(totals(service.clone()).await, before);
}

/// Only a holding the wallet has can be hidden.
#[tokio::test]
async fn only_a_held_asset_can_be_hidden() {
    let service = service(None).await;
    let mut wallet = WalletState::single_address("w", "W", Chain::Ethereum, "0xabc", None, false);
    wallet.holdings = vec![holding("ethereum:native", "1")];
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .unwrap();
    service
        .apply_state_command(hide("solana:native", true))
        .await
        .unwrap();
    assert!(
        service.app_state().await.wallets[0]
            .hidden_holdings
            .is_empty()
    );
}

#[tokio::test]
async fn a_hidden_holding_stays_hidden_after_reopening() {
    let directory = std::env::temp_dir().join(crate::store::new_event_id());
    std::fs::create_dir_all(&directory).unwrap();
    let db = directory.join("state.sqlite");
    {
        let service = service(Some(&db)).await;
        let mut wallet =
            WalletState::single_address("w", "W", Chain::Ethereum, "0xabc", None, false);
        wallet.holdings = vec![holding("ethereum:native", "1")];
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        service
            .apply_state_command(hide("ethereum:native", true))
            .await
            .unwrap();
    }
    let reopened = service(Some(&db)).await;
    assert_eq!(
        reopened.app_state().await.wallets[0].hidden_holdings,
        ["ethereum:native"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

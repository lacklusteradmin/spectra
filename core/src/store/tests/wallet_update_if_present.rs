use crate::service::WalletService;
use crate::state::{StateCommand, WalletSummary};

fn wallet(id: &str, name: &str) -> WalletSummary {
    WalletSummary::single_address(id, name, "Bitcoin", "bc1qexample", None, false)
}

/// A balance result that arrives after the wallet was deleted must not
/// bring it back.
#[tokio::test]
async fn does_not_resurrect_a_deleted_wallet() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: wallet("w1", "Cold"),
        })
        .await
        .expect("upsert");
    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: "w1".to_string(),
        })
        .await
        .expect("remove");

    let late = service
        .apply_state_command(StateCommand::UpdateWalletIfPresent {
            wallet: wallet("w1", "Cold with fresh balance"),
        })
        .await
        .expect("update");
    assert!(late.state.wallets.is_empty(), "wallet came back");
    assert!(late.events.is_empty());
}

#[tokio::test]
async fn updates_a_wallet_that_is_still_there() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: wallet("w1", "Cold"),
        })
        .await
        .expect("upsert");
    let updated = service
        .apply_state_command(StateCommand::UpdateWalletIfPresent {
            wallet: wallet("w1", "Renamed"),
        })
        .await
        .expect("update");
    assert_eq!(updated.state.wallets[0].name, "Renamed");
    assert_eq!(updated.events.len(), 1);
}

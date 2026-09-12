use crate::service::WalletService;
use crate::state::{CoreAppState, StateCommand};

fn tmp_db(tag: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-owned-state-{tag}-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

fn service() -> std::sync::Arc<WalletService> {
    WalletService::new_typed(Vec::new()).expect("service")
}

#[tokio::test]
async fn defaults_to_usd_before_anything_is_stored() {
    let service = service();
    let db = tmp_db("defaults");
    let state = service.open_state(db.clone()).await.expect("open");
    assert_eq!(state.settings.fiat_currency_code, "USD");
    // Everything a user supplies is still empty; the token list is not one
    // of those, because opening seeds it from the catalog rather than
    // leaving a caller to remember the merge.
    assert_eq!(
        CoreAppState {
            token_preferences: Vec::new(),
            ..state.clone()
        },
        CoreAppState::default()
    );
    assert_eq!(
        state.token_preferences,
        crate::store::plan_merge_built_in_token_preferences(
            crate::store::built_in_token_preferences(),
            Vec::new()
        ),
        "opening seeds the catalog's own list, in the order the merge gives it"
    );
    let _ = std::fs::remove_file(&db);
}

/// The point of Stage 0: a command changes core's state and survives a
/// restart without the caller doing anything to save it.
#[tokio::test]
async fn a_command_persists_without_the_caller_saving() {
    let db = tmp_db("persist");

    let first = service();
    first.open_state(db.clone()).await.expect("open");
    let transition = first
        .apply_state_command(StateCommand::SetFiatCurrency {
            fiat_currency_code: "EUR".to_string(),
        })
        .await
        .expect("apply");
    assert_eq!(transition.state.settings.fiat_currency_code, "EUR");
    assert_eq!(transition.events.len(), 1);
    assert_eq!(transition.events[0].kind, "fiatCurrencyChanged");

    // A second service, as a second process would see it.
    let second = service();
    let reopened = second.open_state(db.clone()).await.expect("reopen");
    assert_eq!(reopened.settings.fiat_currency_code, "EUR");
    assert_eq!(second.fiat_currency_code().await, "EUR");

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn currency_codes_are_normalized() {
    let service = service();
    service.open_state(tmp_db("normalize")).await.expect("open");
    let transition = service
        .apply_state_command(StateCommand::SetFiatCurrency {
            fiat_currency_code: "  eur \n".to_string(),
        })
        .await
        .expect("apply");
    assert_eq!(transition.state.settings.fiat_currency_code, "EUR");
}

/// Setting a value to what it already is is not a change: no event, and
/// nothing is written.
#[tokio::test]
async fn a_no_op_command_emits_no_event() {
    let service = service();
    service.open_state(tmp_db("noop")).await.expect("open");
    let transition = service
        .apply_state_command(StateCommand::SetFiatCurrency {
            fiat_currency_code: "usd".to_string(),
        })
        .await
        .expect("apply");
    assert!(transition.events.is_empty());
    assert_eq!(transition.state.settings.fiat_currency_code, "USD");
}

/// Without `open_state` the service still works, in memory only. Tests and
/// short-lived tools rely on this.
#[tokio::test]
async fn commands_apply_in_memory_when_no_database_is_bound() {
    let service = service();
    let transition = service
        .apply_state_command(StateCommand::SetFiatCurrency {
            fiat_currency_code: "JPY".to_string(),
        })
        .await
        .expect("apply");
    assert_eq!(transition.state.settings.fiat_currency_code, "JPY");
    assert_eq!(service.fiat_currency_code().await, "JPY");
}

#[tokio::test]
async fn field_intents_do_not_overwrite_each_other_or_resurrect_wallets() {
    use crate::store::state::{StateCommand, WalletSummary};
    let service = crate::service::WalletService::new_typed(vec![]).unwrap();
    let wallet = WalletSummary::single_address(
        "intent",
        "Original",
        "Ethereum",
        "0x1111111111111111111111111111111111111111",
        None,
        true,
    );
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .unwrap();
    let (rename, inclusion) = tokio::join!(
        service.apply_state_command(StateCommand::RenameWallet {
            wallet_id: "intent".into(),
            name: "  Renamed  ".into()
        }),
        service.apply_state_command(StateCommand::SetWalletPortfolioInclusion {
            wallet_id: "intent".into(),
            included: false
        })
    );
    rename.unwrap();
    inclusion.unwrap();
    let wallet = service.app_state().await.wallets.remove(0);
    assert_eq!(wallet.name, "Renamed");
    assert!(!wallet.include_in_portfolio_total);
    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: "intent".into(),
        })
        .await
        .unwrap();
    service
        .apply_state_command(StateCommand::RenameWallet {
            wallet_id: "intent".into(),
            name: "Late".into(),
        })
        .await
        .unwrap();
    assert!(service.app_state().await.wallets.is_empty());
}

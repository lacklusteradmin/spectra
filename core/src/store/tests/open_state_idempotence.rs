use crate::service::WalletService;
use crate::state::StateCommand;

fn tmp_db() -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-open-idem-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

/// A second `open_state` must not discard state written since the first.
/// The app calls it from a launch reload that races user actions.
#[tokio::test]
async fn reopening_does_not_revert_newer_writes() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let db = tmp_db();
    service.open_state(db.clone()).await.expect("open");

    service
        .apply_state_command(StateCommand::SetFiatCurrency {
            fiat_currency_code: "EUR".to_string(),
        })
        .await
        .expect("apply");

    let reopened = service.open_state(db.clone()).await.expect("reopen");
    assert_eq!(reopened.settings.fiat_currency_code, "EUR");
    let _ = std::fs::remove_file(&db);
}

/// A different database is a genuine open, and does replace the state.
#[tokio::test]
async fn opening_a_different_database_replaces_the_state() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let first = tmp_db();
    service.open_state(first.clone()).await.expect("open");
    service
        .apply_state_command(StateCommand::SetFiatCurrency {
            fiat_currency_code: "EUR".to_string(),
        })
        .await
        .expect("apply");

    let second = format!("{first}.other");
    let _ = std::fs::remove_file(&second);
    let switched = service
        .open_state(second.clone())
        .await
        .expect("open other");
    assert_eq!(switched.settings.fiat_currency_code, "USD");

    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
}

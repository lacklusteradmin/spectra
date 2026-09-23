//! History pagination and diagnostics rows go with the commands that
//! invalidate them, rather than with a front end remembering to reset them.
use crate::service::WalletService;
use crate::store::state::{AppSettingUpdate, StateCommand, WalletState};

fn db(label: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "spectra-feed-{label}-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned()
}

fn row(wallet_id: &str) -> crate::diagnostics::HistoryDiagnostics {
    crate::diagnostics::HistoryDiagnostics {
        wallet_id: wallet_id.to_string(),
        identifier: "addr".into(),
        source_used: "esplora".into(),
        transaction_count: 1,
        scanned_count: None,
        next_cursor: None,
        error: None,
        per_source: Vec::new(),
    }
}

#[tokio::test]
async fn removing_a_wallet_forgets_its_pagination_and_diagnostics() {
    let path = db("remove");
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(path.clone()).await.expect("open");
    // A unique id: the diagnostics registry is shared by every test.
    let wallet_id = format!("feed-{}", crate::store::new_event_id());
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address(
                wallet_id.clone(),
                "Watch",
                "Bitcoin",
                "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
                None,
                true,
            ),
        })
        .await
        .expect("upsert");
    service.advance_history_cursor("bitcoin".into(), wallet_id.clone(), None);
    crate::diagnostics::diagnostics_record("Bitcoin".into(), row(&wallet_id));
    assert!(
        service
            .history_cursor("bitcoin".into(), wallet_id.clone())
            .is_exhausted
    );

    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: wallet_id.clone(),
        })
        .await
        .expect("remove");

    assert!(
        !service
            .history_cursor("bitcoin".into(), wallet_id.clone())
            .is_exhausted
    );
    assert!(!crate::diagnostics::diagnostics_all("Bitcoin".into()).contains_key(&wallet_id));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn a_network_switch_or_an_esplora_change_restarts_the_family_feed() {
    let path = db("switch");
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(path.clone()).await.expect("open");

    service.advance_history_cursor("bitcoin".into(), "w".into(), None);
    service
        .apply_state_command(StateCommand::SelectChainForFamily {
            chain_id: "bitcoin-testnet".into(),
        })
        .await
        .expect("switch");
    assert!(
        !service
            .history_cursor("bitcoin".into(), "w".into())
            .is_exhausted
    );

    service.advance_history_cursor("bitcoin".into(), "w".into(), None);
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: AppSettingUpdate::AddCustomEndpoint {
                capabilities: crate::endpoint_capability_options(
                    "bitcoin".into(),
                    crate::EndpointApi::Esplora,
                ),
                chain_id: "bitcoin".into(),
                api: "esplora".into(),
                endpoint: "https://custom.example/api".into(),
            },
        })
        .await
        .expect("esplora");
    assert!(
        !service
            .history_cursor("bitcoin".into(), "w".into())
            .is_exhausted
    );

    // A change that is not about the feed leaves it where it was.
    service.advance_history_cursor("bitcoin".into(), "w".into(), None);
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: AppSettingUpdate::UsePriceAlerts { value: false },
        })
        .await
        .expect("unrelated");
    assert!(
        service
            .history_cursor("bitcoin".into(), "w".into())
            .is_exhausted
    );
    let _ = std::fs::remove_file(&path);
}

/// A history run writes its own rows and the chain's health. The app did this
/// from the result of the call, on the two paths it drove; the scheduled
/// refresh core runs recorded nothing.
#[tokio::test]
async fn a_history_run_records_its_rows_and_the_chains_health() {
    use crate::registry::Chain;
    use crate::service::{HistoryRefreshOutcome, HistoryWalletDiagnostics};
    let path = db("history-run");
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(path.clone()).await.expect("open");
    let wallet_id = format!("run-{}", crate::store::new_event_id());
    let outcome = |refreshed: u32, failed: u32| HistoryRefreshOutcome {
        wallets_refreshed: refreshed,
        wallets_failed: failed,
        added: 0,
        updated: 0,
        exhausted: true,
        diagnostics: vec![HistoryWalletDiagnostics {
            wallet_id: wallet_id.clone(),
            identifier: "ltc1q".into(),
            source_used: "esplora".into(),
            transaction_count: 3,
            next_cursor: None,
            error: None,
        }],
    };

    service
        .record_history_run(Chain::Litecoin, &Ok(outcome(0, 1)))
        .await;
    let rows = crate::diagnostics::diagnostics_all("Litecoin".into());
    assert_eq!(
        rows.get(&wallet_id).map(|row| row.transaction_count),
        Some(3)
    );
    let state = service.diagnostic_state().await;
    assert_eq!(
        state.degraded.get("Litecoin").map(String::as_str),
        Some("Litecoin history refresh failed. Using cached history.")
    );

    service
        .record_history_run(Chain::Litecoin, &Ok(outcome(1, 0)))
        .await;
    let state = service.diagnostic_state().await;
    assert!(!state.degraded.contains_key("Litecoin"));
    assert!(state.last_good_unix.contains_key("Litecoin"));
    let _ = std::fs::remove_file(&path);
}

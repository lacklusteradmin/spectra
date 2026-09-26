use crate::service::DiagnosticLogLevel;
use crate::service::WalletService;

fn tmp_db(label: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-events-{label}-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn events_survive_reopening_the_database() {
    let db = tmp_db("reopen");
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(db.clone()).await.expect("open");
    service
        .append_chain_operational_event(
            "bitcoin".into(),
            DiagnosticLogLevel::Warning,
            "broadcast deferred".into(),
            Some("abc123".into()),
        )
        .await
        .expect("append");

    let reopened = WalletService::new(Vec::new()).expect("service");
    reopened.open_state(db.clone()).await.expect("open");
    let events = reopened.operational_events("bitcoin".into()).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].input.message, "broadcast deferred");
    assert_eq!(events[0].input.level, DiagnosticLogLevel::Warning);
    assert_eq!(events[0].input.transaction_hash.as_deref(), Some("abc123"));
    assert!(
        events[0].timestamp_unix > 0.0,
        "core did not stamp the time"
    );

    let _ = std::fs::remove_file(&db);
}

/// Newest first, and the cap holds — the property the planner stated but
/// could not enforce, because a caller wrote the answer down.
#[tokio::test]
async fn the_log_is_newest_first_and_bounded() {
    let service = WalletService::new(Vec::new()).expect("service");
    for index in 0..205 {
        service
            .append_chain_operational_event(
                "solana".into(),
                DiagnosticLogLevel::Info,
                format!("event {index}"),
                None,
            )
            .await
            .expect("append");
    }
    let events = service.operational_events("solana".into()).await;
    assert_eq!(events.len(), 200, "the cap did not hold");
    assert_eq!(events[0].input.message, "event 204");
    assert_eq!(events[199].input.message, "event 5");
    // A different chain keeps its own list.
    assert!(
        service
            .operational_events("bitcoin".into())
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn clearing_one_chain_leaves_the_others() {
    let service = WalletService::new(Vec::new()).expect("service");
    for chain in ["bitcoin", "solana"] {
        service
            .append_chain_operational_event(
                chain.into(),
                DiagnosticLogLevel::Error,
                "send failed".into(),
                None,
            )
            .await
            .expect("append");
    }
    service
        .clear_operational_events(Some("bitcoin".into()))
        .await
        .expect("clear one");
    assert!(
        service
            .operational_events("bitcoin".into())
            .await
            .is_empty()
    );
    assert_eq!(service.operational_events("solana".into()).await.len(), 1);

    service
        .clear_operational_events(None)
        .await
        .expect("clear all");
    assert!(service.operational_events("solana".into()).await.is_empty());
}

/// Core logs the work it performs, so every front end gets the same lines: a
/// failed rescan and a failed status recheck each leave one, with no caller
/// appending anything.
#[tokio::test]
async fn core_records_its_own_refresh_and_recheck_outcomes() {
    use crate::fetch::refresh_policy::DeviceConditions;
    use crate::service::app_refresh::AppRefreshIntent;
    let db = tmp_db("core-owned");
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(db).await.expect("open");
    let offline = DeviceConditions {
        app_is_active: true,
        is_network_reachable: false,
        is_constrained_network: false,
        is_expensive_network: false,
        is_low_power_mode: false,
        battery_level: 1.0,
        wants_price_refresh: false,
    };
    service
        .refresh_app(
            AppRefreshIntent::DeepRescan {
                chain_id: "bitcoin".into(),
            },
            offline,
        )
        .await
        .expect("an offline rescan answers with its failure");
    let rescan = service.operational_events("bitcoin".into()).await;
    assert!(
        rescan
            .iter()
            .any(|e| e.input.category == "Rescan" && e.input.level == DiagnosticLogLevel::Warning)
    );
    assert!(
        rescan
            .iter()
            .all(|e| e.input.source.as_deref() == Some("core"))
    );

    assert!(
        service
            .recheck_transaction_status("missing".into())
            .await
            .is_err()
    );
    let logs = service.diagnostic_state().await.logs;
    assert!(
        logs.iter()
            .any(|e| e.input.category == "Pending Transactions"
                && e.input.level == DiagnosticLogLevel::Error)
    );
}

/// The durable log trims every field and keeps the newest 800 lines. This is
/// core's rule; front ends only append.
#[tokio::test]
async fn appended_lines_are_trimmed_and_capped_at_eight_hundred() {
    use crate::service::{DiagnosticCommand, DiagnosticLogInput};
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(tmp_db("cap")).await.expect("open");
    let input = |message: String| DiagnosticLogInput {
        level: DiagnosticLogLevel::Info,
        category: "  Network  ".into(),
        message,
        chain_id: Some(" bitcoin ".into()),
        wallet_id: Some("   ".into()),
        transaction_hash: None,
        source: Some(" rpc ".into()),
        metadata: None,
    };
    for index in 0..810 {
        service
            .apply_diagnostic_command(DiagnosticCommand::Append {
                input: input(format!(" Event {index} ")),
            })
            .await
            .expect("append");
    }
    let logs = service.diagnostic_state().await.logs;
    assert_eq!(logs.len(), 800);
    let newest = &logs[0].input;
    assert_eq!(newest.message, "Event 809");
    assert_eq!(newest.category, "Network");
    assert_eq!(newest.chain_id.as_deref(), Some("bitcoin"));
    assert_eq!(newest.source.as_deref(), Some("rpc"));
    assert_eq!(newest.wallet_id, None, "a blank field is no field");
    assert_eq!(logs[799].input.message, "Event 10");
}

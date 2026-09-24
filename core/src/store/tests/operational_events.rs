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
    assert!(service
        .operational_events("bitcoin".into())
        .await
        .is_empty());
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
    assert!(service
        .operational_events("bitcoin".into())
        .await
        .is_empty());
    assert_eq!(service.operational_events("solana".into()).await.len(), 1);

    service
        .clear_operational_events(None)
        .await
        .expect("clear all");
    assert!(service.operational_events("solana".into()).await.is_empty());
}

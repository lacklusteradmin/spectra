use crate::service::WalletService;

#[tokio::test]
async fn receive_reservation_is_stable_across_calls() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let first = service
        .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
        .await
        .expect("reserve");
    let second = service
        .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
        .await
        .expect("reserve");
    // Opening the receive sheet twice must not burn two addresses.
    assert_eq!(first, second);
}

#[tokio::test]
async fn change_indices_are_never_handed_out_twice() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let mut handles = Vec::new();
    for _ in 0..32 {
        let service = service.clone();
        handles.push(tokio::spawn(async move {
            service
                .reserve_change_index("w1".into(), "Bitcoin".into())
                .await
                .expect("reserve")
        }));
    }
    let mut seen = Vec::new();
    for handle in handles {
        seen.push(handle.await.expect("join"));
    }
    seen.sort_unstable();
    let unique: std::collections::HashSet<_> = seen.iter().copied().collect();
    assert_eq!(unique.len(), 32, "an index was reserved twice: {seen:?}");
    assert_eq!(seen, (0..32).collect::<Vec<i64>>());
}

#[tokio::test]
async fn clearing_a_reservation_frees_the_next_index() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let first = service
        .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
        .await
        .expect("reserve");
    service
        .clear_reserved_receive_index("w1".into(), "Bitcoin".into())
        .await
        .expect("clear");
    let second = service
        .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
        .await
        .expect("reserve");
    // The used address must not be reissued.
    assert_ne!(first, second);
    assert_eq!(second, first + 1);
}

#[tokio::test]
async fn keypool_survives_reopening_the_database() {
    let db = {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-keypool-reopen-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    };
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service.open_state(db.clone()).await.expect("open");
    let reserved = service
        .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
        .await
        .expect("reserve");

    let reopened = WalletService::new_typed(Vec::new()).expect("service");
    reopened.open_state(db.clone()).await.expect("open");
    let after = reopened
        .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
        .await
        .expect("reserve");
    // A restart must not reissue the address already handed out.
    assert_eq!(reserved, after);

    let _ = std::fs::remove_file(&db);
}

/// The baseline is core's now, so a recorded owned address has to move it
/// without anyone passing one in.
///
/// This is the property the old shape could not have: the caller supplied
/// the baseline, so the reservation was only as current as the caller's
/// copy of the owned-address table.
#[tokio::test]
async fn a_recorded_owned_address_raises_the_baseline() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    // Bitcoin is a deep-UTXO chain, so the baseline reads indices.
    service
        .register_owned_address(
            "w1".into(),
            "Bitcoin".into(),
            "bc1qexample".into(),
            None,
            Some("external".into()),
            Some(7),
        )
        .await
        .expect("register");
    let reserved = service
        .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
        .await
        .expect("reserve");
    assert_eq!(
        reserved, 8,
        "index 7 was already handed out; the next receive index must clear it"
    );
}

/// The table is core's, so it has to come back on its own.
#[tokio::test]
async fn owned_addresses_survive_reopening_the_database() {
    let db = {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-owned-reopen-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    };
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service.open_state(db.clone()).await.expect("open");
    service
        .register_owned_address(
            "w1".into(),
            "Bitcoin".into(),
            "bc1qexample".into(),
            Some("m/84'/0'/0'/0/3".into()),
            Some("external".into()),
            Some(3),
        )
        .await
        .expect("register");

    let reopened = WalletService::new_typed(Vec::new()).expect("service");
    reopened.open_state(db.clone()).await.expect("open");
    assert_eq!(
        reopened
            .owned_addresses_for_wallet("w1".into(), Some("Bitcoin".into()))
            .await,
        vec!["bc1qexample".to_string()]
    );
    // And the baseline it feeds comes back with it.
    assert_eq!(
        reopened
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve"),
        4
    );

    let _ = std::fs::remove_file(&db);
}

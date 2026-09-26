use crate::service::WalletService;

#[tokio::test]
async fn receive_reservation_is_stable_across_calls() {
    let service = WalletService::new(Vec::new()).expect("service");
    let first = service
        .reserve_receive_index("w1".into(), "bitcoin".into(), 0)
        .await
        .expect("reserve");
    let second = service
        .reserve_receive_index("w1".into(), "bitcoin".into(), 0)
        .await
        .expect("reserve");
    // Opening the receive sheet twice must not burn two addresses.
    assert_eq!(first, second);
}

#[tokio::test]
async fn change_indices_are_never_handed_out_twice() {
    let service = WalletService::new(Vec::new()).expect("service");
    let mut handles = Vec::new();
    for _ in 0..32 {
        let service = service.clone();
        handles.push(tokio::spawn(async move {
            service
                .reserve_change_index("w1".into(), "bitcoin".into())
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
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(db.clone()).await.expect("open");
    let reserved = service
        .reserve_receive_index("w1".into(), "bitcoin".into(), 0)
        .await
        .expect("reserve");

    let reopened = WalletService::new(Vec::new()).expect("service");
    reopened.open_state(db.clone()).await.expect("open");
    let after = reopened
        .reserve_receive_index("w1".into(), "bitcoin".into(), 0)
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
    let service = WalletService::new(Vec::new()).expect("service");
    // Bitcoin is a deep-UTXO chain, so the baseline reads indices.
    service
        .register_owned_address(
            "w1".into(),
            "bitcoin".into(),
            "bc1qexample".into(),
            None,
            Some("external".into()),
            Some(7),
        )
        .await
        .expect("register");
    let reserved = service
        .reserve_receive_index("w1".into(), "bitcoin".into(), 0)
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
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(db.clone()).await.expect("open");
    service
        .register_owned_address(
            "w1".into(),
            "bitcoin".into(),
            "bc1qexample".into(),
            Some("m/84'/0'/0'/0/3".into()),
            Some("external".into()),
            Some(3),
        )
        .await
        .expect("register");

    let reopened = WalletService::new(Vec::new()).expect("service");
    reopened.open_state(db.clone()).await.expect("open");
    assert_eq!(
        reopened
            .owned_addresses_for_wallet("w1".into(), Some("bitcoin".into()))
            .await,
        vec!["bc1qexample".to_string()]
    );
    // And the baseline it feeds comes back with it.
    assert_eq!(
        reopened
            .reserve_receive_index("w1".into(), "bitcoin".into(), 0)
            .await
            .expect("reserve"),
        4
    );

    let _ = std::fs::remove_file(&db);
}

/// The diagnostics row reports the reserved address as it was recorded when
/// it was handed out — its path included — and not the wallet's account path.
#[tokio::test]
async fn keypool_diagnostics_report_the_recorded_reservation() {
    use crate::store::state::{StateCommand, WalletState};

    let db = std::env::temp_dir()
        .join(format!(
            "spectra-keypool-diagnostics-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(db.clone()).await.expect("open");
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address(
                "w1",
                "Savings",
                "bitcoin",
                "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
                None,
                true,
            ),
        })
        .await
        .expect("upsert");

    let before = service
        .keypool_diagnostics("bitcoin".into())
        .await
        .expect("diagnostics");
    assert_eq!(before.len(), 1);
    assert!(before[0].reserved_receive.is_none(), "nothing reserved yet");

    let index = service
        .reserve_receive_index("w1".into(), "bitcoin".into(), 7)
        .await
        .expect("reserve");
    service
        .register_owned_address(
            "w1".into(),
            "bitcoin".into(),
            "bc1qreserved".into(),
            Some(format!("m/84'/0'/0'/0/{index}")),
            Some("external".into()),
            Some(index),
        )
        .await
        .expect("register");

    let rows = service
        .keypool_diagnostics("bitcoin".into())
        .await
        .expect("diagnostics");
    assert_eq!(rows[0].wallet_name, "Savings");
    assert_eq!(rows[0].keypool.reserved_receive_index, Some(index));
    let reserved = rows[0].reserved_receive.as_ref().expect("the reserved row");
    assert_eq!(reserved.address, "bc1qreserved");
    assert_eq!(
        reserved.derivation_path.as_deref(),
        Some(format!("m/84'/0'/0'/0/{index}").as_str()),
        "the path at the reserved index, not the account path"
    );

    let _ = std::fs::remove_file(&db);
}

/// A wallet's known addresses come from core's own tables, each once.
#[tokio::test]
async fn known_wallet_addresses_merge_the_wallet_and_its_owned_rows() {
    use crate::store::state::{StateCommand, WalletState};

    let db = std::env::temp_dir()
        .join(format!(
            "spectra-known-addresses-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(db.clone()).await.expect("open");
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address(
                "w1",
                "Watch",
                "ethereum",
                "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
                None,
                true,
            ),
        })
        .await
        .expect("upsert");
    for address in [
        // The same address in another case is the same row.
        "0x742d35cc6634c0532925a3b844bc454e4438f44e",
        "0x1111111111111111111111111111111111111111",
    ] {
        service
            .register_owned_address(
                "w1".into(),
                "ethereum".into(),
                address.into(),
                None,
                None,
                None,
            )
            .await
            .expect("register");
    }

    assert_eq!(
        service
            .known_wallet_addresses("w1".into())
            .await
            .expect("known"),
        vec![
            "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            "0x1111111111111111111111111111111111111111".to_string(),
        ]
    );
    assert!(
        service
            .known_wallet_addresses("nobody".into())
            .await
            .expect("known")
            .is_empty()
    );

    let _ = std::fs::remove_file(&db);
}

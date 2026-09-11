use crate::state::CoreAppState;

/// Refusing corrupt metadata must leave every wallet and the bad bytes on disk.
#[test]
fn unreadable_preferences_refuse_loading_without_deleting_wallets() {
    let db = {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-badrow-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    };

    let mut state = CoreAppState::default();
    state.wallets.push(crate::store::state::WalletSummary {
        id: "w1".into(),
        name: "Kept".into(),
        is_watch_only: false,
        chain_name: "Bitcoin".into(),
        include_in_portfolio_total: true,
        network_mode: None,
        xpub: None,
        derivation_preset: "standard".into(),
        derivation_path: None,
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: Vec::new(),
    });
    crate::store::wallet_db::app_state_save(&db, &state).expect("save");

    // Overwrite the token-preferences blob with a shape this build cannot
    // read — an older row, or a newer one.
    // Straight into the meta table, the way an older build would have left
    // it — no helper, so the test cannot accidentally go through a path
    // that normalises the row on the way in.
    {
        let conn = rusqlite::Connection::open(&db).expect("open");
        conn.execute(
            "INSERT OR REPLACE INTO app_state_meta (key, value) VALUES (?1, ?2)",
            rusqlite::params!["token_preferences", r#"[{"legacy":true}]"#],
        )
        .expect("write the bad row");
    }

    // The row is dropped and rebuilt from the catalog on the next
    // evaluation; the load itself succeeds, because what it would take
    // down with it — the wallet list — cannot be rebuilt from anything.
    let loaded = crate::store::wallet_db::app_state_load(&db).expect("load");
    assert!(loaded.token_preferences.is_empty());
    assert_eq!(loaded.wallets.len(), 1);
    let wallets = crate::store::wallet_db::wallet_load_all(&db).unwrap();
    assert_eq!(wallets.len(), 1);
    assert_eq!(wallets[0].name, "Kept");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT value FROM app_state_meta WHERE key = 'token_preferences'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, r#"[{"legacy":true}]"#);
}

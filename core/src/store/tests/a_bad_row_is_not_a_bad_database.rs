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
    state.wallets.push(crate::store::state::WalletState {
        id: "w1".into(),
        name: "Kept".into(),
        is_watch_only: false,
        chain_name: "Bitcoin".into(),
        include_in_portfolio_total: true,
        chain_id: "bitcoin".into(),
        xpub: None,
        derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
        derivation_path: None,
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: Vec::new(),
    });
    crate::store::wallet_db::app_state_save(&crate::wallet_db::WalletDatabase::new(&db), &state)
        .expect("save");

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
    let loaded =
        crate::store::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db))
            .expect("load");
    assert!(loaded.token_preferences.is_empty());
    assert_eq!(loaded.wallets.len(), 1);
    let wallets =
        crate::store::wallet_db::wallet_load_all(&crate::wallet_db::WalletDatabase::new(&db))
            .unwrap();
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

/// A wallet row this build cannot decode costs that row, not the app.
///
/// The shape stored under `derivationOverrides` shrank from ten fields to two
/// and gained `deny_unknown_fields`, so every row an earlier build wrote
/// carries a `mnemonicWordlist` this one refuses. That refusal used to fail
/// `wallet_load_all`, and with it `app_state_load`, `open_state`, and every
/// call that waits on `open_state` — the install could not list a wallet,
/// import one, or reset itself, and deleting the app was the only way out.
///
/// Three things are asserted together because the fix is only safe if all
/// three hold: the readable wallet still loads, the refused bytes are still on
/// disk, and the commit that follows the load does not prune the row it could
/// not see.
#[test]
fn an_unreadable_wallet_row_does_not_take_the_readable_ones_with_it() {
    let db = {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-badwallet-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    };

    let wallet = |id: &str, name: &str| crate::store::state::WalletState {
        id: id.into(),
        name: name.into(),
        is_watch_only: false,
        chain_name: "Bitcoin".into(),
        include_in_portfolio_total: true,
        chain_id: "bitcoin".into(),
        xpub: None,
        derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
        derivation_path: None,
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: Vec::new(),
    };

    let mut state = CoreAppState::default();
    state
        .wallets
        .push(wallet("stale", "Written by an older build"));
    state.wallets.push(wallet("fresh", "Readable"));
    crate::store::wallet_db::app_state_save(&crate::wallet_db::WalletDatabase::new(&db), &state)
        .expect("save");

    // Put the pre-shrink override shape back on one row, exactly as a build
    // before the shrink left it — straight into the table, so no helper can
    // normalise it on the way in.
    let stale_payload = {
        let conn = rusqlite::Connection::open(&db).expect("open");
        let payload: String = conn
            .query_row("SELECT payload FROM wallets WHERE id = 'stale'", [], |r| {
                r.get(0)
            })
            .expect("read the row");
        let payload = payload.replace(
            r#""derivationOverrides":{"passphrase":null,"hmacKey":null}"#,
            r#""derivationOverrides":{"passphrase":null,"mnemonicWordlist":null,"hmacKey":null}"#,
        );
        conn.execute(
            "UPDATE wallets SET payload = ?1 WHERE id = 'stale'",
            rusqlite::params![payload],
        )
        .expect("write the stale row");
        payload
    };
    assert!(
        stale_payload.contains("mnemonicWordlist"),
        "the row under test must carry the field this build refuses"
    );

    // The load succeeds, and keeps everything it could read.
    let loaded =
        crate::store::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db))
            .expect("a row this build cannot read must not fail the load");
    assert_eq!(
        loaded
            .wallets
            .iter()
            .map(|w| w.id.as_str())
            .collect::<Vec<_>>(),
        ["fresh"],
    );

    // Committing on top of that load must not delete the row the load skipped:
    // it is absent from both sides of the diff, so nothing may prune it.
    let mut next = loaded.clone();
    next.wallets.push(wallet("added", "Imported afterwards"));
    crate::wallet_db::AppStateChanges::between(Some(&loaded), &next)
        .expect("diff")
        .save(&crate::wallet_db::WalletDatabase::new(&db))
        .expect("commit");

    let conn = rusqlite::Connection::open(&db).unwrap();
    let raw: String = conn
        .query_row("SELECT payload FROM wallets WHERE id = 'stale'", [], |r| {
            r.get(0)
        })
        .expect("the refused row must still be on disk");
    assert_eq!(raw, stale_payload);
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM wallets ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, ["added", "fresh", "stale"]);
}

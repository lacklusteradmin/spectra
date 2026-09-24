use crate::store::state::CoreAppState;

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
        signing: crate::store::state::WalletSigning::SeedPhrase {
            password_protected: false,
        },
        include_in_portfolio_total: true,
        chain_id: "bitcoin".into(),
        xpub: None,
        derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
        derivation_path: None,
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: Vec::new(),
    });
    crate::wallet_db::app_state_save(&crate::wallet_db::WalletDatabase::new(&db), &state)
        .expect("save");

    // Corrupt stored user preferences must refuse the load.
    {
        let conn = rusqlite::Connection::open(&db).expect("open");
        conn.execute(
            "INSERT OR REPLACE INTO app_state_meta (key, value) VALUES (?1, ?2)",
            rusqlite::params!["token_preferences", r#"[{"legacy":true}]"#],
        )
        .expect("write the bad row");
    }

    assert!(crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db)).is_err());
    let wallets =
        crate::wallet_db::wallet_load_all(&crate::wallet_db::WalletDatabase::new(&db)).unwrap();
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

/// An invalid wallet refuses the entire load and leaves every stored row intact.
#[test]
fn an_unreadable_wallet_refuses_loading_without_deleting_rows() {
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
        signing: crate::store::state::WalletSigning::SeedPhrase {
            password_protected: false,
        },
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
    crate::wallet_db::app_state_save(&crate::wallet_db::WalletDatabase::new(&db), &state)
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

    assert!(crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db)).is_err());

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
    assert_eq!(ids, ["fresh", "stale"]);
}

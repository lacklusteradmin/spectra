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
                crate::registry::Chain::Bitcoin,
                "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
                None,
                true,
            ),
        })
        .await
        .expect("upsert");
    service
        .advance_history_cursor(crate::registry::Chain::Bitcoin, wallet_id.clone(), None)
        .unwrap();
    crate::diagnostics::diagnostics_record(crate::registry::Chain::Bitcoin, row(&wallet_id));
    assert!(
        service
            .history_cursor(crate::registry::Chain::Bitcoin, wallet_id.clone())
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
            .history_cursor(crate::registry::Chain::Bitcoin, wallet_id.clone())
            .is_exhausted
    );
    assert!(
        !crate::diagnostics::diagnostics_all(crate::registry::Chain::Bitcoin)
            .contains_key(&wallet_id)
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn an_esplora_change_restarts_the_bitcoin_feed() {
    let path = db("esplora");
    let service = WalletService::new(Vec::new()).expect("service");
    service.open_state(path.clone()).await.expect("open");

    service
        .advance_history_cursor(crate::registry::Chain::Bitcoin, "w".into(), None)
        .unwrap();
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: AppSettingUpdate::AddCustomEndpoint {
                capabilities: crate::endpoint_capability_options(
                    crate::registry::Chain::Bitcoin,
                    crate::EndpointApi::Esplora,
                ),
                chain_id: crate::registry::Chain::Bitcoin,
                api: "esplora".into(),
                endpoint: "https://custom.example/api".into(),
            },
        })
        .await
        .expect("esplora");
    assert!(
        !service
            .history_cursor(crate::registry::Chain::Bitcoin, "w".into())
            .is_exhausted
    );

    // A change that is not about the feed leaves it where it was.
    service
        .advance_history_cursor(crate::registry::Chain::Bitcoin, "w".into(), None)
        .unwrap();
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: AppSettingUpdate::UsePriceAlerts { value: false },
        })
        .await
        .expect("unrelated");
    assert!(
        service
            .history_cursor(crate::registry::Chain::Bitcoin, "w".into())
            .is_exhausted
    );
    let _ = std::fs::remove_file(&path);
}

/// A history run writes its own rows and the chain's health, on every path
/// including the scheduled refresh.
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
    let rows = crate::diagnostics::diagnostics_all(crate::registry::Chain::Litecoin);
    assert_eq!(
        rows.get(&wallet_id).map(|row| row.transaction_count),
        Some(3)
    );
    let state = service.diagnostic_state().await;
    assert_eq!(
        state.degraded.get(&crate::registry::Chain::Litecoin),
        Some(&crate::service::ChainDegradation::HistoryRefreshFailed)
    );

    service
        .record_history_run(Chain::Litecoin, &Ok(outcome(1, 0)))
        .await;
    let state = service.diagnostic_state().await;
    assert!(
        !state
            .degraded
            .contains_key(&crate::registry::Chain::Litecoin)
    );
    assert!(
        state
            .last_good_unix
            .contains_key(&crate::registry::Chain::Litecoin)
    );
    let _ = std::fs::remove_file(&path);
}

/// A network's last custom endpoint gone, using only custom endpoints there
/// goes too: it would leave the network nothing to ask.
#[tokio::test]
async fn removing_the_last_custom_endpoint_stops_using_only_custom_ones() {
    let service = crate::service::WalletService::new(Vec::new()).unwrap();
    let chain = crate::registry::Chain::Bitcoin;
    for endpoint in ["https://one.example/api", "https://two.example/api"] {
        service
            .apply_state_command(StateCommand::SetAppSetting {
                update: AppSettingUpdate::AddCustomEndpoint {
                    capabilities: crate::endpoint_capability_options(
                        chain,
                        crate::EndpointApi::Esplora,
                    ),
                    chain_id: chain,
                    api: "esplora".into(),
                    endpoint: endpoint.into(),
                },
            })
            .await
            .unwrap();
    }
    let set = |update| StateCommand::SetAppSetting { update };
    service
        .apply_state_command(set(AppSettingUpdate::CustomEndpointsOnly {
            chain_id: chain,
            value: true,
        }))
        .await
        .unwrap();
    let remove = |endpoint: &str| {
        set(AppSettingUpdate::RemoveCustomEndpoint {
            chain_id: chain,
            endpoint: endpoint.into(),
        })
    };
    let state = service
        .apply_state_command(remove("https://one.example/api"))
        .await
        .unwrap()
        .state;
    assert_eq!(state.settings.custom_endpoints.len(), 1);
    assert_eq!(
        state.settings.custom_endpoints_only,
        vec![chain],
        "one is left to ask"
    );
    let state = service
        .apply_state_command(remove("https://two.example/api"))
        .await
        .unwrap()
        .state;
    assert!(state.settings.custom_endpoints.is_empty());
    assert!(
        state.settings.custom_endpoints_only.is_empty(),
        "none is left to ask"
    );
    let unchanged = service
        .apply_state_command(remove("https://two.example/api"))
        .await
        .unwrap();
    assert!(
        unchanged.events.is_empty(),
        "removing what is not there changes nothing"
    );
}

/// An edit changes the endpoint where it stands, and a network using only
/// the user's endpoints keeps doing so: the edit is never a removal of its
/// only one. An edit into another saved endpoint is refused.
#[tokio::test]
async fn editing_a_custom_endpoint_keeps_it_and_using_only_custom_ones() {
    let service = crate::service::WalletService::new(Vec::new()).unwrap();
    let chain = crate::registry::Chain::Bitcoin;
    let set = |update| StateCommand::SetAppSetting { update };
    let capabilities = crate::endpoint_capability_options(chain, crate::EndpointApi::Esplora);
    for endpoint in ["https://one.example/api", "https://two.example/api"] {
        service
            .apply_state_command(set(AppSettingUpdate::AddCustomEndpoint {
                capabilities: capabilities.clone(),
                chain_id: chain,
                api: "esplora".into(),
                endpoint: endpoint.into(),
            }))
            .await
            .unwrap();
    }
    service
        .apply_state_command(set(AppSettingUpdate::CustomEndpointsOnly {
            chain_id: chain,
            value: true,
        }))
        .await
        .unwrap();
    let replace = |from: &str, to: &str| {
        set(AppSettingUpdate::ReplaceCustomEndpoint {
            chain_id: chain,
            endpoint: from.into(),
            api: "esplora".into(),
            new_endpoint: to.into(),
            capabilities: capabilities.clone(),
        })
    };
    let state = service
        .apply_state_command(replace(
            "https://two.example/api",
            "https://three.example/api",
        ))
        .await
        .unwrap()
        .state;
    let endpoints: Vec<_> = state
        .settings
        .custom_endpoints
        .iter()
        .map(|e| e.endpoint.as_str())
        .collect();
    assert_eq!(
        endpoints,
        ["https://three.example/api", "https://one.example/api"]
    );
    assert_eq!(state.settings.custom_endpoints_only, vec![chain]);

    let refused = service
        .apply_state_command(replace(
            "https://three.example/api",
            "https://one.example/api",
        ))
        .await
        .unwrap();
    assert!(
        refused
            .events
            .contains(&crate::store::state::StateEvent::AppSettingRejected)
    );
    let unknown = service
        .apply_state_command(replace(
            "https://nowhere.example/api",
            "https://four.example/api",
        ))
        .await
        .unwrap();
    assert!(
        unknown
            .events
            .contains(&crate::store::state::StateEvent::AppSettingRejected)
    );
}

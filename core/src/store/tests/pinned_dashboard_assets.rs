use crate::service::WalletService;
use crate::state::StateCommand;

#[tokio::test]
async fn token_ids_are_trimmed_and_deduplicated_in_pin_order() {
    let service = WalletService::new(Vec::new()).expect("service");
    let transition = service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec![
                " ethereum ".into(),
                "bitcoin".into(),
                "ethereum".into(),
                "".into(),
                "solana".into(),
            ],
        })
        .await
        .expect("apply");
    assert_eq!(
        transition.state.settings.pinned_dashboard_token_ids,
        vec![
            "ethereum".to_string(),
            "bitcoin".to_string(),
            "solana".to_string()
        ]
    );
}

#[tokio::test]
async fn setting_the_same_pins_emits_nothing() {
    let service = WalletService::new(Vec::new()).expect("service");
    let command = || StateCommand::SetPinnedDashboardAssets {
        token_ids: vec!["bitcoin".into()],
    };
    let first = service.apply_state_command(command()).await.expect("apply");
    assert_eq!(first.events.len(), 1);
    let second = service.apply_state_command(command()).await.expect("apply");
    assert!(
        second.events.is_empty(),
        "re-pinning the same set is a no-op"
    );
}

#[tokio::test]
async fn unpinning_every_asset_stays_empty_after_reopening_and_reset_restores_defaults() {
    let path = std::env::temp_dir()
        .join(format!("pins-{}.sqlite", crate::store::new_event_id()))
        .to_string_lossy()
        .into_owned();
    let service = WalletService::new(Vec::new()).expect("service");
    let state = service.open_state(path.clone()).await.expect("open");
    let defaults = vec!["bitcoin", "ethereum", "tether", "usd-coin"];
    assert_eq!(state.settings.pinned_dashboard_assets(), defaults);
    for token_id in &defaults {
        let transition = service
            .apply_state_command(StateCommand::SetDashboardAssetPinned {
                token_id: (*token_id).into(),
                is_pinned: false,
            })
            .await
            .expect("unpin");
        assert_eq!(transition.events.len(), 1);
    }
    let reopened = WalletService::new(Vec::new()).expect("service");
    let state = reopened.open_state(path).await.expect("reopen");
    assert!(state.settings.pinned_dashboard_assets().is_empty());
    assert!(reopened
        .dashboard_pin_options()
        .await
        .expect("options")
        .iter()
        .all(|option| !option.is_pinned));
    assert!(reopened
        .portfolio_snapshot()
        .await
        .expect("portfolio")
        .groups
        .is_empty());

    let reset = reopened
        .apply_state_command(StateCommand::ResetPinnedDashboardAssets)
        .await
        .expect("reset");
    assert_eq!(reset.state.settings.pinned_dashboard_assets(), defaults);
    assert_eq!(reset.events.len(), 1);
    let cleared = reopened
        .apply_state_command(StateCommand::SetPinnedDashboardAssets { token_ids: vec![] })
        .await
        .expect("clear");
    assert!(cleared.state.settings.pinned_dashboard_assets().is_empty());
    assert_eq!(cleared.events.len(), 1);
    let unchanged = reopened
        .apply_state_command(StateCommand::SetPinnedDashboardAssets { token_ids: vec![] })
        .await
        .expect("clear again");
    assert!(unchanged.events.is_empty());
    let reset = reopened
        .reset_data(vec![crate::state::ResetScope::DashboardCustomization])
        .await
        .expect("reset dashboard");
    assert_eq!(reset.state.settings.pinned_dashboard_assets(), defaults);
}

/// Pinning or unpinning one asset starts from the saved selection and each
/// option says whether it is pinned now.
#[tokio::test]
async fn one_asset_is_pinned_against_the_set_the_dashboard_shows() {
    let service = WalletService::new(Vec::new()).expect("service");
    async fn stored_pins(service: &WalletService) -> Vec<String> {
        service
            .app_state()
            .await
            .settings
            .pinned_dashboard_token_ids
    }
    let is_pinned = |options: &[crate::store::wallet_domain::CoreDashboardPinOption], id: &str| {
        options
            .iter()
            .find(|option| option.token_id == id)
            .map(|option| option.is_pinned)
    };
    let options = service.dashboard_pin_options().await.expect("options");
    assert_eq!(is_pinned(&options, "bitcoin"), Some(true));
    assert_eq!(is_pinned(&options, "solana"), Some(false));

    service
        .apply_state_command(StateCommand::SetDashboardAssetPinned {
            token_id: "bitcoin".into(),
            is_pinned: false,
        })
        .await
        .expect("unpin");
    assert_eq!(
        stored_pins(&service).await,
        vec!["ethereum", "tether", "usd-coin"]
    );

    service
        .apply_state_command(StateCommand::SetDashboardAssetPinned {
            token_id: " solana ".into(),
            is_pinned: true,
        })
        .await
        .expect("pin");
    assert_eq!(
        stored_pins(&service).await,
        vec!["ethereum", "tether", "usd-coin", "solana"]
    );
    let options = service.dashboard_pin_options().await.expect("options");
    assert_eq!(is_pinned(&options, "bitcoin"), Some(false));
    assert_eq!(is_pinned(&options, "solana"), Some(true));

    let refused = service
        .apply_state_command(StateCommand::SetDashboardAssetPinned {
            token_id: "no-such-token".into(),
            is_pinned: true,
        })
        .await;
    assert!(refused.is_err(), "an unknown asset is not pinned");
}

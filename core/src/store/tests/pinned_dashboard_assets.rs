use crate::service::WalletService;
use crate::state::StateCommand;

async fn pins(service: &WalletService) -> Vec<String> {
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets { token_ids: vec![] })
        .await
        .expect("read")
        .state
        .settings
        .pinned_dashboard_token_ids
}

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

/// The four iOS used to keep to itself, so its pin cards and core's
/// grouping disagreed about what a fresh wallet pins.
#[tokio::test]
async fn an_unpinned_dashboard_reads_as_the_default_four() {
    let service = WalletService::new(Vec::new()).expect("service");
    let settings = service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets { token_ids: vec![] })
        .await
        .expect("read")
        .state
        .settings;
    assert!(settings.pinned_dashboard_token_ids.is_empty());
    assert_eq!(
        settings.pinned_dashboard_assets(),
        vec!["bitcoin", "ethereum", "tether", "usd-coin"]
    );

    let chosen = service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["solana".into()],
        })
        .await
        .expect("apply")
        .state
        .settings;
    assert_eq!(chosen.pinned_dashboard_assets(), vec!["solana".to_string()]);
}

#[tokio::test]
async fn clearing_pins_is_distinguishable_from_never_pinning() {
    let service = WalletService::new(Vec::new()).expect("service");
    assert!(pins(&service).await.is_empty());
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["bitcoin".into()],
        })
        .await
        .expect("apply");
    let cleared = service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets { token_ids: vec![] })
        .await
        .expect("apply");
    assert!(cleared.state.settings.pinned_dashboard_token_ids.is_empty());
    assert_eq!(cleared.events.len(), 1, "clearing is a real change");
}

/// Pinning or unpinning one asset starts from the set the dashboard shows —
/// the default four when nothing has been pinned — and each option says
/// whether it is on the dashboard now.
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

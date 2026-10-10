use crate::service::WalletService;
use crate::store::state::StateCommand;

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
    assert!(
        reopened
            .dashboard_pin_options()
            .await
            .expect("options")
            .iter()
            .all(|option| !option.is_pinned)
    );
    assert!(
        reopened
            .portfolio_snapshot()
            .await
            .expect("portfolio")
            .groups
            .is_empty()
    );

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
        .reset_data(vec![
            crate::store::state::ResetScope::DashboardCustomization,
        ])
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
    let is_pinned = |options: &[crate::store::wallet_domain::DashboardPinOption], id: &str| {
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

/// The pinned options come first, whatever their symbols, then the rest;
/// each part is ordered by symbol, then token id.
#[tokio::test]
async fn pin_options_list_the_pinned_first_then_by_symbol_and_token_id() {
    use crate::registry::Chain;
    let service = WalletService::new(Vec::new()).expect("service");
    // Two unpinned assets that share a symbol, so the token id decides.
    for chain in [Chain::Ethereum, Chain::Base] {
        service
            .apply_state_command(StateCommand::AddCustomToken {
                standard: None,
                chain_id: chain,
                symbol: "ETH".into(),
                name: "Lookalike".into(),
                contract: "0x1111111111111111111111111111111111111111".into(),
                coingecko_id: String::new(),
                coinpaprika_id: String::new(),
                decimals: 18,
            })
            .await
            .expect("token");
    }
    service
        .apply_state_command(StateCommand::SetPinnedDashboardAssets {
            token_ids: vec!["tether".into(), "ethereum".into()],
        })
        .await
        .expect("pin");
    let options = service.dashboard_pin_options().await.expect("options");
    let order: Vec<_> = options
        .iter()
        .map(|option| {
            (
                option.is_pinned,
                option.symbol.as_str(),
                option.token_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        &order[..2],
        [(true, "ETH", "ethereum"), (true, "USDT", "tether")]
    );
    let rest = &order[2..];
    assert!(rest.iter().all(|(pinned, _, _)| !pinned));
    // Unpinned, Bitcoin sorts before both pins by symbol and still follows them.
    assert!(rest.contains(&(false, "BTC", "bitcoin")));
    let mut sorted = rest.to_vec();
    sorted.sort_by(|a, b| (a.1, a.2).cmp(&(b.1, b.2)));
    assert_eq!(rest, sorted);
    let lookalikes: Vec<_> = rest
        .iter()
        .filter(|(_, symbol, _)| *symbol == "ETH")
        .map(|(_, _, token_id)| *token_id)
        .collect();
    assert_eq!(
        lookalikes,
        [
            "custom:base:erc-20:0x1111111111111111111111111111111111111111",
            "custom:ethereum:erc-20:0x1111111111111111111111111111111111111111"
        ]
    );
}

/// A pin names a token id. A ticker such as `ETH`, which several assets
/// share, and a test network's coin, which has no dashboard row, are refused
/// by name, and the saved pins stay as they were.
#[tokio::test]
async fn a_ticker_or_a_testnet_coin_is_not_pinned_and_the_saved_pins_stay() {
    use crate::registry::Chain;
    let service = WalletService::new(Vec::new()).expect("service");
    let before = service.app_state().await.settings.pinned_dashboard_assets();
    let testnet = crate::tokens::list_token_deployments(Some(Chain::EthereumSepolia))
        .into_iter()
        .find(|token| token.is_native())
        .expect("Sepolia's coin")
        .token_id;
    for refused in ["ETH".to_string(), testnet] {
        let error = service
            .apply_state_command(StateCommand::SetPinnedDashboardAssets {
                token_ids: vec!["tether".into(), refused.clone()],
            })
            .await
            .expect_err("not a pinnable token id");
        assert!(
            matches!(error, crate::SpectraBridgeError::InvalidInput { .. }),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!("unknown or unpinnable token ID: {refused}")
        );
        assert_eq!(
            service.app_state().await.settings.pinned_dashboard_assets(),
            before
        );
    }
}

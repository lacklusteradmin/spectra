//! Portfolio movement compares complete, core-owned observations across restarts.
use super::*;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioMovementBaseline {
    pub composition: Vec<String>,
    pub total_usd: f64,
}

fn observe(state: &CoreAppState) -> Option<PortfolioMovementBaseline> {
    let mut composition = Vec::new();
    let mut total_usd = 0.0;
    for wallet in state
        .wallets
        .iter()
        .filter(|w| w.include_in_portfolio_total)
    {
        for holding in &wallet.holdings {
            let chain = holding.chain()?;
            if chain.is_testnet() {
                continue;
            }
            crate::decimal::canonical(&holding.amount)?;
            composition.push(serde_json::to_string(&(&wallet.id, holding.deployment_id())).ok()?);
            if crate::decimal::is_zero(&holding.amount) {
                continue;
            }
            total_usd += super::valuation::value(state, holding)?;
        }
    }
    composition.sort();
    if composition.is_empty() || !total_usd.is_finite() {
        return None;
    }
    Some(PortfolioMovementBaseline {
        composition,
        total_usd,
    })
}

fn evaluate(state: &mut CoreAppState, app_is_active: bool) -> Option<LargeMovementEvaluation> {
    if !state.settings.use_large_movement_notifications {
        state.movement_baseline = None;
        return None;
    }
    let current = observe(state);
    let previous = std::mem::replace(&mut state.movement_baseline, current.clone());
    let (previous, current) = (previous?, current?);
    if app_is_active || previous.composition != current.composition {
        return None;
    }
    let mut result = evaluate_large_movement(
        previous.total_usd,
        current.total_usd,
        state.settings.large_movement_alert_usd_threshold,
        state.settings.large_movement_alert_percent_threshold,
    );
    if let Some(delta) = super::valuation::to_display(state, result.absolute_delta) {
        result.absolute_delta = delta;
        result.currency = state.settings.fiat_currency;
    }
    result.should_alert.then_some(result)
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The device supplies activity only; holdings, quotes, settings and baseline are owned.
    pub async fn evaluate_portfolio_movement(
        &self,
        app_is_active: bool,
    ) -> Result<Option<LargeMovementEvaluation>, SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let database = service.bound_database().await?;
            let before = service.wallet_state.read().await.clone();
            let mut state = before.clone();
            let result = evaluate(&mut state, app_is_active);
            if state.movement_baseline != before.movement_baseline {
                let changes = crate::wallet_db::AppStateChanges::between(Some(&before), &state)?;
                tokio::task::spawn_blocking(move || changes.save(&database))
                    .await
                    .map_err(|e| SpectraBridgeError::from(e.to_string()))??;
                service.publish_state(state).await;
            }
            Ok(result)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> CoreAppState {
        let mut s = CoreAppState::default();
        s.wallets.push(crate::store::state::WalletState {
            id: "wallet-a".into(),
            name: "A".into(),
            signing: crate::store::state::WalletSigning::WatchOnly,
            chain_id: "ethereum".into(),
            include_in_portfolio_total: true,
            xpub: None,
            derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
            derivation_path: None,
            derivation_overrides: Default::default(),
            addresses: vec![],
            holdings: vec![crate::store::wallet_domain::AssetHolding {
                id: String::new(),
                name: "Ethereum".into(),
                symbol: "ETH".into(),
                chain_id: "ethereum".into(),
                coingecko_id: "ethereum".into(),
                token_standard: "Native".into(),
                contract_address: None,
                amount: "1".into(),
            }],
        });
        s.quotes.prices.insert("ethereum:native".into(), 1000.0);
        s
    }
    #[test]
    fn movement_rebaselines_on_composition_missing_quotes_and_foreground() {
        let mut s = state();
        assert!(evaluate(&mut s, false).is_none());
        s.quotes.prices.insert("ethereum:native".into(), 1200.0);
        assert_eq!(evaluate(&mut s, false).unwrap().absolute_delta, 200.0);
        assert!(evaluate(&mut s, false).is_none());
        s.wallets[0].id = "wallet-b".into();
        s.quotes.prices.insert("ethereum:native".into(), 1500.0);
        assert!(evaluate(&mut s, false).is_none());
        s.quotes.prices.clear();
        assert!(evaluate(&mut s, false).is_none());
        assert!(s.movement_baseline.is_none());
        s.quotes.prices.insert("ethereum:native".into(), 1800.0);
        assert!(evaluate(&mut s, false).is_none());
        s.quotes.prices.insert("ethereum:native".into(), 2200.0);
        assert!(evaluate(&mut s, true).is_none());
        assert!(evaluate(&mut s, false).is_none());
        s.settings.use_large_movement_notifications = false;
        assert!(evaluate(&mut s, false).is_none());
        assert!(s.movement_baseline.is_none());
    }
    #[tokio::test]
    async fn movement_baseline_survives_reopen_and_consumes_each_change_once() {
        let path =
            std::env::temp_dir().join(format!("movement-{}.db", crate::store::new_event_id()));
        let service = WalletService::new(vec![]).unwrap();
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        service
            .mutate_persisted_state(|s| {
                *s = state();
                vec![crate::store::state::StateEvent::StateReplaced]
            })
            .await
            .unwrap();
        assert!(service
            .evaluate_portfolio_movement(false)
            .await
            .unwrap()
            .is_none());
        let reopened = WalletService::new(vec![]).unwrap();
        reopened
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        reopened
            .mutate_persisted_state(|s| {
                s.quotes.prices.insert("ethereum:native".into(), 800.0);
                vec![crate::store::state::StateEvent::StateReplaced]
            })
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            reopened.evaluate_portfolio_movement(false),
            reopened.evaluate_portfolio_movement(false)
        );
        let results: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
        assert_eq!(results.len(), 1);
        assert!(!results[0].direction_up);
        reopened
            .reset_data(vec![crate::store::state::ResetScope::HistoryAndCache])
            .await
            .unwrap();
        assert!(reopened.app_state().await.movement_baseline.is_none());
        let _ = std::fs::remove_file(path);
    }
}

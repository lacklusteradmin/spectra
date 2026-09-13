//! Alert edits are intents over the latest owned state, never list replacement.
use super::{
    state::{CoreAppState, StateEvent},
    wallet_domain::CorePriceAlertCondition,
    PriceAlertEvaluationAlert,
};

fn event(kind: &str, subject: String) -> Vec<StateEvent> {
    vec![StateEvent {
        kind: kind.into(),
        subject_id: Some(subject),
    }]
}
pub(super) fn add(
    state: &mut CoreAppState,
    key: String,
    target: f64,
    currency: String,
    condition: CorePriceAlertCondition,
) -> Vec<StateEvent> {
    let currency = currency.trim().to_uppercase();
    let rate = if currency == "USD" {
        Some(1.0)
    } else {
        state.fiat_rates_from_usd.get(&currency).copied()
    };
    let Some(rate) = rate.filter(|v| v.is_finite() && *v > 0.0) else {
        return event("priceAlertRejected", "Missing currency rate".into());
    };
    let target = target / rate;
    if !target.is_finite() || target <= 0.0 {
        return event(
            "priceAlertRejected",
            "Target must be finite and positive".into(),
        );
    }
    let metadata = state
        .wallets
        .iter()
        .flat_map(|w| &w.holdings)
        .find(|h| h.deployment_key() == key)
        .map(|h| (h.name.clone(), h.symbol.clone(), h.chain_name.clone()))
        .or_else(|| {
            crate::registry::Chain::all()
                .find(|c| c.entry().native_deployment_id == key)
                .map(|c| {
                    (
                        c.coin_name().into(),
                        c.coin_symbol().into(),
                        c.chain_display_name().into(),
                    )
                })
        });
    let Some((asset_display_name, symbol, chain_name)) = metadata else {
        return event("priceAlertRejected", "Unknown asset".into());
    };
    if state
        .price_alerts
        .iter()
        .any(|a| a.holding_key == key && a.condition == condition && a.target_price == target)
    {
        return event(
            "priceAlertRejected",
            "An identical alert already exists".into(),
        );
    }
    let id = super::new_event_id();
    state.price_alerts.insert(
        0,
        PriceAlertEvaluationAlert {
            id: id.clone(),
            holding_key: key,
            asset_display_name,
            symbol,
            chain_name,
            target_price: target,
            condition,
            is_enabled: true,
            has_triggered: false,
        },
    );
    event("priceAlertAdded", id)
}
pub(super) fn toggle(state: &mut CoreAppState, id: String) -> Vec<StateEvent> {
    let Some(alert) = state.price_alerts.iter_mut().find(|a| a.id == id) else {
        return event("priceAlertRejected", "Alert not found".into());
    };
    alert.is_enabled = !alert.is_enabled;
    if !alert.is_enabled {
        alert.has_triggered = false;
    }
    event("priceAlertChanged", id)
}
pub(super) fn remove(state: &mut CoreAppState, id: String) -> Vec<StateEvent> {
    let before = state.price_alerts.len();
    state.price_alerts.retain(|a| a.id != id);
    if state.price_alerts.len() == before {
        event("priceAlertRejected", "Alert not found".into())
    } else {
        event("priceAlertRemoved", id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alert_intents_preserve_other_trigger_state_and_convert_owned_rates() {
        let mut state = CoreAppState::default();
        state.fiat_rates_from_usd.insert("EUR".into(), 0.8);
        assert_eq!(
            add(
                &mut state,
                "ethereum:native".into(),
                0.000008,
                "EUR".into(),
                CorePriceAlertCondition::Above
            )[0]
            .kind,
            "priceAlertAdded"
        );
        let first = state.price_alerts[0].id.clone();
        assert!((state.price_alerts[0].target_price - 0.00001).abs() < 1e-20);
        state.price_alerts[0].has_triggered = true;
        add(
            &mut state,
            "bitcoin:native".into(),
            100.0,
            "USD".into(),
            CorePriceAlertCondition::Below,
        );
        let second = state.price_alerts[0].id.clone();
        toggle(&mut state, second.clone());
        assert!(
            state
                .price_alerts
                .iter()
                .find(|a| a.id == first)
                .unwrap()
                .has_triggered
        );
        remove(&mut state, second);
        assert!(state.price_alerts[0].has_triggered);
        toggle(&mut state, first);
        assert!(!state.price_alerts[0].has_triggered);
        assert!(!state.price_alerts[0].is_enabled);
        let before = state.price_alerts.clone();
        assert_eq!(
            remove(&mut state, "missing".into())[0].kind,
            "priceAlertRejected"
        );
        assert_eq!(state.price_alerts, before);
    }
}

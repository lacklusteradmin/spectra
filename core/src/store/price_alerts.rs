//! Alert edits are intents over the latest owned state, never list replacement.
use super::{
    state::{CoreAppState, StateEvent},
    wallet_domain::CorePriceAlertCondition,
    PriceAlertEvaluationAlert,
};

/// Typed reasons for refused alert edits; front ends localize them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum PriceAlertRejection {
    /// A target in a display currency core holds no rate for.
    MissingCurrencyRate,
    /// A target that is zero, negative or not a number.
    InvalidTarget,
    /// A holding core cannot name.
    UnknownAsset,
    /// The same asset, condition and target already has an alert.
    DuplicateAlert,
    /// The alert being changed no longer exists.
    AlertNotFound,
}

fn rejected(reason: PriceAlertRejection) -> Vec<StateEvent> {
    vec![StateEvent::PriceAlertRejected { reason }]
}
pub(super) fn add(
    state: &mut CoreAppState,
    key: String,
    target: f64,
    currency: super::state::FiatCurrency,
    condition: CorePriceAlertCondition,
) -> Vec<StateEvent> {
    let rate = if currency == crate::store::state::FiatCurrency::Usd {
        Some(1.0)
    } else {
        state.fiat_rates_from_usd.get(currency.code()).copied()
    };
    let Some(rate) = rate.filter(|v| v.is_finite() && *v > 0.0) else {
        return rejected(PriceAlertRejection::MissingCurrencyRate);
    };
    let target = target / rate;
    if !target.is_finite() || target <= 0.0 {
        return rejected(PriceAlertRejection::InvalidTarget);
    }
    let metadata = state
        .wallets
        .iter()
        .flat_map(|w| &w.holdings)
        .find(|h| h.deployment_id() == key)
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
        return rejected(PriceAlertRejection::UnknownAsset);
    };
    if state
        .price_alerts
        .iter()
        .any(|a| a.holding_key == key && a.condition == condition && a.target_price == target)
    {
        return rejected(PriceAlertRejection::DuplicateAlert);
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
    vec![StateEvent::PriceAlertAdded { id }]
}
pub(super) fn toggle(state: &mut CoreAppState, id: String) -> Vec<StateEvent> {
    let Some(alert) = state.price_alerts.iter_mut().find(|a| a.id == id) else {
        return rejected(PriceAlertRejection::AlertNotFound);
    };
    alert.is_enabled = !alert.is_enabled;
    if !alert.is_enabled {
        alert.has_triggered = false;
    }
    vec![StateEvent::PriceAlertChanged { id }]
}
pub(super) fn remove(state: &mut CoreAppState, id: String) -> Vec<StateEvent> {
    let before = state.price_alerts.len();
    state.price_alerts.retain(|a| a.id != id);
    if state.price_alerts.len() == before {
        rejected(PriceAlertRejection::AlertNotFound)
    } else {
        vec![StateEvent::PriceAlertRemoved { id }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alert_intents_preserve_other_trigger_state_and_convert_owned_rates() {
        let mut state = CoreAppState::default();
        state.fiat_rates_from_usd.insert("EUR".into(), 0.8);
        assert!(matches!(
            add(
                &mut state,
                "ethereum:native".into(),
                0.000008,
                crate::store::state::FiatCurrency::Eur,
                CorePriceAlertCondition::Above
            )[0],
            StateEvent::PriceAlertAdded { .. }
        ));
        let first = state.price_alerts[0].id.clone();
        assert!((state.price_alerts[0].target_price - 0.00001).abs() < 1e-20);
        state.price_alerts[0].has_triggered = true;
        add(
            &mut state,
            "bitcoin:native".into(),
            100.0,
            crate::store::state::FiatCurrency::Usd,
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
        assert!(matches!(
            remove(&mut state, "missing".into())[0],
            StateEvent::PriceAlertRejected { .. }
        ));
        assert_eq!(state.price_alerts, before);
    }

    /// A refusal carries a code for the front end to word, not core's prose.
    #[test]
    fn a_refusal_names_its_reason_as_a_code() {
        let mut state = CoreAppState::default();
        let subject = |events: Vec<StateEvent>| match &events[0] {
            StateEvent::PriceAlertRejected { reason } => *reason,
            other => panic!("not a refusal: {other:?}"),
        };
        assert_eq!(
            subject(add(
                &mut state,
                "bitcoin:native".into(),
                0.0,
                crate::store::state::FiatCurrency::Usd,
                CorePriceAlertCondition::Above
            )),
            PriceAlertRejection::InvalidTarget
        );
        assert_eq!(
            subject(add(
                &mut state,
                "bitcoin:native".into(),
                1.0,
                crate::store::state::FiatCurrency::Eur,
                CorePriceAlertCondition::Above
            )),
            PriceAlertRejection::MissingCurrencyRate
        );
        assert_eq!(
            subject(add(
                &mut state,
                "nowhere:native".into(),
                1.0,
                crate::store::state::FiatCurrency::Usd,
                CorePriceAlertCondition::Above
            )),
            PriceAlertRejection::UnknownAsset
        );
        add(
            &mut state,
            "bitcoin:native".into(),
            1.0,
            crate::store::state::FiatCurrency::Usd,
            CorePriceAlertCondition::Above,
        );
        assert_eq!(
            subject(add(
                &mut state,
                "bitcoin:native".into(),
                1.0,
                crate::store::state::FiatCurrency::Usd,
                CorePriceAlertCondition::Above
            )),
            PriceAlertRejection::DuplicateAlert
        );
        assert_eq!(
            subject(toggle(&mut state, "missing".into())),
            PriceAlertRejection::AlertNotFound
        );
    }
}

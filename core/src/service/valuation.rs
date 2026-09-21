//! One valuation policy for portfolio display, dashboard ordering and movement alerts.
use super::*;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct QuotedTotal {
    pub total: f64,
    pub unpriced_count: u64,
    pub fiat_total: Option<f64>,
}

#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioValuation {
    pub currency: crate::store::state::FiatCurrency,
    pub portfolio: QuotedTotal,
    pub wallets: HashMap<String, QuotedTotal>,
}

pub(super) fn price(state: &CoreAppState, holding: &AssetHolding) -> Option<f64> {
    if holding.chain()?.is_testnet() {
        return None;
    }
    state
        .quotes
        .prices
        .get(&holding.deployment_id())
        .copied()
        .filter(|p| p.is_finite() && *p > 0.0)
}

pub(super) fn value(state: &CoreAppState, holding: &AssetHolding) -> Option<f64> {
    if !holding.amount.is_finite() || holding.amount < 0.0 {
        return None;
    }
    let value = holding.amount * price(state, holding)?;
    value.is_finite().then_some(value)
}

pub(super) fn total<'a>(
    state: &CoreAppState,
    holdings: impl Iterator<Item = &'a AssetHolding>,
) -> QuotedTotal {
    let mut total = 0.0;
    let mut unpriced_count = 0;
    for holding in holdings {
        if holding.chain().is_some_and(|chain| chain.is_testnet()) || holding.amount == 0.0 {
            continue;
        }
        match value(state, holding) {
            Some(value) if (total + value).is_finite() => total += value,
            _ => unpriced_count += 1,
        }
    }
    let rate = if state.settings.fiat_currency == crate::store::state::FiatCurrency::Usd {
        Some(1.0)
    } else {
        state
            .fiat_rates_from_usd
            .get(state.settings.fiat_currency.code())
            .copied()
            .filter(|r| r.is_finite() && *r > 0.0)
    };
    QuotedTotal {
        total,
        unpriced_count,
        fiat_total: rate.map(|rate| total * rate).filter(|v| v.is_finite()),
    }
}

pub(super) fn portfolio_valuation(state: &CoreAppState) -> PortfolioValuation {
    PortfolioValuation {
        currency: state.settings.fiat_currency,
        portfolio: total(
            state,
            state
                .wallets
                .iter()
                .filter(|w| w.include_in_portfolio_total)
                .flat_map(|w| &w.holdings),
        ),
        wallets: state
            .wallets
            .iter()
            .map(|wallet| (wallet.id.clone(), total(state, wallet.holdings.iter())))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_invalid_and_overflowed_quotes_are_not_zero_or_stored_prices() {
        let mut state = CoreAppState::default();
        let mut coin = crate::tokens::deployment("ethereum:native")
            .unwrap()
            .holding_template();
        coin.amount = 2.0;
        coin.price_usd = 999.0;
        for price in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::MAX),
        ] {
            state.quotes.prices.clear();
            if let Some(price) = price {
                state.quotes.prices.insert(coin.deployment_id(), price);
            }
            let result = total(&state, std::iter::once(&coin));
            assert_eq!(result.total, 0.0);
            assert_eq!(result.unpriced_count, 1);
        }
        state.quotes.prices.insert(coin.deployment_id(), 3000.0);
        state.settings.fiat_currency = crate::store::state::FiatCurrency::Eur;
        let result = total(&state, std::iter::once(&coin));
        assert_eq!(result.total, 6000.0);
        assert_eq!(result.fiat_total, None);
        state.fiat_rates_from_usd.insert("EUR".into(), 0.9);
        assert_eq!(
            total(&state, std::iter::once(&coin)).fiat_total,
            Some(5400.0)
        );
    }
}

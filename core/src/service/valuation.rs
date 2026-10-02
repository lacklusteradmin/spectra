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
    /// Each wallet's holdings by deployment id, in the display currency.
    /// A holding that is unpriced is absent.
    pub holding_values: HashMap<String, HashMap<String, f64>>,
    /// One unit of each held deployment, in the display currency.
    pub prices: HashMap<String, f64>,
    /// Each price alert's target, in the display currency.
    pub alert_targets: HashMap<String, f64>,
}

pub(super) fn price(state: &CoreAppState, holding: &AssetHolding) -> Option<f64> {
    if holding.chain_id.is_testnet() {
        return None;
    }
    state
        .quotes
        .prices
        .get(&holding.deployment_id())
        .copied()
        .filter(|p| p.is_finite() && *p > 0.0)
}

/// USD value of the holding's whole balance.
pub(super) fn value(state: &CoreAppState, holding: &AssetHolding) -> Option<f64> {
    value_of(state, holding, &holding.amount)
}

/// USD value of `amount` of the holding's asset.
fn value_of(state: &CoreAppState, holding: &AssetHolding, amount: &str) -> Option<f64> {
    let amount = crate::decimal::canonical(amount)?;
    let value = crate::decimal::to_f64(&amount) * price(state, holding)?;
    value.is_finite().then_some(value)
}

/// A wallet's holdings in the order it shows them: most valuable first,
/// unpriced ones after, each run by symbol. Stable, so equal rows keep
/// the catalog's order.
pub(super) fn order_holdings_by_value(state: &CoreAppState, holdings: &mut [AssetHolding]) {
    use std::cmp::Ordering;
    holdings.sort_by(|a, b| {
        match (value(state, a), value(state, b)) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
        .then_with(|| a.symbol.to_lowercase().cmp(&b.symbol.to_lowercase()))
    });
}

/// USD to the display currency, when the rate is known.
pub(super) fn display_rate(state: &CoreAppState) -> Option<f64> {
    if state.settings.fiat_currency == crate::store::state::FiatCurrency::Usd {
        return Some(1.0);
    }
    state
        .fiat_rates_from_usd
        .get(state.settings.fiat_currency.code())
        .copied()
        .filter(|r| r.is_finite() && *r > 0.0)
}

/// A USD figure in the display currency.
pub(super) fn to_display(state: &CoreAppState, usd: f64) -> Option<f64> {
    let value = usd * display_rate(state)?;
    value.is_finite().then_some(value)
}

/// `amount` of the holding's asset, in the display currency.
pub(super) fn display_value_of(
    state: &CoreAppState,
    holding: &AssetHolding,
    amount: &str,
) -> Option<f64> {
    to_display(state, value_of(state, holding, amount)?)
}

/// One unit of the holding's asset, in the display currency.
pub(super) fn display_price(state: &CoreAppState, holding: &AssetHolding) -> Option<f64> {
    to_display(state, price(state, holding)?)
}

pub(super) fn total<'a>(
    state: &CoreAppState,
    holdings: impl Iterator<Item = &'a AssetHolding>,
) -> QuotedTotal {
    let mut total = 0.0;
    let mut unpriced_count = 0;
    for holding in holdings {
        if (holding.chain_id.is_testnet()) || crate::decimal::is_zero(&holding.amount) {
            continue;
        }
        match value(state, holding) {
            Some(value) if (total + value).is_finite() => total += value,
            _ => unpriced_count += 1,
        }
    }
    let rate = display_rate(state);
    QuotedTotal {
        total,
        unpriced_count,
        fiat_total: rate.map(|rate| total * rate).filter(|v| v.is_finite()),
    }
}

pub(super) fn portfolio_valuation(state: &CoreAppState) -> PortfolioValuation {
    let holding_values = state
        .wallets
        .iter()
        .map(|wallet| {
            let values = wallet
                .holdings
                .iter()
                .filter_map(|h| Some((h.deployment_id(), to_display(state, value(state, h)?)?)))
                .collect();
            (wallet.id.clone(), values)
        })
        .collect();
    let prices = state
        .wallets
        .iter()
        .flat_map(|wallet| &wallet.holdings)
        .filter_map(|h| Some((h.deployment_id(), display_price(state, h)?)))
        .collect();
    let alert_targets = state
        .price_alerts
        .iter()
        .filter_map(|alert| Some((alert.id.clone(), to_display(state, alert.target_price)?)))
        .collect();
    PortfolioValuation {
        currency: state.settings.fiat_currency,
        holding_values,
        prices,
        alert_targets,
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
        coin.amount = "2".into();
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

    /// A testnet coin is never quoted, even when a price is keyed by its id.
    #[test]
    fn a_testnet_holding_has_no_value() {
        let mut state = CoreAppState::default();
        let mut coin = crate::registry::Chain::EthereumSepolia.native_holding_template();
        coin.amount = "1".into();
        state.quotes.prices.insert(coin.deployment_id(), 3000.0);
        assert_eq!(value(&state, &coin), None);
        assert_eq!(total(&state, std::iter::once(&coin)).total, 0.0);
    }

    /// Most valuable first, unpriced after, ties by symbol.
    #[test]
    fn a_wallet_shows_its_most_valuable_holdings_first() {
        let mut state = CoreAppState::default();
        let tokens: Vec<_> = crate::store::built_in_token_preferences()
            .into_iter()
            .filter(|p| p.token.chain_id == crate::registry::Chain::Ethereum)
            .take(2)
            .collect();
        let holding = |deployment_id: &str, amount: &str| {
            let mut coin = crate::tokens::deployment(deployment_id)
                .unwrap()
                .holding_template();
            coin.amount = amount.into();
            coin
        };
        let ether = holding("ethereum:native", "1");
        let cheap = holding(&tokens[0].token.deployment_id, "10");
        let unpriced = holding(&tokens[1].token.deployment_id, "5");
        state.quotes.prices.insert(ether.deployment_id(), 3000.0);
        state.quotes.prices.insert(cheap.deployment_id(), 1.0);
        let mut holdings = vec![unpriced.clone(), cheap.clone(), ether.clone()];
        order_holdings_by_value(&state, &mut holdings);
        let order: Vec<_> = holdings.iter().map(AssetHolding::deployment_id).collect();
        assert_eq!(
            order,
            [ether, cheap, unpriced].map(|h| h.deployment_id()).to_vec()
        );
    }
}

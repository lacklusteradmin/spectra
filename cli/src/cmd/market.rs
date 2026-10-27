//! Prices, the portfolio's value and the display currency, all as core
//! stores and values them: the CLI multiplies nothing.

use std::collections::BTreeMap;

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::amount::presentation::{AssetPrecisionCatalog, format_asset_amount};
use spectra_core::registry::Chain;
use spectra_core::service::{PortfolioValuation, QuoteRefreshState};
use spectra_core::store::state::{AppSettingUpdate, FiatCurrency, StateCommand};
use spectra_core::store::wallet_domain::{DashboardAssetGroup, DashboardPinOption};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Args)]
pub struct PriceArgs {
    /// Chain display name or registry id.
    chain: Option<String>,
    /// Refresh prices for stored holdings and price alerts.
    #[arg(long)]
    refresh: bool,
    /// Read core-owned cached quotes without network.
    #[arg(long)]
    stored: bool,
}

#[derive(Args)]
pub struct PortfolioArgs {
    /// Replace dashboard pins with these token IDs; repeat to set their order.
    #[arg(long)]
    pin_token: Vec<String>,
    /// Unpin one token ID from the set the dashboard shows (repeatable).
    #[arg(long)]
    unpin_token: Vec<String>,
    /// List core-owned pin candidates without network.
    #[arg(long)]
    pin_options: bool,
    /// Render core dashboard groups using stored balances and quotes, without network.
    #[arg(long)]
    stored: bool,
    /// Skip wallets whose balance lookup fails instead of stopping.
    #[arg(long, default_value_t = true)]
    skip_unreachable: bool,
}

#[derive(Args)]
pub struct CurrencyArgs {
    /// ISO 4217 code to switch to. Omit to print the current one.
    code: Option<String>,
    /// Fetch the cross-rates and store them (needs network).
    #[arg(long)]
    refresh_rates: bool,
    /// Print the stored cross-rates rather than the selected currency.
    #[arg(long)]
    rates: bool,
}

// ─── price ──────────────────────────────────────────────────────────────────

/// The quotes core holds, in USD by price id, and when they last moved.
#[derive(Serialize)]
struct QuotesReport {
    quotes: QuoteRefreshState,
}

impl Report for QuotesReport {
    fn render(&self) {
        let quotes = &self.quotes;
        println!();
        let prices: BTreeMap<&String, &f64> = quotes.prices.iter().collect();
        for (id, price) in prices {
            out::wide_field(
                22,
                id,
                &format!("{} USD", out::fiat(Some(*price), FiatCurrency::Usd)),
            );
        }
        if let Some(failure) = &quotes.prices_error {
            println!("  {} prices: {failure}", out::fail_mark());
        }
        if let Some(failure) = &quotes.fiat_error {
            println!("  {} fiat rates: {failure}", out::fail_mark());
        }
    }
}

/// A chain's native asset, priced now in the display currency.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpotReport {
    chain: Chain,
    symbol: String,
    price_usd: Option<f64>,
    price: Option<f64>,
    currency: String,
}

impl Report for SpotReport {
    fn render(&self) {
        let currency = FiatCurrency::from_code(&self.currency).unwrap_or(FiatCurrency::Usd);
        println!();
        println!(
            "  {}  {} {}  {}",
            out::tint("●", self.chain).bold(),
            out::fiat(self.price, currency).bold(),
            out::hint(&self.currency),
            out::tint(&self.symbol, self.chain).bold(),
        );
    }
}

pub fn price(ctx: &Ctx, out: Out, args: PriceArgs) -> CliResult<()> {
    if args.refresh || args.stored {
        let service = ctx.service()?;
        let state = if args.refresh {
            ctx.rt.block_on(service.refresh_owned_prices(true))?
        } else {
            ctx.rt.block_on(service.app_state())
        };
        return out.report(&QuotesReport {
            quotes: state.quotes,
        });
    }
    let chain = resolve_chain(
        args.chain
            .as_deref()
            .ok_or_else(|| CliError::usage("specify a chain, --stored or --refresh"))?,
    )?;
    // Core fetches the quote and converts it with its own stored rate.
    let quote = ctx.rt.block_on(ctx.service()?.native_spot_price(chain))?;
    out.report(&SpotReport {
        chain: quote.chain_id,
        symbol: quote.symbol,
        price_usd: quote.price_usd,
        price: quote.price,
        currency: quote.currency,
    })
}

// ─── portfolio ──────────────────────────────────────────────────────────────

/// The assets the dashboard can pin, and which it does.
#[derive(Serialize)]
struct PinOptionsReport {
    options: Vec<DashboardPinOption>,
}

impl Report for PinOptionsReport {
    fn render(&self) {
        println!();
        for option in &self.options {
            println!(
                "  {}  {:<8} {}  {}",
                if option.is_pinned {
                    out::accent("●").bold()
                } else {
                    out::hint("○")
                },
                option.symbol.bold(),
                option.name,
                out::hint(&option.token_id)
            );
        }
    }
}

/// The dashboard's groups as core builds them from stored balances and
/// quotes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredPortfolioReport {
    groups: Vec<DashboardAssetGroup>,
    valuation: PortfolioValuation,
    revision: u64,
    asset_precision: AssetPrecisionCatalog,
}

impl Report for StoredPortfolioReport {
    fn render(&self) {
        let currency = self.valuation.currency;
        println!();
        for group in &self.groups {
            println!(
                "  {}  {:>18} {:<8} {}",
                if group.is_pinned {
                    out::accent("●").bold()
                } else {
                    out::hint("·")
                },
                group.total_amount,
                group.identity.symbol.bold(),
                out::fiat(group.total_value, currency)
            );
        }
        println!();
        render_total(&self.valuation);
    }
}

fn render_total(valuation: &PortfolioValuation) {
    let unpriced = valuation.portfolio.unpriced_count;
    println!(
        "  {}  {} {}{}",
        out::accent("Σ").bold(),
        out::fiat(valuation.portfolio.fiat_total, valuation.currency).bold(),
        out::hint(valuation.currency.code()),
        if unpriced > 0 {
            out::hint(&format!("  · {unpriced} without a price")).to_string()
        } else {
            String::new()
        },
    );
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HoldingValue {
    deployment_id: String,
    symbol: String,
    amount: String,
    value: Option<f64>,
    /// The amount as a row shows it, cut to the asset's display places.
    #[serde(skip)]
    shown: String,
}

#[derive(Serialize)]
struct WalletValue {
    wallet: String,
    chain: Chain,
    total: Option<f64>,
    holdings: Vec<HoldingValue>,
    #[serde(skip)]
    name: String,
    #[serde(skip)]
    is_watch_only: bool,
}

#[derive(Serialize)]
struct Unavailable {
    wallet: String,
    error: String,
    #[serde(skip)]
    name: String,
}

/// The portfolio valued now: balances, prices and the display currency's
/// rate refreshed, then valued by core.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortfolioReport {
    #[serde(skip)]
    fiat: FiatCurrency,
    currency: &'static str,
    total: Option<f64>,
    unpriced_count: u64,
    wallets: Vec<WalletValue>,
    unavailable: Vec<Unavailable>,
    failures: Vec<String>,
}

impl Report for PortfolioReport {
    fn render(&self) {
        println!();
        if self.wallets.is_empty() && self.unavailable.is_empty() {
            println!("  {}", out::hint("no wallets"));
            return;
        }
        for wallet in &self.wallets {
            for holding in &wallet.holdings {
                println!(
                    "  {}  {:<14}  {:>18}  {}",
                    out::wallet_dot(wallet.chain, wallet.is_watch_only),
                    wallet.name,
                    format!("{} {}", holding.shown, holding.symbol),
                    out::fiat(holding.value, self.fiat).bold(),
                );
            }
        }
        for unavailable in &self.unavailable {
            println!(
                "  {}  {:<14}  {}",
                out::fail_mark(),
                unavailable.name,
                out::hint(&format!("unavailable — {}", unavailable.error)),
            );
        }
        println!();
        let unpriced = self.unpriced_count;
        println!(
            "  {}  {} {}{}",
            out::accent("Σ").bold(),
            out::fiat(self.total, self.fiat).bold(),
            out::hint(self.currency),
            if unpriced > 0 {
                out::hint(&format!("  · {unpriced} without a price")).to_string()
            } else {
                String::new()
            },
        );
        for failure in &self.failures {
            println!("  {} {}", out::fail_mark(), out::hint(failure));
        }
    }
}

pub fn portfolio(ctx: &Ctx, out: Out, args: PortfolioArgs) -> CliResult<()> {
    if !args.pin_token.is_empty() {
        ctx.apply(StateCommand::SetPinnedDashboardAssets {
            token_ids: args.pin_token,
        })?;
    }
    for token_id in args.unpin_token {
        ctx.apply(StateCommand::SetDashboardAssetPinned {
            token_id,
            is_pinned: false,
        })?;
    }
    if args.pin_options {
        let options = ctx.rt.block_on(ctx.service()?.dashboard_pin_options())?;
        return out.report(&PinOptionsReport { options });
    }
    if args.stored {
        let snapshot = ctx.rt.block_on(ctx.service()?.portfolio_snapshot())?;
        return out.report(&StoredPortfolioReport {
            groups: snapshot.groups,
            valuation: snapshot.valuation,
            revision: snapshot.revision,
            asset_precision: snapshot.asset_precision,
        });
    }

    // Live means core refreshes what it values from — balances, prices and
    // the display currency's rate — and then values it.
    let service = ctx.service()?;
    let wallets = ctx.rt.block_on(service.app_state()).wallets;
    let mut unavailable = Vec::new();
    for wallet in &wallets {
        if let Err(error) = ctx
            .rt
            .block_on(service.refresh_wallet_balances(wallet.id.clone()))
        {
            if !args.skip_unreachable {
                return Err(error.into());
            }
            unavailable.push(Unavailable {
                wallet: wallet.id.clone(),
                error: error.to_string(),
                name: wallet.name.clone(),
            });
        }
    }
    let mut failures = Vec::new();
    if !wallets.is_empty() {
        let quotes = ctx.rt.block_on(service.refresh_owned_prices(false))?.quotes;
        failures.extend(
            quotes
                .prices_error
                .map(|failure| format!("prices: {failure}")),
        );
        let quotes = ctx
            .rt
            .block_on(service.refresh_owned_fiat_rates(false))?
            .quotes;
        failures.extend(
            quotes
                .fiat_error
                .map(|failure| format!("fiat rates: {failure}")),
        );
    }
    let snapshot = ctx.rt.block_on(service.portfolio_snapshot())?;
    let valuation = &snapshot.valuation;
    let precision = &snapshot.asset_precision;
    out.report(&PortfolioReport {
        fiat: valuation.currency,
        currency: valuation.currency.code(),
        total: valuation.portfolio.fiat_total,
        unpriced_count: valuation.portfolio.unpriced_count,
        wallets: snapshot
            .wallets
            .iter()
            .map(|wallet| {
                let values = valuation.holding_values.get(&wallet.id);
                WalletValue {
                    wallet: wallet.id.clone(),
                    chain: wallet.chain_id,
                    total: valuation.wallets.get(&wallet.id).and_then(|t| t.fiat_total),
                    holdings: wallet
                        .holdings
                        .iter()
                        .filter(|h| !spectra_core::amount::decimal::is_zero(&h.amount))
                        .map(|holding| {
                            let decimals = precision
                                .by_deployment_id
                                .get(&holding.id)
                                .copied()
                                .unwrap_or(precision.unknown_decimals);
                            let shown = format_asset_amount(holding.amount.clone(), decimals)
                                .map(|text| {
                                    let below = if text.below_threshold { "<" } else { "" };
                                    format!("{below}{}", text.value)
                                })
                                .unwrap_or_else(|| holding.amount.clone());
                            HoldingValue {
                                deployment_id: holding.id.clone(),
                                symbol: holding.symbol.clone(),
                                amount: holding.amount.clone(),
                                value: values.and_then(|v| v.get(&holding.id)).copied(),
                                shown,
                            }
                        })
                        .collect(),
                    name: wallet.name.clone(),
                    is_watch_only: wallet.signing.is_watch_only(),
                }
            })
            .collect(),
        unavailable,
        failures,
    })
}

// ─── currency ───────────────────────────────────────────────────────────────

/// The display currency, and the one it replaced when it changed.
#[derive(Serialize)]
struct CurrencyReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<&'static str>,
    currency: &'static str,
}

impl Report for CurrencyReport {
    fn render(&self) {
        match self.from {
            None => out::field("currency", &self.currency.bold().to_string()),
            Some(from) if from == self.currency => {
                println!("  {} already {}", out::hint("·"), self.currency.bold())
            }
            Some(from) => println!(
                "  {} {} {} {}",
                out::ok_mark(),
                out::hint(from),
                out::hint("→"),
                self.currency.bold()
            ),
        }
    }
}

pub fn currency(ctx: &Ctx, out: Out, args: CurrencyArgs) -> CliResult<()> {
    if args.refresh_rates || args.rates {
        return rates(ctx, out, args.refresh_rates);
    }
    let current = ctx.state()?.settings.fiat_currency.code();
    let Some(requested) = args.code else {
        return out.report(&CurrencyReport {
            from: None,
            currency: current,
        });
    };
    // Reject currency codes absent from the rate table.
    let Some(currency) = FiatCurrency::from_code(&requested) else {
        return Err(CliError::rejected(format!(
            "{requested:?} is not a currency this app quotes in"
        )));
    };
    let transition = ctx.apply(StateCommand::SetAppSetting {
        update: AppSettingUpdate::FiatCurrency { value: currency },
    })?;
    out.report(&CurrencyReport {
        from: Some(current),
        currency: transition.state.settings.fiat_currency.code(),
    })
}

/// The stored USD cross-rates.
#[derive(Serialize)]
struct RatesReport {
    base: &'static str,
    count: usize,
    rates: BTreeMap<String, f64>,
}

impl Report for RatesReport {
    fn render(&self) {
        println!();
        if self.rates.is_empty() {
            println!(
                "  {}",
                out::hint("no rates stored — run with --refresh-rates")
            );
            return;
        }
        for (code, rate) in &self.rates {
            out::field(code, &rate.to_string());
        }
        println!();
        println!("  {}", out::hint("per 1 USD"));
    }
}

/// The rates are core's state, so this reads them from the same store the
/// app does, optionally refreshed first.
fn rates(ctx: &Ctx, out: Out, refresh: bool) -> CliResult<()> {
    if refresh {
        ctx.rt.block_on(ctx.service()?.refresh_fiat_rates())?;
    }
    let rates: BTreeMap<String, f64> = ctx.state()?.fiat_rates_from_usd.into_iter().collect();
    out.report(&RatesReport {
        base: "USD",
        count: rates.len(),
        rates,
    })
}

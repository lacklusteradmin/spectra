//! The display currency is domain state shared with the app: setting it here
//! changes what the phone shows.

use clap::Args;
use colored::Colorize as _;
use spectra_core::store::state::{FiatCurrency, StateCommand};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Args)]
pub struct PriceArgs {
    /// Chain display name, registry id or symbol.
    chain: Option<String>,
    /// Refresh prices for stored holdings and dashboard pins.
    #[arg(long)]
    refresh: bool,
    /// Read core-owned cached quotes without network.
    #[arg(long)]
    stored: bool,
}

#[derive(Args)]
pub struct PortfolioArgs {
    /// Pin exact token IDs (repeat for multiple tokens).
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

pub fn price(ctx: &Ctx, out: Out, args: PriceArgs) -> CliResult<()> {
    if args.refresh || args.stored {
        let service = ctx.service()?;
        let state = if args.refresh {
            ctx.rt.block_on(service.refresh_owned_prices(true))?
        } else {
            ctx.rt.block_on(service.app_state())
        };
        out.text(|| println!("{:?}", state.quotes.prices));
        out.emit(serde_json::json!({"quotes":state.quotes}));
        return Ok(());
    }
    let chain = resolve_chain(
        args.chain
            .as_deref()
            .ok_or_else(|| CliError::usage("specify a chain, --stored or --refresh"))?,
    )?;
    // Core fetches the quote and converts it with its own stored rate.
    let quote = ctx
        .rt
        .block_on(ctx.service()?.native_spot_price(chain.str_id().into()))?;

    out.text(|| {
        println!();
        let price = quote
            .price
            .map(|price| format!("{price:.2}"))
            .unwrap_or_else(|| "—".into());
        println!(
            "  {}  {} {}  {}",
            out::tint("●", chain.str_id()).bold(),
            price.bold(),
            out::hint(&quote.currency),
            out::tint(chain.coin_symbol(), chain.str_id()).bold(),
        );
    });
    let mut json = serde_json::to_value(&quote).map_err(|e| CliError::failure(e.to_string()))?;
    json["ok"] = true.into();
    out.emit(json);
    Ok(())
}

pub fn portfolio(ctx: &Ctx, out: Out, args: PortfolioArgs) -> CliResult<()> {
    if !args.pin_token.is_empty() {
        ctx.rt.block_on(ctx.service()?.apply_state_command(
            spectra_core::store::state::StateCommand::SetPinnedDashboardAssets {
                token_ids: args.pin_token,
            },
        ))?;
    }
    for token_id in args.unpin_token {
        ctx.rt.block_on(ctx.service()?.apply_state_command(
            spectra_core::store::state::StateCommand::SetDashboardAssetPinned {
                token_id,
                is_pinned: false,
            },
        ))?;
    }
    if args.pin_options {
        let options = ctx.rt.block_on(ctx.service()?.dashboard_pin_options())?;
        out.text(|| println!("{options:?}"));
        out.emit(serde_json::json!({"options":options}));
        return Ok(());
    }
    if args.stored {
        let snapshot = ctx.rt.block_on(ctx.service()?.portfolio_snapshot())?;
        out.text(|| println!("{:?}", snapshot.valuation));
        out.emit(serde_json::json!({"groups":snapshot.groups,"valuation":snapshot.valuation,"revision":snapshot.revision,"assetPrecision":snapshot.asset_precision}));
        return Ok(());
    }
    let service = ctx.service()?;
    let wallets = ctx.rt.block_on(service.app_state()).wallets;
    if wallets.is_empty() {
        out.text(|| println!("  {}", out::hint("no wallets")));
        out.emit(serde_json::json!({ "ok": true, "total": 0.0, "wallets": [] }));
        return Ok(());
    }

    // Live means core refreshes what it values from — balances, prices and
    // the display currency's rate — and then values it. The CLI multiplies
    // nothing.
    let mut unavailable = Vec::new();
    for wallet in &wallets {
        if let Err(error) = ctx
            .rt
            .block_on(service.refresh_wallet_balances(wallet.id.clone()))
        {
            if !args.skip_unreachable {
                return Err(error.into());
            }
            unavailable.push(serde_json::json!({"wallet": wallet.id, "error": error.to_string()}));
            out.text(|| {
                println!(
                    "  {}  {:<14}  {}",
                    out::wallet_dot(&wallet.chain_id, wallet.is_watch_only()),
                    wallet.name,
                    out::hint(&format!("unavailable — {error}")),
                )
            });
        }
    }
    let mut failures = Vec::new();
    let quotes = ctx.rt.block_on(service.refresh_owned_prices(false))?.quotes;
    failures.extend(quotes.prices_error);
    let quotes = ctx
        .rt
        .block_on(service.refresh_owned_fiat_rates(false))?
        .quotes;
    failures.extend(quotes.fiat_error);
    let snapshot = ctx.rt.block_on(service.portfolio_snapshot())?;
    let valuation = &snapshot.valuation;
    let code = valuation.currency.code();
    let fiat = |value: Option<f64>| {
        value
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "—".into())
    };

    out.text(|| {
        println!();
        for wallet in &snapshot.wallets {
            let values = valuation.holding_values.get(&wallet.id);
            for holding in wallet
                .holdings
                .iter()
                .filter(|h| !spectra_core::decimal::is_zero(&h.amount))
            {
                let decimals = snapshot
                    .asset_precision
                    .by_deployment_id
                    .get(&holding.id)
                    .copied()
                    .unwrap_or(snapshot.asset_precision.unknown_decimals);
                let amount =
                    spectra_core::formatting::format_asset_amount(holding.amount.clone(), decimals)
                        .map(|text| {
                            format!(
                                "{}{}",
                                if text.below_threshold { "<" } else { "" },
                                text.value
                            )
                        })
                        .unwrap_or_else(|| holding.amount.clone());
                println!(
                    "  {}  {:<14}  {:>14}  {}",
                    out::wallet_dot(&wallet.chain_id, wallet.signing.is_watch_only()),
                    wallet.name,
                    format!("{amount} {}", holding.symbol),
                    fiat(values.and_then(|v| v.get(&holding.id)).copied()).bold(),
                );
            }
        }
        println!();
        let unpriced = valuation.portfolio.unpriced_count;
        println!(
            "  {}  {} {}{}",
            out::accent("Σ").bold(),
            fiat(valuation.portfolio.fiat_total).bold(),
            out::hint(code),
            if unpriced > 0 {
                out::hint(&format!("  · {unpriced} without a price"))
            } else {
                out::hint("")
            },
        );
        for failure in &failures {
            println!("  {} {}", out::fail_mark(), out::hint(failure));
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "currency": code,
        "total": valuation.portfolio.fiat_total,
        "unpricedCount": valuation.portfolio.unpriced_count,
        "wallets": snapshot.wallets.iter().map(|wallet| serde_json::json!({
            "wallet": wallet.id,
            "chainId": wallet.chain_id,
            "total": valuation.wallets.get(&wallet.id).and_then(|t| t.fiat_total),
            "holdings": wallet.holdings.iter().map(|holding| serde_json::json!({
                "deploymentId": holding.id,
                "symbol": holding.symbol,
                "amount": holding.amount,
                "value": valuation.holding_values.get(&wallet.id).and_then(|v| v.get(&holding.id)),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "unavailable": unavailable,
        "failures": failures,
    }));
    Ok(())
}

pub fn currency(ctx: &Ctx, out: Out, args: CurrencyArgs) -> CliResult<()> {
    if args.refresh_rates || args.rates {
        return rates(ctx, out, args.refresh_rates);
    }
    let current = ctx.state()?.settings.fiat_currency.code();
    let Some(requested) = args.code else {
        out.text(|| {
            println!();
            out::field("currency", &current.bold().to_string());
            println!(
                "  {}",
                out::hint("shared with the app — the same setting, the same store")
            );
        });
        out.emit(serde_json::json!({ "ok": true, "currency": current }));
        return Ok(());
    };

    // Reject currency codes absent from the rate table.
    let Some(currency) = FiatCurrency::from_code(&requested) else {
        return Err(CliError::rejected(format!(
            "{requested:?} is not a currency this app quotes in"
        )));
    };
    let transition = ctx.apply(StateCommand::SetFiatCurrency { currency })?;
    let updated = transition.state.settings.fiat_currency.code();

    out.text(|| {
        if updated == current {
            println!("  {} already {}", out::hint("·"), updated.bold());
        } else {
            println!(
                "  {} {} {} {}",
                out::ok_mark(),
                out::hint(current),
                out::hint("→"),
                updated.bold()
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "from": current,
        "currency": updated,
    }));
    Ok(())
}

/// The stored USD cross-rates, optionally refreshed first.
///
/// The rates are core's state, so this reads them from the same store the app
/// does. iOS held them in a blob of its own, which is why they had no CLI at
/// all.
fn rates(ctx: &Ctx, out: Out, refresh: bool) -> CliResult<()> {
    let service = ctx.service()?;
    if refresh {
        ctx.rt
            .block_on(service.refresh_fiat_rates())
            .map_err(CliError::from)?;
    }
    let stored = ctx.state()?.fiat_rates_from_usd;
    let mut rows: Vec<(String, f64)> = stored.into_iter().collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    out.text(|| {
        println!();
        if rows.is_empty() {
            println!(
                "  {}",
                out::hint("no rates stored — run with --refresh-rates")
            );
            return;
        }
        for (code, rate) in &rows {
            out::field(code, &format!("{rate:.6}"));
        }
        println!();
        println!("  {}", out::hint("per 1 USD"));
    });
    out.emit(serde_json::json!({
        "ok": true,
        "base": "USD",
        "count": rows.len(),
        "rates": rows.iter().cloned().collect::<std::collections::HashMap<String, f64>>(),
    }));
    Ok(())
}

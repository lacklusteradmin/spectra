//! The display currency is domain state shared with the app: setting it here
//! changes what the phone shows.

use clap::Args;
use colored::Colorize as _;
use spectra_core::price::PriceRequestCoin;
use spectra_core::registry::Chain;
use spectra_core::store::state::StateCommand;
use std::collections::BTreeSet;

use super::chain::{service_for_chain, BALANCE, RPC};
use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx};
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
    let usd = spot_price_usd(ctx, &[chain])?
        .get(chain.coin_symbol())
        .copied()
        .unwrap_or(0.0);
    let (rate, code) = fiat_conversion(ctx)?;

    out.text(|| {
        println!();
        println!(
            "  {}  {} {}  {}",
            out::tint("●", chain.chain_display_name()).bold(),
            format!("{:.2}", usd * rate).bold(),
            out::hint(&code),
            out::tint(chain.coin_symbol(), chain.chain_display_name()).bold(),
        );
        println!("     {} {}", out::hint("via"), out::hint("CoinGecko"));
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.chain_display_name(),
        "symbol": chain.coin_symbol(),
        "priceUsd": usd,
        "price": usd * rate,
        "currency": code,
    }));
    Ok(())
}

pub fn portfolio(ctx: &Ctx, out: Out, args: PortfolioArgs) -> CliResult<()> {
    let wallets = ctx.state()?.wallets;
    if wallets.is_empty() {
        out.text(|| println!("  {}", out::hint("no wallets")));
        out.emit(serde_json::json!({ "ok": true, "total": 0.0, "holdings": [] }));
        return Ok(());
    }

    let chains: Vec<Chain> = wallets
        .iter()
        .map(|wallet| wallet.chain_name.clone())
        .collect::<BTreeSet<_>>()
        .iter()
        .filter_map(|name| resolve_chain(name).ok())
        .collect();
    let prices = spot_price_usd(ctx, &chains)?;
    let (rate, code) = fiat_conversion(ctx)?;

    let mut rows = Vec::new();
    let mut total_usd = 0.0;
    out.text(|| println!());
    for wallet in &wallets {
        let Ok(chain) = resolve_chain(&wallet.chain_name) else {
            continue;
        };
        let amount = match native_balance(ctx, chain, wallet_address(wallet)) {
            Ok(amount) => amount,
            Err(error) if args.skip_unreachable => {
                out.text(|| {
                    println!(
                        "  {}  {:<14}  {}",
                        out::wallet_dot(&wallet.chain_name, wallet.is_watch_only),
                        wallet.name,
                        out::hint(&format!("unavailable — {error}")),
                    )
                });
                continue;
            }
            Err(error) => return Err(error),
        };

        let price_usd = prices.get(chain.coin_symbol()).copied().unwrap_or(0.0);
        let value_usd = amount * price_usd;
        total_usd += value_usd;

        out.text(|| {
            println!(
                "  {}  {:<14}  {:>14}  {}  {}",
                out::wallet_dot(&wallet.chain_name, wallet.is_watch_only),
                wallet.name,
                format!("{:.4} {}", amount, chain.coin_symbol()),
                out::hint(&format!("@ {:.2}", price_usd * rate)),
                format!("{:.2}", value_usd * rate).bold(),
            )
        });
        rows.push(serde_json::json!({
            "wallet": wallet.id,
            "chain": chain.chain_display_name(),
            "symbol": chain.coin_symbol(),
            "amount": amount,
            "priceUsd": price_usd,
            "valueUsd": value_usd,
        }));
    }

    out.text(|| {
        println!();
        println!(
            "  {}  {} {}",
            out::accent("Σ").bold(),
            format!("{:.2}", total_usd * rate).bold(),
            out::hint(&code),
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "currency": code,
        "rate": rate,
        "totalUsd": total_usd,
        "total": total_usd * rate,
        "holdings": rows,
    }));
    Ok(())
}

pub fn currency(ctx: &Ctx, out: Out, args: CurrencyArgs) -> CliResult<()> {
    if args.refresh_rates || args.rates {
        return rates(ctx, out, args.refresh_rates);
    }
    let current = ctx.state()?.settings.fiat_currency_code;
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

    let transition = ctx.apply(StateCommand::SetFiatCurrency {
        fiat_currency_code: requested.clone(),
    })?;
    // A code no rate table carries is refused rather than stored: it used to
    // be accepted, and every amount then rendered unconverted with that code
    // beside it.
    if transition
        .events
        .iter()
        .any(|event| event.kind == "fiatCurrencyRejected")
    {
        return Err(CliError::rejected(format!(
            "{requested:?} is not a currency this app quotes in"
        )));
    }
    let updated = transition.state.settings.fiat_currency_code;

    out.text(|| {
        if updated == current {
            println!("  {} already {}", out::hint("·"), updated.bold());
        } else {
            println!(
                "  {} {} {} {}",
                out::ok_mark(),
                out::hint(&current),
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

// ─── Shared lookups ─────────────────────────────────────────────────────────

/// Spot USD prices keyed by coin symbol, in one CoinGecko call.
pub(super) fn spot_price_usd(
    ctx: &Ctx,
    chains: &[Chain],
) -> CliResult<std::collections::HashMap<String, f64>> {
    if chains.is_empty() {
        return Ok(Default::default());
    }
    let requests: Vec<PriceRequestCoin> = chains
        .iter()
        .map(|chain| PriceRequestCoin {
            holding_key: chain.coin_symbol().to_string(),
            coin_gecko_id: chain.coin_gecko_id().to_string(),
        })
        .collect();
    // Pricing needs no chain endpoints, and now no service either: the read
    // is a function over the coins asked about.
    ctx.rt
        .block_on(spectra_core::service::fetch_prices_typed(requests))
        .map_err(CliError::from)
}

fn native_balance(ctx: &Ctx, chain: Chain, address: &str) -> CliResult<f64> {
    let service = service_for_chain(chain, BALANCE | RPC)?;
    let summary = ctx
        .rt
        .block_on(
            service.fetch_native_balance_summary(chain.str_id().to_string(), address.to_string()),
        )
        .map_err(CliError::from)?;
    Ok(summary.amount_display.parse().unwrap_or(0.0))
}

/// USD → the selected display currency, as (rate, code).
///
/// Falls back to USD when the selection is USD or the rate lookup fails: a
/// display currency is never worth failing a command over.
fn fiat_conversion(ctx: &Ctx) -> CliResult<(f64, String)> {
    let code = ctx.state()?.settings.fiat_currency_code;
    if code == "USD" {
        return Ok((1.0, code));
    }
    let rates = ctx
        .rt
        .block_on(spectra_core::service::fetch_fiat_rates_typed(vec![
            code.clone()
        ]));
    Ok(match rates.map(|rates| rates.get(&code).copied()) {
        Ok(Some(rate)) if rate > 0.0 => (rate, code),
        _ => (1.0, "USD".to_string()),
    })
}

//! Prices, portfolio value, and the display currency.
//!
//! The currency is a domain setting: it lives in `CoreAppState.settings` and is
//! shared with the app, so setting it here changes what the phone shows. It
//! goes through `SetFiatCurrency` like every other state change.

use clap::Args;
use colored::Colorize as _;
use spectra_core::price::PriceRequestCoin;
use spectra_core::registry::Chain;
use spectra_core::service::WalletService;
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
    chain: String,
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
}

pub fn price(ctx: &Ctx, out: Out, args: PriceArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
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
        fiat_currency_code: requested,
    })?;
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

// ─── Shared lookups ─────────────────────────────────────────────────────────

/// Spot USD prices keyed by coin symbol, in one CoinGecko call.
fn spot_price_usd(
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
            symbol: chain.coin_symbol().to_string(),
            coin_gecko_id: chain.coin_gecko_id().to_string(),
        })
        .collect();
    // Pricing needs no chain endpoints — it is not a per-chain RPC call.
    let service = WalletService::new_typed(Vec::new()).map_err(CliError::from)?;
    ctx.rt
        .block_on(service.fetch_prices_typed("CoinGecko".to_string(), requests))
        .map_err(CliError::from)
}

fn native_balance(ctx: &Ctx, chain: Chain, address: &str) -> CliResult<f64> {
    let service = service_for_chain(chain, BALANCE | RPC)?;
    let summary = ctx
        .rt
        .block_on(service.fetch_native_balance_summary(
            chain.str_id().to_string(),
            address.to_string(),
        ))
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
    let service = WalletService::new_typed(Vec::new()).map_err(CliError::from)?;
    let rates = ctx
        .rt
        .block_on(service.fetch_fiat_rates_typed("OpenER".to_string(), vec![code.clone()]));
    Ok(match rates.map(|rates| rates.get(&code).copied()) {
        Ok(Some(rate)) if rate > 0.0 => (rate, code),
        _ => (1.0, "USD".to_string()),
    })
}

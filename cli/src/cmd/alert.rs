//! Price alerts. Core owns both the rules and the evaluation now — it used to
//! own only the evaluator, with Swift holding the list and applying the verdict
//! to it, which is the `core_plan_*` shape `PLAN.md` is removing.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::StateCommand;
use spectra_core::store::wallet_domain::CorePriceAlertCondition;
use spectra_core::store::{
    core_plan_price_alert_evaluation, PriceAlertEvaluationAlert, PriceAlertEvaluationPrice,
};

use super::market::spot_price_usd;
use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum AlertCommand {
    /// Alerts this wallet has set.
    List,
    /// Add an alert on a chain's native asset.
    Add(AddArgs),
    /// Remove an alert by id or symbol.
    Remove(RemoveArgs),
    /// Fetch live prices and report which alerts fire.
    Check,
}

#[derive(Args)]
pub struct AddArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
    /// Price to watch for, in USD.
    #[arg(long)]
    target: f64,
    /// Fire when the price rises above the target instead of below it.
    #[arg(long)]
    above: bool,
}

#[derive(Args)]
pub struct RemoveArgs {
    /// Alert id or asset symbol.
    alert: String,
}

pub fn run(ctx: &Ctx, out: Out, command: AlertCommand) -> CliResult<()> {
    match command {
        AlertCommand::List => list(ctx, out),
        AlertCommand::Add(args) => add(ctx, out, args),
        AlertCommand::Remove(args) => remove(ctx, out, args),
        AlertCommand::Check => check(ctx, out),
    }
}

fn describe(alert: &PriceAlertEvaluationAlert) -> String {
    format!(
        "{} {} {:.2}",
        alert.symbol,
        match alert.condition {
            CorePriceAlertCondition::Above => "≥",
            CorePriceAlertCondition::Below => "≤",
        },
        alert.target_price
    )
}

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let alerts = ctx.state()?.price_alerts;
    out.text(|| {
        println!();
        if alerts.is_empty() {
            println!("  {}", out::hint("no alerts"));
            return;
        }
        for alert in &alerts {
            println!(
                "  {}  {:<22} {}",
                out::tint("●", &alert.chain_name).bold(),
                describe(alert).bold(),
                out::hint(if alert.has_triggered { "triggered" } else { "armed" }),
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "alerts": alerts
            .iter()
            .map(|alert| serde_json::json!({
                "id": alert.id,
                "symbol": alert.symbol,
                "chain": alert.chain_name,
                "target": alert.target_price,
                "condition": match alert.condition {
                    CorePriceAlertCondition::Above => "above",
                    CorePriceAlertCondition::Below => "below",
                },
                "enabled": alert.is_enabled,
                "triggered": alert.has_triggered,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

fn add(ctx: &Ctx, out: Out, args: AddArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let mut alerts = ctx.state()?.price_alerts;
    let new = PriceAlertEvaluationAlert {
        id: uuid::Uuid::new_v4().to_string().to_uppercase(),
        holding_key: chain.coin_symbol().to_string(),
        asset_name: chain.coin_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        chain_name: chain.chain_display_name().to_string(),
        target_price: args.target,
        condition: if args.above {
            CorePriceAlertCondition::Above
        } else {
            CorePriceAlertCondition::Below
        },
        is_enabled: true,
        has_triggered: false,
    };
    let description = describe(&new);
    alerts.push(new);

    let before = ctx.state()?.price_alerts.len();
    let transition = ctx.apply(StateCommand::SetPriceAlerts { alerts })?;
    if transition.state.price_alerts.len() == before {
        return Err(CliError::rejected(
            "an alert needs a positive target price to fire",
        ));
    }

    out.text(|| println!("  {} watching {}", out::ok_mark(), description.bold()));
    out.emit(serde_json::json!({ "ok": true, "alert": description }));
    Ok(())
}

fn remove(ctx: &Ctx, out: Out, args: RemoveArgs) -> CliResult<()> {
    let alerts = ctx.state()?.price_alerts;
    let remaining: Vec<_> = alerts
        .iter()
        .filter(|alert| {
            !alert.id.eq_ignore_ascii_case(&args.alert)
                && !alert.symbol.eq_ignore_ascii_case(&args.alert)
        })
        .cloned()
        .collect();
    if remaining.len() == alerts.len() {
        return Err(CliError::rejected(format!(
            "no alert matching {:?}",
            args.alert
        )));
    }
    ctx.apply(StateCommand::SetPriceAlerts { alerts: remaining })?;
    out.text(|| println!("  {} removed", out::ok_mark()));
    out.emit(serde_json::json!({ "ok": true, "removed": args.alert }));
    Ok(())
}

fn check(ctx: &Ctx, out: Out) -> CliResult<()> {
    let alerts = ctx.state()?.price_alerts;
    if alerts.is_empty() {
        return Err(CliError::rejected("no alerts to check"));
    }

    let chains: Vec<_> = alerts
        .iter()
        .filter_map(|alert| resolve_chain(&alert.chain_name).ok())
        .collect();
    let prices = spot_price_usd(ctx, &chains)?;

    let plan = core_plan_price_alert_evaluation(
        alerts.clone(),
        prices
            .iter()
            .map(|(symbol, price)| PriceAlertEvaluationPrice {
                holding_key: symbol.clone(),
                live_price: *price,
            })
            .collect(),
    );

    // Core decided which alerts changed state; storing that decision is a
    // command, not something this side works out again.
    if !plan.updates.is_empty() {
        let updated: Vec<_> = alerts
            .iter()
            .map(|alert| {
                let mut alert = alert.clone();
                if let Some(update) = plan.updates.iter().find(|u| u.id == alert.id) {
                    alert.has_triggered = update.has_triggered;
                }
                alert
            })
            .collect();
        ctx.apply(StateCommand::SetPriceAlerts { alerts: updated })?;
    }

    out.text(|| {
        println!();
        if plan.notifications.is_empty() {
            println!("  {}", out::hint("nothing fired"));
        }
        for notification in &plan.notifications {
            println!(
                "  {}  {} crossed {:.2}",
                out::tint("!", &notification.chain_name).bold(),
                notification.symbol.bold(),
                notification.target_price,
            );
        }
        println!();
        println!(
            "  {} checked, {} fired",
            out::accent(&alerts.len().to_string()).bold(),
            plan.notifications.len(),
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "checked": alerts.len(),
        "fired": plan.notifications
            .iter()
            .map(|n| serde_json::json!({
                "symbol": n.symbol,
                "chain": n.chain_name,
                "target": n.target_price,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

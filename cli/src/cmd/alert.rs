//! Price-alert commands. Core owns storage, validation, and evaluation.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::{StateCommand, StateEvent};
use spectra_core::store::wallet_domain::CorePriceAlertCondition;
use spectra_core::store::{PriceAlertEvaluationAlert, PriceAlertRejection};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum AlertCommand {
    /// Alerts this wallet has set.
    List,
    /// Compare stored portfolio holdings and quotes with the durable movement baseline.
    Movement {
        /// Advance the baseline without notifying, as a foreground app does.
        #[arg(long)]
        active: bool,
    },
    /// Add an alert on a chain's native asset.
    Add(AddArgs),
    /// Remove an alert by id or symbol.
    Remove(RemoveArgs),
    /// Toggle one alert by its identifier.
    Toggle { id: String },
    /// Fetch live prices and report which alerts fire.
    Check {
        /// Evaluate already stored quotes without fetching.
        #[arg(long)]
        stored: bool,
    },
}

#[derive(Args)]
pub struct AddArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long, required_unless_present = "holding", conflicts_with = "holding")]
    chain: Option<String>,
    /// Deployment identifier of a stored asset.
    #[arg(long)]
    holding: Option<String>,
    /// Currency of the entered target; core converts using its stored rate.
    #[arg(long, default_value = "USD")]
    currency: String,
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
        AlertCommand::Movement { active } => {
            let notification = ctx
                .rt
                .block_on(ctx.service()?.evaluate_portfolio_movement(active))?;
            out.emit(serde_json::json!({"ok":true,"notification":notification}));
            Ok(())
        }
        AlertCommand::Add(args) => add(ctx, out, args),
        AlertCommand::Remove(args) => remove(ctx, out, args),
        AlertCommand::Toggle { id } => {
            apply_alert(ctx, StateCommand::TogglePriceAlert { id })?;
            out.emit(serde_json::json!({"ok":true}));
            Ok(())
        }
        AlertCommand::Check { stored } => check(ctx, out, stored),
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
                out::hint(if alert.has_triggered {
                    "triggered"
                } else {
                    "armed"
                }),
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

fn apply_alert(ctx: &Ctx, command: StateCommand) -> CliResult<()> {
    let result = ctx.apply(command)?;
    let reason = result.events.iter().find_map(|event| match event {
        StateEvent::PriceAlertRejected { reason } => Some(*reason),
        _ => None,
    });
    match reason {
        Some(reason) => Err(CliError::rejected(rejection_text(reason))),
        None => Ok(()),
    }
}

/// Core decides; the front end only chooses the wording.
fn rejection_text(reason: PriceAlertRejection) -> &'static str {
    match reason {
        PriceAlertRejection::MissingCurrencyRate => "no exchange rate is stored for that currency",
        PriceAlertRejection::InvalidTarget => "the target must be a positive number",
        PriceAlertRejection::UnknownAsset => "no asset matches that holding",
        PriceAlertRejection::DuplicateAlert => "an identical alert already exists",
        PriceAlertRejection::AlertNotFound => "no such alert",
    }
}
fn add(ctx: &Ctx, out: Out, args: AddArgs) -> CliResult<()> {
    let key = match args.holding {
        Some(key) => key,
        None => resolve_chain(args.chain.as_deref().unwrap_or_default())?
            .entry()
            .native_deployment_id
            .clone(),
    };
    apply_alert(
        ctx,
        StateCommand::AddPriceAlert {
            holding_key: key,
            target_price: args.target,
            currency: spectra_core::store::state::FiatCurrency::from_code(&args.currency)
                .ok_or_else(|| {
                    CliError::rejected(format!(
                        "{:?} is not a currency this app quotes in",
                        args.currency
                    ))
                })?,
            condition: if args.above {
                CorePriceAlertCondition::Above
            } else {
                CorePriceAlertCondition::Below
            },
        },
    )?;
    out.text(|| println!("Alert added"));
    out.emit(serde_json::json!({"ok":true}));
    Ok(())
}
fn remove(ctx: &Ctx, out: Out, args: RemoveArgs) -> CliResult<()> {
    let matches: Vec<_> = ctx
        .state()?
        .price_alerts
        .into_iter()
        .filter(|a| {
            a.id.eq_ignore_ascii_case(&args.alert) || a.symbol.eq_ignore_ascii_case(&args.alert)
        })
        .collect();
    if matches.is_empty() {
        return Err(CliError::rejected("Alert not found"));
    }
    for alert in matches {
        apply_alert(ctx, StateCommand::RemovePriceAlert { id: alert.id })?;
    }
    out.emit(serde_json::json!({"ok":true}));
    Ok(())
}

fn check(ctx: &Ctx, out: Out, stored: bool) -> CliResult<()> {
    let alerts = ctx.state()?.price_alerts;
    if alerts.is_empty() {
        return Err(CliError::rejected("no alerts to check"));
    }

    let service = ctx.service()?;
    if !stored {
        ctx.rt.block_on(service.refresh_owned_prices(true))?;
    }
    let notifications = ctx.rt.block_on(service.evaluate_price_alerts())?;

    out.text(|| {
        println!();
        if notifications.is_empty() {
            println!("  {}", out::hint("nothing fired"));
        }
        for notification in &notifications {
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
            notifications.len(),
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "checked": alerts.len(),
        "fired": notifications
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

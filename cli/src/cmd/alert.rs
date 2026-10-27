//! Price alerts and the large-movement check. Core owns storage, validation
//! and evaluation.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::LargeMovementEvaluation;
use spectra_core::store::state::{FiatCurrency, StateCommand, StateEvent};
use spectra_core::store::wallet_domain::PriceAlertCondition;
use spectra_core::store::{PriceAlertNotification, PriceAlertRejection, PriceAlertRule};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Done, Out, Report};

#[derive(Subcommand)]
pub enum AlertCommand {
    /// List stored price alerts.
    List,
    /// Compare stored portfolio holdings and quotes with the durable movement baseline.
    Movement {
        /// Advance the baseline without notifying, as a foreground app does.
        #[arg(long)]
        active: bool,
    },
    /// Add a price alert for a native asset or stored token holding.
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
    /// Chain display name or registry id.
    #[arg(long, required_unless_present = "holding", conflicts_with = "holding")]
    chain: Option<String>,
    /// Deployment identifier of a stored asset.
    #[arg(long)]
    holding: Option<String>,
    /// Currency of the entered target; core converts using its stored rate.
    #[arg(long, default_value = "USD")]
    currency: String,
    /// Target price in the currency selected with --currency.
    #[arg(long)]
    target: String,
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
            out.report(&MovementReport { notification })
        }
        AlertCommand::Add(args) => add(ctx, out, args),
        AlertCommand::Remove(args) => remove(ctx, out, args),
        AlertCommand::Toggle { id } => {
            apply_alert(ctx, StateCommand::TogglePriceAlert { id: id.clone() })?;
            out.report(&Done::new(format!("toggled {id}")))
        }
        AlertCommand::Check { stored } => check(ctx, out, stored),
    }
}

fn condition_mark(condition: PriceAlertCondition) -> &'static str {
    match condition {
        PriceAlertCondition::Above => "≥",
        PriceAlertCondition::Below => "≤",
    }
}

/// Whether the portfolio moved past the large-movement threshold since its
/// baseline.
#[derive(Serialize)]
struct MovementReport {
    notification: Option<LargeMovementEvaluation>,
}

impl Report for MovementReport {
    fn render(&self) {
        let Some(movement) = &self.notification else {
            println!("  {}", out::hint("no baseline to compare with yet"));
            return;
        };
        let direction = if movement.direction_up {
            out::up("up")
        } else {
            out::down("down")
        };
        println!(
            "  {} {} {} ({:.1}%){}",
            direction,
            out::fiat(Some(movement.absolute_delta), movement.currency).bold(),
            movement.currency.code(),
            movement.ratio * 100.0,
            if movement.should_alert {
                "  — past the threshold"
            } else {
                ""
            }
        );
    }
}

#[derive(Serialize)]
struct AlertRow {
    id: String,
    symbol: String,
    chain: Chain,
    target: f64,
    condition: &'static str,
    enabled: bool,
    triggered: bool,
    #[serde(skip)]
    mark: &'static str,
}

impl From<&PriceAlertRule> for AlertRow {
    fn from(alert: &PriceAlertRule) -> Self {
        Self {
            id: alert.id.clone(),
            symbol: alert.symbol.clone(),
            chain: alert.chain_id,
            target: alert.target_price,
            condition: match alert.condition {
                PriceAlertCondition::Above => "above",
                PriceAlertCondition::Below => "below",
            },
            enabled: alert.is_enabled,
            triggered: alert.has_triggered,
            mark: condition_mark(alert.condition),
        }
    }
}

/// The stored alerts; targets are in USD, which core converts entries into.
#[derive(Serialize)]
struct AlertsReport {
    alerts: Vec<AlertRow>,
}

impl Report for AlertsReport {
    fn render(&self) {
        println!();
        if self.alerts.is_empty() {
            println!("  {}", out::hint("no alerts"));
            return;
        }
        for alert in &self.alerts {
            let rule = format!(
                "{} {} {} USD",
                alert.symbol,
                alert.mark,
                out::fiat(Some(alert.target), FiatCurrency::Usd)
            );
            let state = match (alert.enabled, alert.triggered) {
                (false, _) => "off",
                (true, true) => "triggered",
                (true, false) => "armed",
            };
            println!(
                "  {}  {:<26} {}",
                out::tint("●", alert.chain).bold(),
                rule.bold(),
                out::hint(state),
            );
        }
    }
}

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let alerts = ctx
        .state()?
        .price_alerts
        .iter()
        .map(AlertRow::from)
        .collect();
    out.report(&AlertsReport { alerts })
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
    let currency = FiatCurrency::from_code(&args.currency).ok_or_else(|| {
        CliError::rejected(format!(
            "{:?} is not a currency this app quotes in",
            args.currency
        ))
    })?;
    let condition = if args.above {
        PriceAlertCondition::Above
    } else {
        PriceAlertCondition::Below
    };
    apply_alert(
        ctx,
        StateCommand::AddPriceAlert {
            holding_key: key.clone(),
            target_price: args.target.clone(),
            currency,
            condition,
        },
    )?;
    out.report(&Done::new(format!(
        "alert added: {key} {} {} {}",
        condition_mark(condition),
        args.target,
        currency.code()
    )))
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
    let count = matches.len();
    for alert in matches {
        apply_alert(ctx, StateCommand::RemovePriceAlert { id: alert.id })?;
    }
    out.report(&Done::new(format!(
        "removed {}",
        out::count(count, "alert")
    )))
}

#[derive(Serialize)]
struct FiredRow {
    symbol: String,
    chain: Chain,
    target: f64,
    price: f64,
    currency: &'static str,
    #[serde(skip)]
    fiat: FiatCurrency,
}

impl From<&PriceAlertNotification> for FiredRow {
    fn from(notification: &PriceAlertNotification) -> Self {
        Self {
            symbol: notification.symbol.clone(),
            chain: notification.chain_id,
            target: notification.target_price,
            price: notification.live_price,
            currency: notification.currency.code(),
            fiat: notification.currency,
        }
    }
}

/// Which alerts the current prices fire.
#[derive(Serialize)]
struct CheckReport {
    checked: usize,
    fired: Vec<FiredRow>,
}

impl Report for CheckReport {
    fn render(&self) {
        println!();
        if self.fired.is_empty() {
            println!("  {}", out::hint("nothing fired"));
        }
        for fired in &self.fired {
            println!(
                "  {}  {} crossed {} {}, now {}",
                out::tint("!", fired.chain).bold(),
                fired.symbol.bold(),
                out::fiat(Some(fired.target), fired.fiat),
                fired.currency,
                out::fiat(Some(fired.price), fired.fiat),
            );
        }
        println!();
        println!(
            "  {} checked, {} fired",
            out::accent(&self.checked.to_string()).bold(),
            self.fired.len(),
        );
    }
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
    out.report(&CheckReport {
        checked: alerts.len(),
        fired: notifications.iter().map(FiredRow::from).collect(),
    })
}

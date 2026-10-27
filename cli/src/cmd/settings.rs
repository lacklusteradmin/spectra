//! The settings core owns. They decide what gets fetched, what a send costs
//! and when an alert fires, so the CLI reads and sets the same ones the app
//! does.

use std::collections::BTreeMap;

use clap::{Args, Subcommand};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::api::tor::TorStatus;
use spectra_core::service::ResetPlan;
use spectra_core::store::state::{
    AppSettingUpdate, AppSettings, BackgroundSyncProfile, ResetScope, StateCommand, StateEvent,
};

use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Done, Out, Report};

#[derive(Subcommand)]
pub enum SettingsCommand {
    /// List settings available through this command and their current values.
    List,
    /// Read one setting.
    Get(GetArgs),
    /// Change one setting.
    Set(SetArgs),
    /// Reset selected data scopes; default: settings, endpoints and token preferences.
    Reset(ResetArgs),
    /// Delete a store this build cannot read, with every wallet secret it names, and start an empty one.
    Discard(DiscardArgs),
}

#[derive(Args)]
pub struct DiscardArgs {
    /// Confirm deleting the stored data and the wallet secrets it names.
    #[arg(long)]
    yes: bool,
}

#[derive(Args)]
pub struct ResetArgs {
    /// Data scope; repeat for multiple scopes. Removing wallets also clears history and cache.
    #[arg(long, value_parser = ResetScope::ALL.map(ResetScope::as_raw))]
    scope: Vec<String>,
    /// Confirm resetting the selected data scopes.
    #[arg(long)]
    yes: bool,
}

#[derive(Args)]
pub struct GetArgs {
    /// Setting key, as `settings list` prints it.
    key: String,
}

#[derive(Args)]
pub struct SetArgs {
    /// Setting key, as `settings list` prints it.
    key: String,
    /// New value. Booleans take true/false; numbers are bounded by core.
    value: String,
}

pub fn run(ctx: &Ctx, out: Out, command: SettingsCommand) -> CliResult<()> {
    match command {
        SettingsCommand::List => list(ctx, out),
        SettingsCommand::Get(args) => get(ctx, out, args),
        SettingsCommand::Set(args) => set(ctx, out, args),
        SettingsCommand::Reset(args) => reset(ctx, out, args),
        SettingsCommand::Discard(args) => discard(ctx, out, args),
    }
}

/// The key a caller types, and how to read and write that field.
///
/// One table rather than a match per operation: `list`, `get` and `set` all
/// need the same key set, and three copies of it is how a key comes to exist
/// for one of them only.
struct Field {
    key: &'static str,
    read: fn(&AppSettings) -> String,
    update: fn(&str) -> Result<AppSettingUpdate, &'static str>,
}

fn parse_bool(raw: &str) -> Result<bool, &'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err("expected true or false"),
    }
}

fn parse_u32(raw: &str) -> Result<u32, &'static str> {
    raw.trim().parse().map_err(|_| "expected a whole number")
}

fn parse_f64(raw: &str) -> Result<f64, &'static str> {
    raw.trim().parse().map_err(|_| "expected a number")
}

const FIELDS: &[Field] = &[
    Field {
        key: "bitcoin-stop-gap",
        read: |s| s.bitcoin_stop_gap.to_string(),
        update: |v| parse_u32(v).map(|value| AppSettingUpdate::BitcoinStopGap { value }),
    },
    Field {
        key: "background-sync-profile",
        read: |s| s.background_sync_profile.as_raw().to_string(),
        update: |v| {
            BackgroundSyncProfile::from_raw(v)
                .map(|value| AppSettingUpdate::BackgroundSyncProfile { value })
                .ok_or("expected conservative, balanced or aggressive")
        },
    },
    Field {
        key: "price-alerts",
        read: |s| s.use_price_alerts.to_string(),
        update: |v| parse_bool(v).map(|value| AppSettingUpdate::UsePriceAlerts { value }),
    },
    Field {
        key: "transaction-status-notifications",
        read: |s| s.use_transaction_status_notifications.to_string(),
        update: |v| {
            parse_bool(v).map(|value| AppSettingUpdate::UseTransactionStatusNotifications { value })
        },
    },
    Field {
        key: "large-movement-notifications",
        read: |s| s.use_large_movement_notifications.to_string(),
        update: |v| {
            parse_bool(v).map(|value| AppSettingUpdate::UseLargeMovementNotifications { value })
        },
    },
    Field {
        key: "large-movement-percent",
        read: |s| s.large_movement_alert_percent_threshold.to_string(),
        update: |v| {
            parse_f64(v).map(|value| AppSettingUpdate::LargeMovementAlertPercentThreshold { value })
        },
    },
    Field {
        key: "tor-enabled",
        read: |s| s.tor_enabled.to_string(),
        update: |v| parse_bool(v).map(|value| AppSettingUpdate::TorEnabled { value }),
    },
    Field {
        key: "tor-custom-proxy",
        read: |s| s.tor_use_custom_proxy.to_string(),
        update: |v| parse_bool(v).map(|value| AppSettingUpdate::TorUseCustomProxy { value }),
    },
    Field {
        key: "tor-proxy-address",
        read: |s| s.tor_custom_proxy_address.clone(),
        update: |v| Ok(AppSettingUpdate::TorCustomProxyAddress { value: v.into() }),
    },
    Field {
        key: "large-movement-usd",
        read: |s| s.large_movement_alert_usd_threshold.to_string(),
        update: |v| {
            parse_f64(v).map(|value| AppSettingUpdate::LargeMovementAlertUsdThreshold { value })
        },
    },
];

fn field(key: &str) -> CliResult<&'static Field> {
    FIELDS
        .iter()
        .find(|field| field.key == key)
        .ok_or_else(|| CliError::rejected(format!("no setting named {key}")))
}

/// Every setting this command reads and writes, by key.
#[derive(Serialize)]
struct SettingsReport {
    settings: BTreeMap<&'static str, String>,
}

impl Report for SettingsReport {
    fn render(&self) {
        println!();
        for field in FIELDS {
            println!(
                "  {:<34} {}",
                field.key.bold(),
                out::hint(&self.settings[field.key])
            );
        }
    }
}

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let settings = ctx.state()?.settings;
    out.report(&SettingsReport {
        settings: FIELDS
            .iter()
            .map(|field| (field.key, (field.read)(&settings)))
            .collect(),
    })
}

/// One setting's value, as core stored it.
#[derive(Serialize)]
struct SettingReport {
    key: &'static str,
    value: String,
    #[serde(skip)]
    changed: bool,
}

impl Report for SettingReport {
    fn render(&self) {
        if self.changed {
            println!("  {} {} = {}", out::ok_mark(), self.key, self.value.bold());
        } else {
            println!("  {}", self.value.bold());
        }
    }
}

fn get(ctx: &Ctx, out: Out, args: GetArgs) -> CliResult<()> {
    let field = field(&args.key)?;
    out.report(&SettingReport {
        key: field.key,
        value: (field.read)(&ctx.state()?.settings),
        changed: false,
    })
}

fn set(ctx: &Ctx, out: Out, args: SetArgs) -> CliResult<()> {
    let field = field(&args.key)?;
    let key = field.key;
    let update = (field.update)(&args.value)
        .map_err(|reason| CliError::rejected(format!("{key}: {reason}")))?;
    let transition = ctx.apply(StateCommand::SetAppSetting { update })?;
    // Core refuses a value it cannot store — an unknown chain, an address that
    // is not a SOCKS5 URL — and says so rather than leaving the caller to
    // notice that the read-back is the old value.
    if transition
        .events
        .iter()
        .any(|event| matches!(event, StateEvent::AppSettingRejected))
    {
        return Err(CliError::rejected(format!(
            "{key}: core refused {:?}",
            args.value
        )));
    }
    // Report what core stored, not what was asked for: it trims strings and
    // bounds numbers, so the two differ often enough to be worth showing.
    out.report(&SettingReport {
        key,
        value: (field.read)(&transition.state.settings),
        changed: true,
    })
}

/// Reset core-owned data and describe the effective plan, including implied scopes.
fn reset(ctx: &Ctx, out: Out, args: ResetArgs) -> CliResult<()> {
    let scopes = if args.scope.is_empty() {
        vec![ResetScope::SettingsAndEndpoints]
    } else {
        args.scope
            .iter()
            .map(|raw| {
                ResetScope::from_raw(raw)
                    .ok_or_else(|| CliError::rejected(format!("{raw:?} is not a reset scope")))
            })
            .collect::<CliResult<Vec<_>>>()?
    };
    if !args.yes {
        let plan = spectra_core::service::reset_dispatch(scopes);
        return Err(CliError::usage(format!(
            "this resets {} — re-run with --yes",
            reset_description(&plan)
        )));
    }
    let outcome = ctx.rt.block_on(ctx.service()?.reset_data(scopes))?;
    out.report(&ResetReport { plan: outcome.plan })
}

/// What a reset cleared, implied scopes included.
#[derive(Serialize)]
struct ResetReport {
    plan: ResetPlan,
}

impl Report for ResetReport {
    fn render(&self) {
        println!(
            "  {} reset: {}",
            out::ok_mark(),
            reset_description(&self.plan)
        );
    }
}

/// Replace a store `open_state` cannot read. Only a store that will not open
/// gets here: a readable one is cleared with `settings reset`.
fn discard(ctx: &Ctx, out: Out, args: DiscardArgs) -> CliResult<()> {
    if !args.yes {
        return Err(CliError::usage(
            "this deletes the stored data and every wallet secret it names — re-run with --yes",
        ));
    }
    ctx.rt
        .block_on(ctx.unopened_service()?.discard_state(ctx.db_path()))?;
    out.report(&Done::new("discarded the stored data"))
}

/// The configured transport's state, once it is ready or has failed.
#[derive(Serialize)]
struct TorReport {
    status: TorStatus,
}

impl Report for TorReport {
    fn render(&self) {
        match &self.status {
            TorStatus::Stopped => println!("  {}", out::hint("Tor is off")),
            TorStatus::Bootstrapping { percent } => {
                println!("  {} Tor bootstrapping, {percent}%", out::hint("…"))
            }
            TorStatus::Ready => println!("  {} Tor ready", out::ok_mark()),
            TorStatus::Error { message } => println!("  {} Tor: {message}", out::fail_mark()),
        }
    }
}

/// Start the configured transport, await readiness and report its status.
pub fn tor(ctx: &Ctx, out: Out, reconnect: bool) -> CliResult<()> {
    let service = ctx.service()?;
    if reconnect {
        ctx.rt.block_on(service.reconnect_tor());
        ctx.rt.block_on(service.await_network_ready())?;
    }
    out.report(&TorReport {
        status: spectra_core::api::tor::tor_status(),
    })
}

fn reset_description(plan: &ResetPlan) -> String {
    [
        (plan.reset_wallets_and_secrets, "wallets and secrets"),
        (plan.reset_history_and_cache, "history and cache"),
        (plan.reset_alerts_and_contacts, "alerts and contacts"),
        (
            plan.reset_settings_and_endpoints,
            "settings, endpoints and token preferences",
        ),
        (plan.reset_dashboard_customization, "dashboard pins"),
    ]
    .into_iter()
    .filter_map(|(enabled, description)| enabled.then_some(description))
    .collect::<Vec<_>>()
    .join(", ")
}

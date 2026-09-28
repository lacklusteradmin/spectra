//! The settings core owns. They decide what gets fetched, what a send costs
//! and when an alert fires, so the CLI reads and sets the same ones the app
//! does.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::{
    AppSettingUpdate, AppSettings, BackgroundSyncProfile, ResetScope, StateCommand, StateEvent,
};

use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum SettingsCommand {
    /// Every setting and its current value.
    List,
    /// Read one setting.
    Get(GetArgs),
    /// Change one setting.
    Set(SetArgs),
    /// Put every setting back to its default.
    Reset(ResetArgs),
}

#[derive(Args)]
pub struct ResetArgs {
    /// Domain reset scopes; repeat for multiple scopes. Default: settings only.
    #[arg(long)]
    scope: Vec<String>,
    /// Reset without asking for confirmation.
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
        key: "tor-kill-switch",
        read: |s| s.tor_kill_switch.to_string(),
        update: |v| parse_bool(v).map(|value| AppSettingUpdate::TorKillSwitch { value }),
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

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let settings = ctx.state()?.settings;
    out.text(|| {
        println!();
        for field in FIELDS {
            println!(
                "  {:<34} {}",
                field.key.bold(),
                out::hint(&(field.read)(&settings))
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "settings": FIELDS
            .iter()
            .map(|field| {
                (
                    field.key.to_string(),
                    serde_json::Value::String((field.read)(&settings)),
                )
            })
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    }));
    Ok(())
}

fn get(ctx: &Ctx, out: Out, args: GetArgs) -> CliResult<()> {
    let field = field(&args.key)?;
    let value = (field.read)(&ctx.state()?.settings);
    out.text(|| println!("  {}", value.bold()));
    out.emit(serde_json::json!({ "ok": true, "key": field.key, "value": value }));
    Ok(())
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
    let stored = (field.read)(&transition.state.settings);
    out.text(|| println!("  {} {key} = {}", out::ok_mark(), stored.bold()));
    out.emit(serde_json::json!({ "ok": true, "key": key, "value": stored }));
    Ok(())
}

/// Put every setting core owns back to its default.
///
/// The defaults live in `AppSettings::default()`. This is the only way to ask
/// for all of them at once, and it is what makes the rule testable — iOS reset
/// settings by assigning each mirror a literal it believed was the default,
/// which no test on either side could check against core.
fn reset(ctx: &Ctx, out: Out, args: ResetArgs) -> CliResult<()> {
    if !args.yes {
        return Err(CliError::usage(
            "this discards every setting, including endpoints and preferences — re-run with --yes",
        ));
    }
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
    ctx.rt.block_on(ctx.service()?.reset_data(scopes))?;
    let count = FIELDS.len();
    out.text(|| println!("  {} {count} settings at their defaults", out::ok_mark()));
    out.emit(serde_json::json!({ "ok": true, "settings": count }));
    Ok(())
}

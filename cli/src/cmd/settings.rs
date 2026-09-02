//! The settings core owns.
//!
//! These lived in a `PersistedAppSettings` record iOS wrote as one blob and no
//! other front end could read. They decide what gets fetched, what a send costs
//! and when an alert fires, so a CLI that could not read or set them was not
//! driving the same core the app was.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::{AppSettingUpdate, AppSettings, StateCommand};

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
        key: "etherscan-api-key",
        read: |s| s.etherscan_api_key.clone(),
        update: |v| Ok(AppSettingUpdate::EtherscanApiKey { value: v.into() }),
    },
    Field {
        key: "monero-backend-url",
        read: |s| s.monero_backend_base_url.clone(),
        update: |v| Ok(AppSettingUpdate::MoneroBackendBaseUrl { value: v.into() }),
    },
    Field {
        key: "monero-backend-api-key",
        read: |s| s.monero_backend_api_key.clone(),
        update: |v| Ok(AppSettingUpdate::MoneroBackendApiKey { value: v.into() }),
    },
    Field {
        key: "bitcoin-esplora-endpoints",
        read: |s| s.bitcoin_esplora_endpoints.clone(),
        update: |v| Ok(AppSettingUpdate::BitcoinEsploraEndpoints { value: v.into() }),
    },
    Field {
        key: "bitcoin-stop-gap",
        read: |s| s.bitcoin_stop_gap.to_string(),
        update: |v| parse_u32(v).map(|value| AppSettingUpdate::BitcoinStopGap { value }),
    },
    Field {
        key: "strict-rpc-only",
        read: |s| s.use_strict_rpc_only.to_string(),
        update: |v| parse_bool(v).map(|value| AppSettingUpdate::UseStrictRpcOnly { value }),
    },
    Field {
        key: "background-sync-profile",
        read: |s| s.background_sync_profile.clone(),
        update: |v| Ok(AppSettingUpdate::BackgroundSyncProfile { value: v.into() }),
    },
    Field {
        key: "refresh-frequency-minutes",
        read: |s| s.automatic_refresh_frequency_minutes.to_string(),
        update: |v| {
            parse_u32(v).map(|value| AppSettingUpdate::AutomaticRefreshFrequencyMinutes { value })
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
        key: "large-movement-usd",
        read: |s| s.large_movement_alert_usd_threshold.to_string(),
        update: |v| {
            parse_f64(v).map(|value| AppSettingUpdate::LargeMovementAlertUsdThreshold { value })
        },
    },
];

/// A setting keyed by chain rather than global, named `<prefix><chain>`.
///
/// These cannot be rows in `FIELDS` — there would have to be seventy-eight of
/// each, regenerated whenever `chains.toml` changes. Each one replaced a
/// scalar field that served a single hard-coded chain.
struct ChainKeyedField {
    prefix: &'static str,
    read: fn(&AppSettings, &str) -> String,
    update: fn(&str, &str) -> AppSettingUpdate,
    /// The chains that currently have a value stored, so `list` can show them.
    stored: fn(&AppSettings) -> Vec<String>,
}

const CHAIN_KEYED: &[ChainKeyedField] = &[
    ChainKeyedField {
        prefix: "fee-priority.",
        read: |s, chain| {
            s.fee_priority_by_chain
                .get(chain)
                .cloned()
                .unwrap_or_else(|| "normal".to_string())
        },
        update: |chain, value| AppSettingUpdate::FeePriority {
            chain: chain.to_string(),
            value: value.to_string(),
        },
        stored: |s| s.fee_priority_by_chain.keys().cloned().collect(),
    },
    ChainKeyedField {
        prefix: "rpc-endpoint.",
        read: |s, chain| s.rpc_endpoint_by_chain.get(chain).cloned().unwrap_or_default(),
        update: |chain, value| AppSettingUpdate::RpcEndpoint {
            chain: chain.to_string(),
            value: value.to_string(),
        },
        stored: |s| s.rpc_endpoint_by_chain.keys().cloned().collect(),
    },
];

/// A setting a caller can name: one of the fixed rows, or one chain's value
/// under one of the keyed families.
enum Setting {
    Scalar(&'static Field),
    ChainKeyed(&'static ChainKeyedField, String),
}

impl Setting {
    fn key(&self) -> String {
        match self {
            Setting::Scalar(field) => field.key.to_string(),
            Setting::ChainKeyed(field, chain) => format!("{}{chain}", field.prefix),
        }
    }

    fn read(&self, settings: &AppSettings) -> String {
        match self {
            Setting::Scalar(field) => (field.read)(settings),
            Setting::ChainKeyed(field, chain) => (field.read)(settings, chain),
        }
    }

    fn update(&self, raw: &str) -> Result<AppSettingUpdate, &'static str> {
        match self {
            Setting::Scalar(field) => (field.update)(raw),
            Setting::ChainKeyed(field, chain) => Ok((field.update)(chain, raw)),
        }
    }
}

fn field(key: &str) -> CliResult<Setting> {
    for keyed in CHAIN_KEYED {
        let Some(name) = key.strip_prefix(keyed.prefix) else {
            continue;
        };
        let chain = spectra_core::registry::Chain::from_display_name(name)
            .ok_or_else(|| CliError::rejected(format!("no chain named {name}")))?;
        return Ok(Setting::ChainKeyed(
            keyed,
            chain.chain_display_name().to_string(),
        ));
    }
    FIELDS
        .iter()
        .find(|field| field.key == key)
        .map(Setting::Scalar)
        .ok_or_else(|| CliError::rejected(format!("no setting named {key}")))
}

/// Every setting that has a value to print: the fixed rows, then whichever
/// chains have a value stored under each keyed family. Chains at the default
/// are left out — listing all seventy-eight of each would bury the rest.
fn settings_in_order(settings: &AppSettings) -> Vec<Setting> {
    let mut all: Vec<Setting> = FIELDS.iter().map(Setting::Scalar).collect();
    for keyed in CHAIN_KEYED {
        let mut chains = (keyed.stored)(settings);
        chains.sort();
        all.extend(
            chains
                .into_iter()
                .map(|chain| Setting::ChainKeyed(keyed, chain)),
        );
    }
    all
}

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let settings = ctx.state()?.settings;
    let all = settings_in_order(&settings);
    out.text(|| {
        println!();
        for setting in &all {
            println!(
                "  {:<34} {}",
                setting.key().bold(),
                out::hint(&setting.read(&settings))
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "settings": all
            .iter()
            .map(|setting| {
                (
                    setting.key(),
                    serde_json::Value::String(setting.read(&settings)),
                )
            })
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    }));
    Ok(())
}

fn get(ctx: &Ctx, out: Out, args: GetArgs) -> CliResult<()> {
    let setting = field(&args.key)?;
    let value = setting.read(&ctx.state()?.settings);
    out.text(|| println!("  {}", value.bold()));
    out.emit(serde_json::json!({ "ok": true, "key": setting.key(), "value": value }));
    Ok(())
}

fn set(ctx: &Ctx, out: Out, args: SetArgs) -> CliResult<()> {
    let setting = field(&args.key)?;
    let key = setting.key();
    let update = setting
        .update(&args.value)
        .map_err(|reason| CliError::rejected(format!("{key}: {reason}")))?;
    let transition = ctx.apply(StateCommand::SetAppSetting { update })?;
    // Report what core stored, not what was asked for: it trims strings and
    // bounds numbers, so the two differ often enough to be worth showing.
    let stored = setting.read(&transition.state.settings);
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
            "this discards every setting, including endpoints and API keys — re-run with --yes",
        ));
    }
    let transition = ctx.apply(StateCommand::ResetAppSettings)?;
    let settings = transition.state.settings;
    let count = settings_in_order(&settings).len();
    out.text(|| println!("  {} {count} settings at their defaults", out::ok_mark()));
    out.emit(serde_json::json!({ "ok": true, "settings": count }));
    Ok(())
}

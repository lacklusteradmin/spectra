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
        key: "pricing-provider",
        read: |s| s.pricing_provider.clone(),
        update: |v| Ok(AppSettingUpdate::PricingProvider { value: v.into() }),
    },
    Field {
        key: "fiat-rate-provider",
        read: |s| s.fiat_rate_provider.clone(),
        update: |v| Ok(AppSettingUpdate::FiatRateProvider { value: v.into() }),
    },
    Field {
        key: "ethereum-rpc-endpoint",
        read: |s| s.ethereum_rpc_endpoint.clone(),
        update: |v| Ok(AppSettingUpdate::EthereumRpcEndpoint { value: v.into() }),
    },
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

/// The prefix that names a per-chain fee preference, as
/// `fee-priority.<chain display name>`.
///
/// Fee priority is the one setting keyed by chain rather than global, so it
/// cannot be a row in `FIELDS` — there would have to be seventy-eight of them,
/// and the table would have to be regenerated whenever `chains.toml` changes.
const FEE_PRIORITY_PREFIX: &str = "fee-priority.";

/// A setting a caller can name: one of the fixed rows, or one chain's fee.
enum Setting {
    Scalar(&'static Field),
    FeePriority(String),
}

impl Setting {
    fn key(&self) -> String {
        match self {
            Setting::Scalar(field) => field.key.to_string(),
            Setting::FeePriority(chain) => format!("{FEE_PRIORITY_PREFIX}{chain}"),
        }
    }

    fn read(&self, settings: &AppSettings) -> String {
        match self {
            Setting::Scalar(field) => (field.read)(settings),
            Setting::FeePriority(chain) => settings
                .fee_priority_by_chain
                .get(chain)
                .cloned()
                .unwrap_or_else(|| "normal".to_string()),
        }
    }

    fn update(&self, raw: &str) -> Result<AppSettingUpdate, &'static str> {
        match self {
            Setting::Scalar(field) => (field.update)(raw),
            Setting::FeePriority(chain) => Ok(AppSettingUpdate::FeePriority {
                chain: chain.clone(),
                value: raw.into(),
            }),
        }
    }
}

fn field(key: &str) -> CliResult<Setting> {
    if let Some(name) = key.strip_prefix(FEE_PRIORITY_PREFIX) {
        let chain = spectra_core::registry::Chain::from_display_name(name)
            .ok_or_else(|| CliError::rejected(format!("no chain named {name}")))?;
        return Ok(Setting::FeePriority(chain.chain_display_name().to_string()));
    }
    FIELDS
        .iter()
        .find(|field| field.key == key)
        .map(Setting::Scalar)
        .ok_or_else(|| CliError::rejected(format!("no setting named {key}")))
}

/// Every setting that has a value to print: the fixed rows, then whichever
/// chains have a fee preference stored. Chains at the default are left out —
/// listing all seventy-eight would bury the seventeen that are not per-chain.
fn settings_in_order(settings: &AppSettings) -> Vec<Setting> {
    let mut all: Vec<Setting> = FIELDS.iter().map(Setting::Scalar).collect();
    let mut chains: Vec<&String> = settings.fee_priority_by_chain.keys().collect();
    chains.sort();
    all.extend(chains.into_iter().map(|c| Setting::FeePriority(c.clone())));
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

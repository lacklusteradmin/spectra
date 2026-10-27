//! Core's self-tests, its refresh policy and the diagnostics documents the
//! app's diagnostics screen shows. The self-tests need no network and no
//! device.

use std::collections::HashMap;

use clap::{Args, Subcommand};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::diagnostics::self_tests::ChainSelfTestResult;
use spectra_core::registry::Chain;
use spectra_core::service::app_refresh::AppRefreshResult;
use spectra_core::service::refresh_policy::MaintenancePlan;
use spectra_core::service::{
    ChainDegradation, ConfiguredSelfTestReport, DiagnosticLogLevel, DiagnosticState,
};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Subcommand)]
pub enum DiagnosticsCommand {
    /// Inspect the core refresh policy with explicit device conditions, offline.
    Maintenance {
        #[arg(long)]
        conditions: String,
    },
    /// Execute a core-owned refresh with explicit intent and device conditions.
    Refresh {
        #[arg(long)]
        intent: String,
        #[arg(long)]
        conditions: String,
    },
    /// Read durable diagnostics, optionally applying one typed JSON intent.
    State {
        #[arg(long)]
        command: Option<String>,
    },
    /// Run core's self-tests for one chain, or all of them.
    SelfTest(SelfTestArgs),
    /// Diagnose the selected network and RPC using the service's current settings.
    Configured {
        #[arg(long)]
        chain: String,
    },
    /// The diagnostics document core builds for a chain, on its selected network.
    Show(ShowArgs),
    /// The diagnostics bundle core builds: every network's document and a header.
    Bundle,
}

#[derive(Args)]
pub struct SelfTestArgs {
    /// Chain display name or registry id. Omit to run every chain.
    #[arg(long)]
    chain: Option<String>,
}

#[derive(Args)]
pub struct ShowArgs {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
}

pub fn run(ctx: &Ctx, out: Out, command: DiagnosticsCommand) -> CliResult<()> {
    match command {
        DiagnosticsCommand::Maintenance { conditions } => {
            let conditions = serde_json::from_str(&conditions)
                .map_err(|e| CliError::usage(format!("invalid conditions: {e}")))?;
            let plan = ctx.rt.block_on(ctx.service()?.maintenance_plan(conditions));
            out.report(&PlanReport { plan })
        }
        DiagnosticsCommand::Refresh { intent, conditions } => {
            let intent = serde_json::from_str(&intent)
                .map_err(|e| CliError::usage(format!("invalid intent: {e}")))?;
            let conditions = serde_json::from_str(&conditions)
                .map_err(|e| CliError::usage(format!("invalid conditions: {e}")))?;
            let refresh = ctx
                .rt
                .block_on(ctx.service()?.refresh_app(intent, conditions))?;
            out.report(&RefreshReport { refresh })
        }
        DiagnosticsCommand::State { command } => {
            let service = ctx.service()?;
            let state = match command {
                Some(json) => {
                    let intent = serde_json::from_str(&json)
                        .map_err(|e| CliError::usage(format!("invalid command: {e}")))?;
                    ctx.rt.block_on(service.apply_diagnostic_command(intent))?
                }
                None => ctx.rt.block_on(service.diagnostic_state()),
            };
            out.report(&StateReport { state })
        }
        DiagnosticsCommand::SelfTest(args) => self_test(out, args),
        DiagnosticsCommand::Show(args) => show(ctx, out, args),
        DiagnosticsCommand::Bundle => bundle(ctx, out),
        DiagnosticsCommand::Configured { chain } => {
            let chain = resolve_chain(&chain)?;
            let report = ctx
                .rt
                .block_on(ctx.service()?.run_configured_self_tests(chain))?;
            let report = ConfiguredReport { report };
            out.report(&report)?;
            if report.ok() {
                Ok(())
            } else {
                Err(CliError::failure("Configured network self-tests failed"))
            }
        }
    }
}

/// What core's refresh policy would run under the given device conditions.
#[derive(Serialize)]
struct PlanReport {
    plan: MaintenancePlan,
}

impl Report for PlanReport {
    fn render(&self) {
        let plan = &self.plan;
        let yes = |value: bool| if value { "yes" } else { "no" };
        println!();
        out::wide_field(
            21,
            "pending transactions",
            yes(plan.refresh_pending_transactions),
        );
        out::wide_field(21, "live prices", yes(plan.refresh_live_prices));
        out::wide_field(21, "background tick", yes(plan.run_background_tick));
        out::wide_field(21, "heavy work", yes(plan.allow_heavy_background_work));
        out::wide_field(21, "poll every", &format!("{}s", plan.poll_seconds));
    }
}

/// What one refresh core ran did.
#[derive(Serialize)]
struct RefreshReport {
    refresh: AppRefreshResult,
}

impl Report for RefreshReport {
    fn render(&self) {
        let refresh = &self.refresh;
        println!();
        if let Some(pending) = &refresh.pending {
            out::wide_field(
                12,
                "pending",
                &format!(
                    "{} polled, {} changed",
                    pending.chains.len(),
                    pending.changes.len()
                ),
            );
        }
        out::wide_field(
            12,
            "alerts",
            &out::count(refresh.price_alerts.len(), "price alert"),
        );
        if let Some(movement) = &refresh.movement
            && movement.should_alert
        {
            out::wide_field(12, "movement", "past the threshold");
        }
        out::wide_field(12, "poll again", &format!("{}s", refresh.poll_seconds));
        for failure in &refresh.failures {
            println!("  {} {failure}", out::fail_mark());
        }
    }
}

/// The diagnostics core keeps across restarts: which networks are degraded,
/// when each last refreshed, and the log.
#[derive(Serialize)]
struct StateReport {
    state: DiagnosticState,
}

impl Report for StateReport {
    fn render(&self) {
        let state = &self.state;
        println!();
        let mut chains: Vec<(&Chain, &ChainDegradation)> = state.degraded.iter().collect();
        chains.sort_by_key(|(chain, _)| **chain);
        for (chain, degradation) in chains {
            let why = match degradation {
                ChainDegradation::HistoryRefreshFailed => "history unreadable".to_string(),
                ChainDegradation::HistoryPartiallyLoaded => "history partly loaded".to_string(),
                ChainDegradation::Failed { message } => message.clone(),
            };
            println!("  {} {}  {}", out::fail_mark(), out::chain(*chain), why);
        }
        for log in &state.logs {
            let level = match log.input.level {
                DiagnosticLogLevel::Debug => out::hint("debug"),
                DiagnosticLogLevel::Info => out::info("info"),
                DiagnosticLogLevel::Warning => out::accent("warn"),
                DiagnosticLogLevel::Error => out::down("error"),
            };
            println!(
                "  {:<5}  {}  {}",
                level,
                out::hint(&log.input.category),
                log.input.message
            );
        }
        println!();
        println!(
            "  {} degraded, {}",
            out::count(state.degraded.len(), "network"),
            out::count(state.logs.len(), "log entry")
        );
    }
}

#[derive(Serialize)]
struct CheckRow {
    name: String,
    passed: bool,
}

#[derive(Serialize)]
struct ChainChecks {
    chain: Chain,
    checks: Vec<CheckRow>,
}

/// Core's offline self-tests, per chain.
#[derive(Serialize)]
struct SelfTestReport {
    total: usize,
    failed: usize,
    chains: Vec<ChainChecks>,
}

impl Report for SelfTestReport {
    fn render(&self) {
        println!();
        for chain in &self.chains {
            let failed: Vec<&CheckRow> = chain.checks.iter().filter(|c| !c.passed).collect();
            println!(
                "  {}  {:<22} {}",
                if failed.is_empty() {
                    out::ok_mark()
                } else {
                    out::fail_mark()
                },
                chain.chain.chain_display_name().bold(),
                out::hint(&format!("{} checks", chain.checks.len())),
            );
            for check in failed {
                println!("     {} {}", out::fail_mark(), check.name);
            }
        }
        println!();
        println!(
            "  {} {}",
            out::accent(&format!("{}/{}", self.total - self.failed, self.total)).bold(),
            out::hint("checks passed"),
        );
    }

    fn ok(&self) -> bool {
        self.failed == 0
    }
}

fn self_test(out: Out, args: SelfTestArgs) -> CliResult<()> {
    // Core keys self-tests by chain id.
    let by_chain: HashMap<Chain, Vec<ChainSelfTestResult>> = match &args.chain {
        Some(name) => {
            let chain = resolve_chain(name)?;
            let results = spectra_core::diagnostics::self_tests::self_tests_run_chain(chain);
            if results.is_empty() {
                return Err(CliError::rejected(format!(
                    "{} has no self-tests",
                    chain.chain_display_name()
                )));
            }
            HashMap::from([(chain, results)])
        }
        None => spectra_core::diagnostics::self_tests::self_tests_run_all(),
    };
    let mut chains: Vec<ChainChecks> = by_chain
        .into_iter()
        .map(|(chain, results)| ChainChecks {
            chain,
            checks: results
                .into_iter()
                .map(|result| CheckRow {
                    name: result.name,
                    passed: result.passed,
                })
                .collect(),
        })
        .collect();
    chains.sort_by_key(|chain| chain.chain);
    let total = chains.iter().map(|chain| chain.checks.len()).sum();
    let failed = chains
        .iter()
        .flat_map(|chain| &chain.checks)
        .filter(|check| !check.passed)
        .count();
    out.report(&SelfTestReport {
        total,
        failed,
        chains,
    })?;
    if failed > 0 {
        return Err(CliError::failure(format!(
            "{failed} of {total} checks failed"
        )));
    }
    Ok(())
}

/// The self-tests run against the network and endpoint a chain is
/// configured to use.
#[derive(Serialize)]
struct ConfiguredReport {
    report: ConfiguredSelfTestReport,
}

impl Report for ConfiguredReport {
    fn render(&self) {
        let report = &self.report;
        println!();
        println!("  {}", out::chain(report.chain_id).bold());
        if let Some(endpoint) = &report.rpc_endpoint {
            out::field("endpoint", endpoint);
        }
        for result in &report.results {
            println!(
                "  {} {}",
                if result.passed {
                    out::ok_mark()
                } else {
                    out::fail_mark()
                },
                result.name
            );
        }
    }

    fn ok(&self) -> bool {
        self.report.results.iter().all(|result| result.passed)
    }
}

/// A chain's diagnostics document, as the app exports it.
#[derive(Serialize)]
struct DocumentReport {
    chain: &'static str,
    network: Chain,
    document: String,
}

impl Report for DocumentReport {
    fn render(&self) {
        println!("{}", self.document);
    }
}

fn show(ctx: &Ctx, out: Out, args: ShowArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let diagnostics = ctx.rt.block_on(ctx.service()?.chain_diagnostics(chain))?;
    out.report(&DocumentReport {
        chain: chain.str_id(),
        network: diagnostics.network_id,
        document: diagnostics.document,
    })
}

/// Every network's diagnostics document under one header, as the app
/// shares it.
#[derive(Serialize)]
struct BundleReport {
    bundle: String,
}

impl Report for BundleReport {
    fn render(&self) {
        println!("{}", self.bundle);
    }
}

fn bundle(ctx: &Ctx, out: Out) -> CliResult<()> {
    let platform = spectra_core::service::DiagnosticsPlatformInfo {
        app_version: env!("CARGO_PKG_VERSION").into(),
        build_number: "cli".into(),
        os_version: std::env::consts::OS.into(),
        locale_identifier: std::env::var("LANG").unwrap_or_default(),
        time_zone_identifier: std::env::var("TZ").unwrap_or_default(),
    };
    let bundle = ctx
        .rt
        .block_on(ctx.service()?.diagnostics_bundle(platform))?;
    out.report(&BundleReport { bundle })
}

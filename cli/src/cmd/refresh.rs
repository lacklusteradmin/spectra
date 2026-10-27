//! `RefreshEngine` owns the app's refresh loops; the CLI reports no device
//! conditions, so it runs none of them and asks for one sweep, awaited —
//! `trigger_immediate` spawns and returns, which suits an app and abandons the
//! work of a process about to exit.

use std::sync::{Arc, Mutex};

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::ChainEndpoints;
use spectra_core::service::app_refresh::AppRefreshResult;
use spectra_core::service::refresh_engine::{RefreshEngine, RefreshObserver};
use spectra_core::store::state::WalletState;

use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Args)]
pub struct RefreshArgs {
    /// Explicit balance endpoint for the selected wallet (also supports loopback fixtures).
    #[arg(long, requires = "wallet")]
    endpoint: Option<String>,
    /// Only this wallet (id, name or address). Default: every stored wallet.
    #[arg(long)]
    wallet: Option<String>,
}

/// What the engine reported, collected for one sweep.
#[derive(Default)]
struct Collected {
    updated: Vec<(String, Option<WalletState>)>,
    refreshed: u32,
    errors: u32,
    complete: bool,
}

struct Collector(Mutex<Collected>);

impl RefreshObserver for Collector {
    fn on_balance_updated(
        &self,
        _chain_id: spectra_core::registry::Chain,
        wallet_id: String,
        summary: Option<WalletState>,
    ) {
        self.0.lock().unwrap().updated.push((wallet_id, summary));
    }

    fn on_refresh_cycle_complete(&self, refreshed: u32, errors: u32) {
        let mut state = self.0.lock().unwrap();
        state.refreshed = refreshed;
        state.errors = errors;
        state.complete = true;
    }

    /// Never called: the CLI reports no device conditions.
    fn on_refresh_complete(&self, _result: AppRefreshResult) {}

    /// Never called, for the same reason.
    fn on_tor_status_changed(&self, _status: spectra_core::api::tor::TorStatus) {}
}

pub fn refresh(ctx: &Ctx, out: Out, args: RefreshArgs) -> CliResult<()> {
    let state = ctx.state()?;
    let wallets: Vec<_> = match &args.wallet {
        Some(needle) => vec![ctx.find_wallet(needle)?],
        None => state.wallets.clone(),
    };
    if wallets.is_empty() {
        return Err(CliError::rejected("no wallets to refresh"));
    }

    let service = ctx.service()?;
    if let Some(endpoint) = args.endpoint {
        let chain = wallets[0].chain_id;
        ctx.rt
            .block_on(service.update_endpoints(vec![ChainEndpoints {
                capabilities: spectra_core::api::endpoint::EndpointCapability::ALL.to_vec(),
                chain_id: chain,
                endpoints: vec![endpoint],
            }]))
            .map_err(CliError::from)?;
    }

    let engine = RefreshEngine::new(service);
    let collector = Arc::new(Collector(Mutex::new(Collected::default())));
    engine.set_observer(collector.clone());
    // Core builds refresh entries from its wallets and their networks.
    let entry_count = ctx
        .rt
        .block_on(engine.sync_entries(args.wallet.as_ref().map(|_| wallets[0].id.clone())));
    if entry_count == 0 {
        return Err(CliError::rejected("no wallet has an address to refresh"));
    }

    out::progress(&format!(
        "refreshing {}",
        out::count(entry_count as usize, "wallet")
    ));
    // One sweep, awaited: this process is about to exit, so a spawned
    // cycle would be abandoned mid-flight.
    ctx.rt.block_on(engine.refresh_now());
    engine.clear_observer();

    let collected = collector.0.lock().unwrap();
    out.report(&RefreshReport {
        refreshed: collected.refreshed,
        errors: collected.errors,
        cycle_completed: collected.complete,
        skipped: wallets.len().saturating_sub(entry_count as usize),
        wallets: collected
            .updated
            .iter()
            .map(|(wallet_id, summary)| UpdatedWallet {
                wallet: wallet_id.clone(),
                updated: summary.is_some(),
                name: wallets
                    .iter()
                    .find(|wallet| &wallet.id == wallet_id)
                    .map_or_else(|| wallet_id.clone(), |wallet| wallet.name.clone()),
                chain: summary.as_ref().map(|summary| summary.chain_id),
                is_watch_only: summary.as_ref().is_some_and(WalletState::is_watch_only),
            })
            .collect(),
    })
}

#[derive(Serialize)]
struct UpdatedWallet {
    wallet: String,
    updated: bool,
    #[serde(skip)]
    name: String,
    #[serde(skip)]
    chain: Option<Chain>,
    #[serde(skip)]
    is_watch_only: bool,
}

/// What one sweep refreshed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshReport {
    refreshed: u32,
    errors: u32,
    cycle_completed: bool,
    skipped: usize,
    wallets: Vec<UpdatedWallet>,
}

impl Report for RefreshReport {
    fn render(&self) {
        println!();
        for wallet in &self.wallets {
            match wallet.chain {
                Some(chain) => println!(
                    "  {}  {:<18} {}",
                    out::wallet_dot(chain, wallet.is_watch_only),
                    wallet.name,
                    out::hint(chain.chain_display_name()),
                ),
                None => println!(
                    "  {}  {:<18} {}",
                    out::fail_mark(),
                    wallet.name,
                    out::hint("no balance")
                ),
            }
        }
        println!();
        println!(
            "  {} refreshed, {} errors",
            out::accent(&self.refreshed.to_string()).bold(),
            if self.errors == 0 {
                self.errors.to_string().normal()
            } else {
                self.errors.to_string().red().bold()
            },
        );
    }

    fn ok(&self) -> bool {
        self.errors == 0
    }
}

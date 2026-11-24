//! `RefreshEngine` owns the app's refresh loops; the CLI reports no device
//! conditions, so it runs none of them and asks for one sweep, awaited —
//! `trigger_immediate` spawns and returns, which suits an app and abandons the
//! work of a process about to exit.

use std::sync::{Arc, Mutex};

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::WalletService;
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

/// What one sweep through core's engine reported.
#[derive(Default)]
pub(crate) struct Sweep {
    /// The refresh entries core built: one per wallet with an address.
    pub entries: u32,
    /// Each wallet the sweep read, with why its balance could not be read.
    pub wallets: Vec<(String, Option<String>)>,
    pub refreshed: u32,
    pub errors: u32,
    pub complete: bool,
}

struct Collector(Mutex<Sweep>);

impl RefreshObserver for Collector {
    fn on_balance_refreshed(
        &self,
        _chain_id: spectra_core::registry::Chain,
        wallet_id: String,
        failure: Option<String>,
    ) {
        self.0.lock().unwrap().wallets.push((wallet_id, failure));
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

/// One balance sweep through core's `RefreshEngine`, of every wallet or of
/// `wallet_id`, awaited: this process is about to exit, so a spawned cycle
/// would be abandoned mid-flight.
pub(crate) fn sweep(ctx: &Ctx, service: Arc<WalletService>, wallet_id: Option<String>) -> Sweep {
    let engine = RefreshEngine::new(service);
    let collector = Arc::new(Collector(Mutex::new(Sweep::default())));
    engine.set_observer(collector.clone());
    // Core builds refresh entries from its wallets and their networks.
    let entries = ctx.rt.block_on(engine.sync_entries(wallet_id));
    if entries > 0 {
        out::progress(&format!(
            "refreshing {}",
            out::count(entries as usize, "wallet")
        ));
        ctx.rt.block_on(engine.refresh_now());
    }
    engine.clear_observer();
    let mut sweep = std::mem::take(&mut *collector.0.lock().unwrap());
    sweep.entries = entries;
    sweep
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

    let service =
        ctx.service_with_endpoints(wallets[0].chain_id, args.endpoint.into_iter().collect())?;

    let swept = sweep(
        ctx,
        service,
        args.wallet.as_ref().map(|_| wallets[0].id.clone()),
    );
    if swept.entries == 0 {
        return Err(CliError::rejected("no wallet has an address to refresh"));
    }
    out.report(&RefreshReport {
        refreshed: swept.refreshed,
        errors: swept.errors,
        cycle_completed: swept.complete,
        skipped: wallets.len().saturating_sub(swept.entries as usize),
        wallets: swept
            .wallets
            .into_iter()
            .map(|(wallet_id, failure)| {
                let wallet = wallets.iter().find(|wallet| wallet.id == wallet_id);
                UpdatedWallet {
                    updated: failure.is_none(),
                    failure,
                    name: wallet.map_or_else(|| wallet_id.clone(), |wallet| wallet.name.clone()),
                    chain: wallet.map(|wallet| wallet.chain_id),
                    is_watch_only: wallet.is_some_and(WalletState::is_watch_only),
                    wallet: wallet_id,
                }
            })
            .collect(),
    })
}

#[derive(Serialize)]
struct UpdatedWallet {
    wallet: String,
    updated: bool,
    /// Why the balance could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
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
            match (&wallet.failure, wallet.chain) {
                (None, Some(chain)) => println!(
                    "  {}  {:<18} {}",
                    out::wallet_dot(chain, wallet.is_watch_only),
                    wallet.name,
                    out::hint(chain.chain_display_name()),
                ),
                (failure, _) => println!(
                    "  {}  {:<18} {}",
                    out::fail_mark(),
                    wallet.name,
                    out::hint(failure.as_deref().unwrap_or("no balance"))
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

    fn failure(&self) -> Option<String> {
        (self.errors > 0).then(|| format!("{} wallets failed to refresh", self.errors))
    }
}

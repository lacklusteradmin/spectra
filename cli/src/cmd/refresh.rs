//! `BalanceRefreshEngine` is the one subsystem where Rust already owns the
//! loop; this is the second `BalanceObserver` it has ever had.
//!
//! One sweep, awaited — `trigger_immediate` spawns and returns, which suits an
//! app and abandons the work of a process about to exit.

use std::sync::{Arc, Mutex};

use clap::Args;
use colored::Colorize as _;
use spectra_core::fetch::refresh::engine::{BalanceObserver, BalanceRefreshEngine};
use spectra_core::service::ChainEndpoints;
use spectra_core::store::state::WalletSummary;

use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

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
    updated: Vec<(String, Option<WalletSummary>)>,
    refreshed: u32,
    errors: u32,
    complete: bool,
}

struct Collector(Mutex<Collected>);

impl BalanceObserver for Collector {
    fn on_balance_updated(
        &self,
        _chain_id: String,
        wallet_id: String,
        summary: Option<WalletSummary>,
    ) {
        self.0.lock().unwrap().updated.push((wallet_id, summary));
    }

    fn on_refresh_cycle_complete(&self, refreshed: u32, errors: u32) {
        let mut state = self.0.lock().unwrap();
        state.refreshed = refreshed;
        state.errors = errors;
        state.complete = true;
    }
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
        let chain = wallets[0]
            .network_chain(&state.settings)
            .ok_or_else(|| CliError::rejected("unknown wallet network"))?;
        ctx.rt
            .block_on(service.update_endpoints_typed(vec![ChainEndpoints {
                chain_id: chain.str_id().into(),
                endpoints: vec![endpoint],
                api_key: None,
            }]))
            .map_err(CliError::from)?;
    }

    let engine = BalanceRefreshEngine::new(service);
    let collector = Arc::new(Collector(Mutex::new(Collected::default())));
    engine.set_observer(collector.clone());
    // Core builds the list from the wallets it holds. This command used to
    // build its own — a third copy of the rule beside the app's and core's, and
    // it disagreed with both: it took any wallet's `xpub` as the fetch key
    // rather than only Bitcoin's, and the wallet's first stored address rather
    // than the one for the network it is on.
    let entry_count = ctx
        .rt
        .block_on(engine.sync_entries(args.wallet.as_ref().map(|_| wallets[0].id.clone())));
    if entry_count == 0 {
        return Err(CliError::rejected("no wallet has an address to refresh"));
    }

    out.text(|| {
        println!(
            "  {} refreshing {} wallet{}…",
            out::hint("→"),
            entry_count as usize,
            if entry_count as usize == 1 { "" } else { "s" }
        )
    });
    // One sweep, awaited: this process is about to exit, so a spawned
    // cycle would be abandoned mid-flight.
    ctx.rt.block_on(engine.refresh_now());
    engine.clear_observer();

    let collected = collector.0.lock().unwrap();
    out.text(|| {
        println!();
        for (wallet_id, summary) in &collected.updated {
            let name = wallets
                .iter()
                .find(|wallet| &wallet.id == wallet_id)
                .map(|wallet| wallet.name.clone())
                .unwrap_or_else(|| wallet_id.clone());
            match summary {
                Some(summary) => println!(
                    "  {}  {:<18} {}",
                    out::wallet_dot(&summary.chain_name, summary.is_watch_only),
                    name,
                    out::hint(&summary.chain_name),
                ),
                None => println!(
                    "  {}  {:<18} {}",
                    out::fail_mark(),
                    name,
                    out::hint("no balance")
                ),
            }
        }
        println!();
        println!(
            "  {} refreshed, {} errors",
            out::accent(&collected.refreshed.to_string()).bold(),
            if collected.errors == 0 {
                collected.errors.to_string().normal()
            } else {
                collected.errors.to_string().red().bold()
            },
        );
    });
    out.emit(serde_json::json!({
        "ok": collected.errors == 0,
        "refreshed": collected.refreshed,
        "errors": collected.errors,
        "cycleCompleted": collected.complete,
        "skipped": wallets.len().saturating_sub(entry_count as usize),
    }));
    Ok(())
}

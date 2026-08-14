//! `BalanceRefreshEngine` is the one subsystem where Rust already owns the
//! loop; this is the second `BalanceObserver` it has ever had.
//!
//! One sweep, awaited — `trigger_immediate` spawns and returns, which suits an
//! app and abandons the work of a process about to exit.

use std::sync::{Arc, Mutex};

use clap::Args;
use colored::Colorize as _;
use spectra_core::fetch::refresh_engine::{BalanceObserver, BalanceRefreshEngine, RefreshEntry};
use spectra_core::service::{ChainEndpoints, WalletService};
use spectra_core::store::state::WalletSummary;

use super::chain::{BALANCE, RPC};
use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Args)]
pub struct RefreshArgs {
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

    // Endpoints for every chain represented, gathered once. The engine takes
    // them at construction the same way the app supplies them.
    let mut chain_endpoints = Vec::new();
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for wallet in &wallets {
        let Ok(chain) = resolve_chain(&wallet.chain_name) else {
            skipped.push(wallet.name.clone());
            continue;
        };
        let endpoints: Vec<String> = spectra_core::app_core_endpoint_records_for_chain(
            chain.chain_display_name().to_string(),
            BALANCE | RPC,
            false,
        )
        .map_err(CliError::from)?
        .into_iter()
        .map(|record| record.endpoint)
        .collect();
        if endpoints.is_empty() {
            skipped.push(wallet.name.clone());
            continue;
        }
        if !chain_endpoints
            .iter()
            .any(|existing: &ChainEndpoints| existing.chain_id == chain.str_id())
        {
            chain_endpoints.push(ChainEndpoints {
                chain_id: chain.str_id().to_string(),
                endpoints,
                api_key: None,
            });
        }
        entries.push(RefreshEntry {
            chain_id: chain.str_id().to_string(),
            wallet_id: wallet.id.clone(),
            // Bitcoin HD wallets refresh by xpub; the engine detects that.
            address: wallet
                .xpub
                .clone()
                .unwrap_or_else(|| wallet_address(wallet).to_string()),
        });
    }
    if entries.is_empty() {
        return Err(CliError::rejected(
            "no wallet has a chain with balance endpoints",
        ));
    }

    let service = WalletService::new_typed(chain_endpoints).map_err(CliError::from)?;
    ctx.rt
        .block_on(service.open_state(ctx.db_path()))
        .map_err(CliError::from)?;

    let engine = BalanceRefreshEngine::new(service);
    let collector = Arc::new(Collector(Mutex::new(Collected::default())));
    engine.set_observer(collector.clone());
    engine.set_entries_typed(entries);

    out.text(|| {
        println!(
            "  {} refreshing {} wallet{}…",
            out::hint("→"),
            wallets.len() - skipped.len(),
            if wallets.len() - skipped.len() == 1 { "" } else { "s" }
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
                None => println!("  {}  {:<18} {}", out::fail_mark(), name, out::hint("no balance")),
            }
        }
        for name in &skipped {
            println!("  {}  {:<18} {}", out::hint("·"), name, out::hint("skipped"));
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
        "skipped": skipped,
    }));
    Ok(())
}

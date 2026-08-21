//! Commands that talk to a chain: the supported list, balances and history.
//!
//! Endpoint selection is core's — `app_core_endpoint_records_for_chain` picks
//! them from the catalog by role. The CLI supplies the role mask for what it
//! is about to do and nothing else.

use clap::Args;
use colored::Colorize as _;
use spectra_core::registry::Chain;
use spectra_core::service::{ChainEndpoints, WalletService};
use std::sync::Arc;

use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

/// Endpoint role bits, as `AppEndpointDirectory` defines them.
mod role {
    pub const BALANCE: u32 = 1 << 1;
    pub const HISTORY: u32 = 1 << 2;
    pub const UTXO: u32 = 1 << 3;
    pub const FEE: u32 = 1 << 4;
    pub const BROADCAST: u32 = 1 << 5;
    pub const RPC: u32 = 1 << 7;
}

pub use role::{BALANCE, BROADCAST, FEE, HISTORY, RPC, UTXO};

#[derive(Args)]
pub struct ChainsArgs {
    /// Only chains whose name or symbol contains this text.
    #[arg(long)]
    filter: Option<String>,
    /// Include testnets.
    #[arg(long)]
    testnets: bool,
}

#[derive(Args)]
pub struct BalanceArgs {
    /// Wallet id, name or address.
    wallet: String,
}

#[derive(Args)]
pub struct HistoryArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Most entries to show.
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

/// A service bound to one chain's endpoints for the roles a command needs.
pub fn service_for_chain(chain: Chain, roles: u32) -> CliResult<Arc<WalletService>> {
    let name = chain.chain_display_name().to_string();
    let endpoints: Vec<String> =
        spectra_core::endpoint_records_for_chain_masked(name.clone(), roles, false)
            .map_err(CliError::from)?
            .into_iter()
            .map(|record| record.endpoint)
            .collect();
    if endpoints.is_empty() {
        return Err(CliError::failure(format!("no endpoints registered for {name}")));
    }
    WalletService::new_typed(vec![ChainEndpoints {
        chain_id: chain.str_id().to_string(),
        endpoints,
        api_key: None,
    }])
    .map_err(CliError::from)
}

pub fn chains(out: Out, args: ChainsArgs) -> CliResult<()> {
    let needle = args.filter.as_deref().map(str::to_lowercase);
    let listed: Vec<Chain> = Chain::all()
        .filter(|chain| args.testnets || chain.mainnet_counterpart() == *chain)
        .filter(|chain| match &needle {
            None => true,
            Some(needle) => {
                chain.chain_display_name().to_lowercase().contains(needle)
                    || chain.coin_symbol().to_lowercase().contains(needle)
            }
        })
        .collect();

    out.text(|| {
        println!();
        for chain in &listed {
            println!(
                "  {}  {:<22} {:<8} {}",
                out::tint("●", chain.chain_display_name()).bold(),
                out::tint(chain.chain_display_name(), chain.chain_display_name()),
                chain.coin_symbol(),
                out::hint(chain.str_id()),
            );
        }
        println!();
        println!(
            "  {} {}",
            out::accent(&listed.len().to_string()).bold(),
            out::hint("chains")
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chains": listed
            .iter()
            .map(|chain| serde_json::json!({
                "id": chain.str_id(),
                "name": chain.chain_display_name(),
                "symbol": chain.coin_symbol(),
                "isEvm": chain.is_evm(),
                // The import picker's list, as a column rather than a second
                // array: a chain is offered for private-key import exactly
                // when a key derives an address on it.
                "privateKeyImport": chain.derives_from_private_key(),
                // Likewise the watch-addresses picker: the app rendered a
                // hand-written eighteen-section list against this flag and
                // disagreed with it in both directions.
                "watchOnlyImport": chain.supports_watch_only_import(),
                // And the staking tab's, which was a seven-case Swift enum and
                // two match arms in `StakingService` before it was a column.
                "staking": chain.supports_staking(),
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

pub fn balance(ctx: &Ctx, out: Out, args: BalanceArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = resolve_chain(&wallet.chain_name)?;
    let service = service_for_chain(chain, BALANCE | RPC)?;

    let summary = ctx
        .rt
        .block_on(service.fetch_native_balance_summary(
            chain.str_id().to_string(),
            wallet_address(&wallet).to_string(),
        ))
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        println!(
            "  {}  {} {}",
            out::wallet_dot(&wallet.chain_name, wallet.is_watch_only),
            summary.amount_display.bold(),
            out::tint(chain.coin_symbol(), &wallet.chain_name).bold(),
        );
        out::field("raw", &out::hint(&summary.smallest_unit).to_string());
        if summary.utxo_count > 0 {
            out::field("utxos", &summary.utxo_count.to_string());
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "chain": chain.chain_display_name(),
        "symbol": chain.coin_symbol(),
        "amount": summary.amount_display,
        "smallestUnit": summary.smallest_unit,
        "utxoCount": summary.utxo_count,
    }));
    Ok(())
}

pub fn history(ctx: &Ctx, out: Out, args: HistoryArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = resolve_chain(&wallet.chain_name)?;
    let service = service_for_chain(chain, HISTORY | BALANCE | RPC)?;

    let entries = ctx
        .rt
        .block_on(service.fetch_normalized_history(
            chain.str_id().to_string(),
            wallet_address(&wallet).to_string(),
        ))
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        if entries.is_empty() {
            println!("  {}", out::hint("no transactions"));
            return;
        }
        for entry in entries.iter().take(args.limit) {
            let incoming = entry.kind.eq_ignore_ascii_case("receive");
            let (mark, amount) = if incoming {
                ("↓", format!("{:>12.4}", entry.amount).truecolor(120, 230, 160))
            } else {
                ("↑", format!("{:>12.4}", entry.amount).truecolor(255, 110, 130))
            };
            println!(
                "  {}  {} {}  {}  {}",
                if incoming {
                    mark.truecolor(120, 230, 160).bold()
                } else {
                    mark.truecolor(255, 110, 130).bold()
                },
                amount.bold(),
                out::tint(&entry.symbol, &wallet.chain_name),
                out::info(&entry.counterparty),
                out::hint(&out::relative_time(entry.timestamp as i64)),
            );
            println!("     {}", out::hint(&out::short_hash(&entry.tx_hash)));
        }
        if entries.len() > args.limit {
            println!();
            println!(
                "  {}",
                out::hint(&format!("+{} more", entries.len() - args.limit))
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "count": entries.len(),
        "transactions": entries
            .iter()
            .take(args.limit)
            .map(|entry| serde_json::json!({
                "hash": entry.tx_hash,
                "kind": entry.kind,
                "amount": entry.amount,
                "symbol": entry.symbol,
                "counterparty": entry.counterparty,
                "timestamp": entry.timestamp,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

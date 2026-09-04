//! Keypool: which receive index a wallet is on, and reserving the next one.
//!
//! Reserving is read-modify-write, and core holds one write lock across the
//! whole operation precisely because two callers racing there hand the same
//! receive address to two people. That guarantee had one caller; a second
//! process is the only way to actually test it.

use clap::{Args, Subcommand};
use colored::Colorize as _;

use crate::ctx::Ctx;
use crate::error::CliResult;
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum PoolCommand {
    /// Where a wallet's receive and change indices stand.
    Show(SelectArgs),
    /// Reserve the next receive index.
    Next(SelectArgs),
    /// Reserve the next change index. Always consumes one.
    NextChange(SelectArgs),
    /// Walk this wallet's derived addresses and record the used ones.
    Discover(SelectArgs),
}

#[derive(Args)]
pub struct SelectArgs {
    /// Wallet id, name or address.
    wallet: String,
}

pub fn run(ctx: &Ctx, out: Out, command: PoolCommand) -> CliResult<()> {
    match command {
        PoolCommand::Show(args) => show(ctx, out, args),
        PoolCommand::Next(args) => next(ctx, out, args, false),
        PoolCommand::NextChange(args) => next(ctx, out, args, true),
        PoolCommand::Discover(args) => discover(ctx, out, args),
    }
}

/// The address walk the app runs on every UTXO refresh.
///
/// It lived in Swift because it needed the seed phrase there. Core reads the
/// seed, the derivation path, the keypool bound, the balance and the history,
/// so the loop is here and the phrase does not leave the crate.
fn discover(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = super::resolve_chain(&wallet.chain_name)?;
    let service = super::chain::service_for_chain(chain, super::chain::BALANCE | super::chain::HISTORY | super::chain::RPC)?;

    let addresses = ctx
        .rt
        .block_on(service.discover_utxo_addresses(wallet.id.clone(), chain.str_id().to_string()))
        .map_err(crate::error::CliError::from)?;

    out.text(|| {
        println!();
        out::field("wallet", &wallet.name.bold().to_string());
        out::field("addresses", &addresses.len().to_string());
        for address in &addresses {
            println!("  {}", out::hint(address));
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "chain": chain.chain_display_name(),
        "addressCount": addresses.len(),
        "addresses": addresses,
    }));
    Ok(())
}

fn show(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let service = ctx.service()?;
    let state = ctx
        .rt
        .block_on(service.keypool_state(wallet.id.clone(), wallet.chain_name.clone()));

    out.text(|| {
        println!();
        out::field("wallet", &wallet.name.bold().to_string());
        out::field("chain", &out::tint(&wallet.chain_name, &wallet.chain_name).to_string());
        out::field("receive", &state.next_external_index.to_string());
        out::field("change", &state.next_change_index.to_string());
        out::field(
            "reserved",
            &state
                .reserved_receive_index
                .map(|i| i.to_string())
                .unwrap_or_else(|| "none".into()),
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "chain": wallet.chain_name,
        "nextExternalIndex": state.next_external_index,
        "nextChangeIndex": state.next_change_index,
        "reservedReceiveIndex": state.reserved_receive_index,
    }));
    Ok(())
}

fn next(ctx: &Ctx, out: Out, args: SelectArgs, change: bool) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let service = ctx.service()?;
    let reserved = ctx
        .rt
        .block_on(async {
            if change {
                service
                    .reserve_change_index(wallet.id.clone(), wallet.chain_name.clone())
                    .await
            } else {
                service
                    .reserve_receive_index(wallet.id.clone(), wallet.chain_name.clone(), 0)
                    .await
            }
        })
        .map_err(crate::error::CliError::from)?;

    out.text(|| {
        println!(
            "  {} reserved {} index {}",
            out::ok_mark(),
            if change { "change" } else { "receive" },
            reserved.to_string().bold(),
        )
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "kind": if change { "change" } else { "receive" },
        "index": reserved,
    }));
    Ok(())
}

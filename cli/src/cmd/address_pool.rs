//! Keypool: which receive index a wallet is on, and reserving the next one.
//!
//! Reserving is read-modify-write, and core holds one write lock across the
//! whole operation precisely because two callers racing there hand the same
//! receive address to two people. That guarantee had one caller; a second
//! process is the only way to actually test it.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::ChainKeypoolStateRecord;

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
    }
}

/// Nothing to seed the pool from on this side — core merges the stored state
/// over it, so an all-zero baseline reads whatever is already there.
fn baseline() -> ChainKeypoolStateRecord {
    ChainKeypoolStateRecord {
        next_external_index: 0,
        next_change_index: 0,
        reserved_receive_index: None,
    }
}

fn show(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let service = ctx.service()?;
    let state = ctx
        .rt
        .block_on(service.keypool_state(
            wallet.id.clone(),
            wallet.chain_name.clone(),
            baseline(),
        ))
        .map_err(crate::error::CliError::from)?;

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
                    .reserve_change_index(wallet.id.clone(), wallet.chain_name.clone(), baseline())
                    .await
            } else {
                service
                    .reserve_receive_index(
                        wallet.id.clone(),
                        wallet.chain_name.clone(),
                        baseline(),
                        0,
                    )
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

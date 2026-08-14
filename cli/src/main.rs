//! `spectra` — a front end for `spectra_core`, and the check that core needs
//! no platform.
//!
//! Every command here is non-interactive by default and scriptable: arguments
//! select, `--json` reports, and exit codes distinguish "core said no" from
//! "something broke". That is deliberate. `PLAN.md` rule 1 makes this CLI the
//! acceptance gate for logic moving out of Swift — "if `spectra` cannot drive
//! it, it is in the wrong place" — and the previous version could only be
//! driven by a person typing into a prompt, which is not a gate.

mod cmd;
mod ctx;
mod error;
mod out;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use ctx::Ctx;
use error::CliError;
use out::Out;

#[derive(Parser)]
#[command(
    name = "spectra",
    about = "Multi-chain self-custody wallet",
    version,
    disable_help_subcommand = true
)]
struct Cli {
    /// Emit machine-readable JSON instead of formatted text.
    #[arg(long, global = true)]
    json: bool,

    /// Wallet data directory (default: $SPECTRA_DATA_DIR, else ~/.spectra).
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create, import and inspect wallets.
    #[command(subcommand)]
    Wallet(cmd::wallet::WalletCommand),
    /// Validate addresses and manage saved recipients.
    #[command(subcommand)]
    Address(cmd::address::AddressCommand),
    /// Chains this build supports.
    Chains(cmd::chain::ChainsArgs),
    /// Fetch a wallet's on-chain balance.
    Balance(cmd::chain::BalanceArgs),
    /// Fetch a wallet's on-chain transaction history.
    History(cmd::chain::HistoryArgs),
    /// Transactions recorded in the local store.
    Txs(cmd::tx::TxsArgs),
    /// Sign and broadcast a transfer.
    Send(cmd::tx::SendArgs),
    /// Spot price for a chain's native asset.
    Price(cmd::market::PriceArgs),
    /// Total holdings across every wallet.
    Portfolio(cmd::market::PortfolioArgs),
    /// Read or set the display currency.
    Currency(cmd::market::CurrencyArgs),
}

fn main() {
    let cli = Cli::parse();
    let out = Out::new(cli.json);

    let result = Ctx::new(cli.data_dir).and_then(|ctx| dispatch(&ctx, out, cli.command));

    if let Err(error) = result {
        out.text(|| eprintln!("  {} {}", out::fail_mark(), error));
        out.emit(serde_json::json!({ "ok": false, "error": error.message }));
        std::process::exit(error.code);
    }
}

fn dispatch(ctx: &Ctx, out: Out, command: Command) -> Result<(), CliError> {
    match command {
        Command::Wallet(command) => cmd::wallet::run(ctx, out, command),
        Command::Address(command) => cmd::address::run(ctx, out, command),
        Command::Chains(args) => cmd::chain::chains(out, args),
        Command::Balance(args) => cmd::chain::balance(ctx, out, args),
        Command::History(args) => cmd::chain::history(ctx, out, args),
        Command::Txs(args) => cmd::tx::txs(ctx, out, args),
        Command::Send(args) => cmd::tx::send(ctx, out, args),
        Command::Price(args) => cmd::market::price(ctx, out, args),
        Command::Portfolio(args) => cmd::market::portfolio(ctx, out, args),
        Command::Currency(args) => cmd::market::currency(ctx, out, args),
    }
}

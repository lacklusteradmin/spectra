//! `spectra` — a front end for `spectra_core`, and the check that core needs
//! no platform.
//!
//! Every command is non-interactive and scriptable, because `PLAN.md` rule 1
//! makes this CLI the acceptance gate: a rule it cannot drive is in the wrong
//! place. The previous version was a REPL, which cannot gate anything.

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
    /// Call every registered endpoint and report which ones answer.
    Endpoints(cmd::chain::EndpointsArgs),
    /// Fetch a wallet's on-chain balance.
    Balance(cmd::chain::BalanceArgs),
    /// Fetch a wallet's on-chain transaction history.
    History(cmd::chain::HistoryArgs),
    /// Transactions recorded in the local store.
    Txs(cmd::tx::TxsArgs),
    /// Assemble a transfer, or sign and broadcast one.
    #[command(subcommand)]
    Send(cmd::tx::SendCommand),
    /// Spot price for a chain's native asset.
    Price(cmd::market::PriceArgs),
    /// Total holdings across every wallet.
    Portfolio(cmd::market::PortfolioArgs),
    /// Read or set the display currency.
    Currency(cmd::market::CurrencyArgs),
    /// Which network of a family each chain is on.
    #[command(subcommand)]
    Network(cmd::network::NetworkCommand),
    /// Settings core owns: providers, endpoints, fee priorities, alert rules.
    #[command(subcommand)]
    Settings(cmd::settings::SettingsCommand),
    /// Validators and staked positions.
    #[command(subcommand)]
    Staking(cmd::staking::StakingCommand),
    /// The token catalog, and which tokens this wallet tracks.
    #[command(subcommand)]
    Token(cmd::token::TokenCommand),
    /// Core's self-tests and diagnostics documents.
    #[command(subcommand)]
    Diagnostics(cmd::diagnostics::DiagnosticsCommand),
    /// Run one balance-refresh sweep through core's engine.
    Refresh(cmd::refresh::RefreshArgs),
    /// Search a seed's derivation paths for funded addresses.
    Rescan(cmd::rescan::RescanArgs),
    /// A wallet's receive/change index pool.
    #[command(subcommand)]
    Pool(cmd::address_pool::PoolCommand),
    /// Price alerts.
    #[command(subcommand)]
    Alert(cmd::alert::AlertCommand),
}

fn main() {
    let cli = Cli::parse();
    let out = Out::new(cli.json);

    let result = Ctx::new(cli.data_dir).and_then(|ctx| dispatch(&ctx, out, cli.command));

    if let Err(error) = result {
        out.text(|| eprintln!("  {} {}", out::fail_mark(), error));
        if !error.already_emitted {
            out.emit(serde_json::json!({ "ok": false, "error": error.message }));
        }
        std::process::exit(error.code);
    }
}

fn dispatch(ctx: &Ctx, out: Out, command: Command) -> Result<(), CliError> {
    match command {
        Command::Wallet(command) => cmd::wallet::run(ctx, out, command),
        Command::Address(command) => cmd::address::run(ctx, out, command),
        Command::Chains(args) => cmd::chain::chains(out, args),
        Command::Endpoints(args) => cmd::chain::endpoints(ctx, out, args),
        Command::Balance(args) => cmd::chain::balance(ctx, out, args),
        Command::History(args) => cmd::chain::history(ctx, out, args),
        Command::Txs(args) => cmd::tx::txs(ctx, out, args),
        Command::Send(command) => cmd::tx::run(ctx, out, command),
        Command::Price(args) => cmd::market::price(ctx, out, args),
        Command::Portfolio(args) => cmd::market::portfolio(ctx, out, args),
        Command::Currency(args) => cmd::market::currency(ctx, out, args),
        Command::Network(command) => cmd::network::run(ctx, out, command),
        Command::Settings(command) => cmd::settings::run(ctx, out, command),
        Command::Staking(command) => cmd::staking::run(ctx, out, command),
        Command::Token(command) => cmd::token::run(ctx, out, command),
        Command::Diagnostics(command) => cmd::diagnostics::run(ctx, out, command),
        Command::Refresh(args) => cmd::refresh::refresh(ctx, out, args),
        Command::Rescan(args) => cmd::rescan::rescan(ctx, out, args),
        Command::Pool(command) => cmd::address_pool::run(ctx, out, command),
        Command::Alert(command) => cmd::alert::run(ctx, out, command),
    }
}

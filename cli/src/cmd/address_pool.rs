//! Keypool: which receive index a wallet is on, and reserving the next one.
//!
//! Reserving is read-modify-write, and core holds one write lock across the
//! whole operation precisely because two callers racing there hand the same
//! receive address to two people. That guarantee had one caller; a second
//! process is the only way to actually test it.

use clap::Subcommand;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::WalletAddressDiscovery;

use crate::ctx::{Ctx, WalletArg};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Subcommand)]
pub enum PoolCommand {
    /// Where a wallet's receive and change indices stand.
    Show(WalletArg),
    /// Discover addresses for all wallets on a chain.
    DiscoverChain { chain: String },
    /// Reserve the next receive index.
    Next(WalletArg),
    /// Reserve the next change index. Always consumes one.
    NextChange(WalletArg),
    /// Walk this wallet's derived addresses and record the used ones.
    Discover(WalletArg),
}

pub fn run(ctx: &Ctx, out: Out, command: PoolCommand) -> CliResult<()> {
    match command {
        PoolCommand::Show(args) => show(ctx, out, args),
        PoolCommand::DiscoverChain { chain } => {
            let chain = super::resolve_chain(&chain)?;
            let results = ctx
                .rt
                .block_on(ctx.service()?.discover_chain_addresses(chain))?;
            out.report(&ChainDiscoveryReport { results })
        }
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
/// The addresses discovery recorded for every wallet on a chain.
#[derive(Serialize)]
struct ChainDiscoveryReport {
    results: Vec<WalletAddressDiscovery>,
}

impl Report for ChainDiscoveryReport {
    fn render(&self) {
        println!();
        for result in &self.results {
            match &result.error {
                Some(error) => println!(
                    "  {} {}  {}",
                    out::fail_mark(),
                    result.wallet_id,
                    out::hint(error)
                ),
                None => println!(
                    "  {} {}  {}",
                    out::ok_mark(),
                    result.wallet_id,
                    out::count(result.addresses.len(), "address")
                ),
            }
        }
    }
}

/// The addresses discovery found used, as it recorded them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryReport {
    wallet: String,
    chain: Chain,
    address_count: usize,
    addresses: Vec<String>,
    #[serde(skip)]
    name: String,
}

impl Report for DiscoveryReport {
    fn render(&self) {
        println!();
        out::wide_field(9, "wallet", &self.name.bold().to_string());
        out::wide_field(9, "addresses", &self.address_count.to_string());
        for address in &self.addresses {
            println!("  {}", out::hint(address));
        }
    }
}

fn discover(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = wallet.chain_id;
    let service = super::chain::service_for_chain(
        ctx,
        chain,
        &[
            super::chain::EndpointCapability::Balance,
            super::chain::EndpointCapability::History,
        ],
    )?;

    let addresses = ctx
        .rt
        .block_on(service.discover_utxo_addresses(wallet.id.clone(), chain))?;
    out.report(&DiscoveryReport {
        wallet: wallet.id,
        chain,
        address_count: addresses.len(),
        addresses,
        name: wallet.name,
    })
}

/// Where a wallet's keypool stands, and the address its reserved receive
/// index was handed out as.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PoolReport {
    wallet: String,
    chain: Chain,
    next_external_index: i64,
    next_change_index: i64,
    reserved_receive_index: Option<i64>,
    reserved_receive_address: Option<String>,
    reserved_receive_path: Option<String>,
    #[serde(skip)]
    name: String,
}

impl Report for PoolReport {
    fn render(&self) {
        println!();
        out::field("wallet", &self.name.bold().to_string());
        out::field("chain", &out::chain(self.chain).to_string());
        out::field("receive", &self.next_external_index.to_string());
        out::field("change", &self.next_change_index.to_string());
        out::field(
            "reserved",
            &self
                .reserved_receive_index
                .map_or_else(|| "none".into(), |index| index.to_string()),
        );
        if let Some(address) = &self.reserved_receive_address {
            out::field("address", address);
        }
        if let Some(path) = &self.reserved_receive_path {
            out::field("path", path);
        }
    }
}

fn show(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    // The diagnostics screen's row: the indices, and the address the reserved
    // receive index was handed out as, read from what was recorded then.
    let row = ctx
        .rt
        .block_on(ctx.service()?.keypool_diagnostics(wallet.chain_id))?
        .into_iter()
        .find(|row| row.wallet_id == wallet.id)
        .ok_or_else(|| CliError::rejected("no keypool for this wallet"))?;
    let reserved = row.reserved_receive;
    out.report(&PoolReport {
        wallet: wallet.id,
        chain: wallet.chain_id,
        next_external_index: row.keypool.next_external_index,
        next_change_index: row.keypool.next_change_index,
        reserved_receive_index: row.keypool.reserved_receive_index,
        reserved_receive_address: reserved.as_ref().map(|r| r.address.clone()),
        reserved_receive_path: reserved.and_then(|r| r.derivation_path),
        name: wallet.name,
    })
}

/// An index reserved, and the receive address it stands for.
#[derive(Serialize)]
struct ReservedReport {
    wallet: String,
    kind: &'static str,
    index: i64,
    address: Option<String>,
}

impl Report for ReservedReport {
    fn render(&self) {
        println!(
            "  {} reserved {} index {}",
            out::ok_mark(),
            self.kind,
            self.index.to_string().bold(),
        );
        if let Some(address) = &self.address {
            println!("     {}", out::info(address));
        }
    }
}

fn next(ctx: &Ctx, out: Out, args: WalletArg, change: bool) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let service = ctx.service()?;
    let index = ctx.rt.block_on(async {
        if change {
            service
                .reserve_change_index(wallet.id.clone(), wallet.chain_id)
                .await
        } else {
            service
                .reserve_receive_index(wallet.id.clone(), wallet.chain_id, 0)
                .await
        }
    })?;
    let address = if change {
        None
    } else {
        let chain = wallet.chain_id.mainnet_counterpart();
        ctx.rt
            .block_on(service.receive_address(wallet.id.clone(), chain, false))?
    };
    out.report(&ReservedReport {
        wallet: wallet.id,
        kind: if change { "change" } else { "receive" },
        index,
        address,
    })
}

//! Reading stored wallets: the list, one wallet, what its page offers, its
//! receive address and its coins.

use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::{AddressBranch, WalletActions, WalletCoins, WalletDerivedState};

use super::WalletView;
use crate::ctx::{Ctx, WalletArg};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Serialize)]
struct ListReport {
    wallets: Vec<WalletView>,
}

impl Report for ListReport {
    fn render(&self) {
        println!();
        if self.wallets.is_empty() {
            println!("  {}", out::hint("no wallets yet"));
            println!(
                "  {} {}",
                out::hint("add one with"),
                out::info("spectra wallet new --chain Bitcoin")
            );
            return;
        }
        for wallet in &self.wallets {
            println!(
                "  {}  {}  {}{}",
                out::wallet_dot(wallet.chain, wallet.is_watch_only),
                wallet.name.bold(),
                out::chain(wallet.chain).bold(),
                if wallet.is_watch_only {
                    out::hint(" watch").to_string()
                } else {
                    String::new()
                },
            );
            println!("     {}", out::info(&wallet.address));
        }
        println!();
        println!("  {}", out::count(self.wallets.len(), "wallet"));
    }
}

pub fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let wallets = ctx.state()?.wallets.iter().map(WalletView::from).collect();
    out.report(&ListReport { wallets })
}

#[derive(Serialize)]
struct ShowReport {
    wallet: WalletView,
}

impl Report for ShowReport {
    fn render(&self) {
        println!();
        self.wallet.render();
        out::field("id", &out::hint(&self.wallet.id).to_string());
    }
}

pub fn show(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    out.report(&ShowReport {
        wallet: WalletView::from(&wallet),
    })
}

/// Core's action descriptor for a wallet: what its page offers.
#[derive(Serialize)]
struct ActionsReport {
    #[serde(skip)]
    name: String,
    actions: WalletActions,
}

impl Report for ActionsReport {
    fn render(&self) {
        let actions = &self.actions;
        println!();
        println!(
            "  {}  {}",
            self.name.bold(),
            out::chain(actions.chain).bold()
        );
        for offer in &actions.actions {
            println!(
                "  {}  {}  {}",
                out::hint(&format!("{:<9}", out::serde_name(&offer.section))),
                format!("{:<13}", out::serde_name(&offer.action)).bold(),
                offer.note
            );
        }
        for limit in &actions.summary.limits {
            out::wide_field(9, "limit", crate::cmd::limit_text(limit));
        }
    }
}

pub fn actions(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let actions = ctx.rt.block_on(ctx.service()?.wallet_actions(wallet.id))?;
    out.report(&ActionsReport {
        name: wallet.name,
        actions,
    })
}

#[derive(Serialize)]
struct ReceiveReport {
    address: String,
    chain: Chain,
    symbol: &'static str,
}

impl Report for ReceiveReport {
    fn render(&self) {
        println!();
        println!("  {}", self.address.bold());
        println!();
        out::field("chain", &out::chain(self.chain).to_string());
        out::field("symbol", self.symbol);
    }
}

/// Ask core for the receive address on the wallet's network. Core reserves
/// UTXO indices and registers derived addresses as owned; repeated calls
/// reuse the reserved index.
pub fn receive(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = wallet.chain_id;
    let address = ctx
        .rt
        .block_on(ctx.service()?.receive_address(wallet.id, chain, true))?
        .ok_or_else(|| {
            CliError::failure(format!(
                "no receive address for {:?} on {}",
                wallet.name,
                chain.chain_display_name()
            ))
        })?;
    out.report(&ReceiveReport {
        address,
        chain,
        symbol: chain.coin_symbol(),
    })
}

#[derive(Serialize)]
struct CoinsReport {
    coins: WalletCoins,
}

impl Report for CoinsReport {
    fn render(&self) {
        let coins = &self.coins;
        println!();
        for address in &coins.addresses {
            let place = match (address.branch, address.index) {
                (Some(AddressBranch::Receive), Some(i)) => format!("receive {i}"),
                (Some(AddressBranch::Change), Some(i)) => format!("change {i}"),
                _ => "seen".to_string(),
            };
            println!(
                "  {}  {}  {} {}{}",
                out::hint(&format!("{place:<11}")),
                out::info(&address.address),
                address.balance,
                coins.symbol,
                if address.used { "" } else { "  (unused)" }
            );
        }
        if let Some(next) = &coins.next_receive_address {
            out::field("next", &out::info(next).to_string());
        }
        println!();
        for output in &coins.outputs {
            println!(
                "  {}:{}  {} {}  {} conf{}",
                output.txid,
                output.vout,
                output.amount,
                coins.symbol,
                output.confirmations,
                if output.spendable { "" } else { "  maturing" }
            );
        }
    }
}

pub fn coins(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let coins = ctx.rt.block_on(ctx.service()?.wallet_coins(wallet.id))?;
    out.report(&CoinsReport { coins })
}

/// What core derives from every wallet: which holdings the portfolio counts,
/// and which wallets can send and receive.
#[derive(Serialize)]
struct DerivedReport {
    #[serde(flatten)]
    derived: WalletDerivedState,
}

impl Report for DerivedReport {
    fn render(&self) {
        let derived = &self.derived;
        println!();
        for holding in &derived.portfolio {
            println!(
                "  {}  {} {}",
                out::tint("●", holding.chain_id),
                holding.amount.bold(),
                holding.symbol
            );
        }
        println!();
        out::wide_field(
            9,
            "counted",
            &out::count(derived.included_portfolio_holdings.len(), "holding"),
        );
        out::wide_field(
            9,
            "can send",
            &out::count(derived.send_enabled_wallet_ids.len(), "wallet"),
        );
        out::wide_field(
            9,
            "receives",
            &out::count(derived.receive_enabled_wallet_ids.len(), "wallet"),
        );
    }
}

pub fn derived(ctx: &Ctx, out: Out) -> CliResult<()> {
    let derived = ctx.rt.block_on(ctx.service()?.wallet_derived_state())?;
    out.report(&DerivedReport { derived })
}

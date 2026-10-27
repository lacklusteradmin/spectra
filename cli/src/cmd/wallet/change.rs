//! Changing a stored wallet, always through core's reducer: core decides
//! whether a wallet may change and persists the result itself.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::store::state::StateCommand;

use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Done, Out, Report};

#[derive(Args)]
pub struct RenameArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// New name.
    name: String,
}

#[derive(Args)]
pub struct InclusionArgs {
    /// Wallet id, name or address.
    wallet: String,
    #[arg(action = clap::ArgAction::Set)]
    included: bool,
}

#[derive(Args)]
pub struct HoldingArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The holding: its deployment id (`ethereum:erc-20:0x…`) or a symbol
    /// only one of the wallet's holdings has.
    asset: String,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Delete without asking for confirmation.
    #[arg(long)]
    yes: bool,
}

#[derive(Serialize)]
struct RenameReport {
    from: String,
    to: String,
}

impl Report for RenameReport {
    fn render(&self) {
        println!(
            "  {} {} {} {}",
            out::ok_mark(),
            out::hint(&self.from),
            out::hint("→"),
            self.to.bold()
        );
    }
}

pub fn rename(ctx: &Ctx, out: Out, args: RenameArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let name = args.name.trim().to_string();
    if name.is_empty() {
        return Err(CliError::rejected("a wallet name cannot be empty"));
    }
    ctx.apply(StateCommand::RenameWallet {
        wallet_id: wallet.id,
        name: name.clone(),
    })?;
    out.report(&RenameReport {
        from: wallet.name,
        to: name,
    })
}

pub fn inclusion(ctx: &Ctx, out: Out, args: InclusionArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    ctx.apply(StateCommand::SetWalletPortfolioInclusion {
        wallet_id: wallet.id,
        included: args.included,
    })?;
    out.report(&Done::new(format!(
        "{} {} portfolio totals",
        wallet.name,
        if args.included {
            "counts in"
        } else {
            "is left out of"
        }
    )))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HiddenReport {
    deployment_id: String,
    hidden: bool,
}

impl Report for HiddenReport {
    fn render(&self) {
        println!(
            "  {} {} {}",
            out::ok_mark(),
            if self.hidden { "hid" } else { "showed" },
            self.deployment_id
        );
    }
}

pub fn set_hidden(ctx: &Ctx, out: Out, args: HoldingArgs, hidden: bool) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let asset = args.asset.trim();
    let mut matches: Vec<String> = wallet
        .holdings
        .iter()
        .map(|holding| holding.deployment_id())
        .filter(|id| id == asset)
        .collect();
    if matches.is_empty() {
        matches = wallet
            .holdings
            .iter()
            .filter(|holding| holding.symbol.eq_ignore_ascii_case(asset))
            .map(|holding| holding.deployment_id())
            .collect();
    }
    // Showing a holding again names it by the id it was hidden under.
    if matches.is_empty() && !hidden && wallet.hidden_holdings.iter().any(|id| id == asset) {
        matches.push(asset.to_string());
    }
    let deployment_id = match matches.as_slice() {
        [one] => one.clone(),
        [] => {
            return Err(CliError::rejected(format!(
                "\"{}\" holds no {asset}",
                wallet.name
            )));
        }
        _ => {
            return Err(CliError::usage(format!(
                "more than one holding is {asset}; name it by its deployment id"
            )));
        }
    };
    ctx.apply(StateCommand::SetHoldingHidden {
        wallet_id: wallet.id,
        deployment_id: deployment_id.clone(),
        hidden,
    })?;
    out.report(&HiddenReport {
        deployment_id,
        hidden,
    })
}

#[derive(Serialize)]
struct DeletedReport {
    deleted: String,
    #[serde(skip)]
    name: String,
}

impl Report for DeletedReport {
    fn render(&self) {
        println!("  {} deleted \"{}\"", out::ok_mark(), self.name);
    }
}

pub fn delete(ctx: &Ctx, out: Out, args: DeleteArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    if !args.yes {
        let secrets = if wallet.is_watch_only() {
            ""
        } else {
            " and its stored signing material"
        };
        return Err(CliError::usage(format!(
            "this deletes \"{}\" ({}), its history{secrets} — re-run with --yes",
            wallet.name,
            wallet.chain_id.chain_display_name()
        )));
    }
    ctx.apply(StateCommand::RemoveWallet {
        wallet_id: wallet.id.clone(),
    })?;
    out.report(&DeletedReport {
        deleted: wallet.id,
        name: wallet.name,
    })
}

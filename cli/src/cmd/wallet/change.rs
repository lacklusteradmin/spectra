//! Changing a stored wallet, always through core's reducer: core decides
//! whether a wallet may change and persists the result itself.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::store::state::StateCommand;

use crate::ctx::{Ctx, WalletArg};
use crate::error::{CliError, CliResult};
use crate::out::{self, Done, Out, Report};

#[derive(Args)]
pub struct RenameArgs {
    #[command(flatten)]
    wallet: WalletArg,
    /// New name.
    name: String,
}

#[derive(Args)]
pub struct InclusionArgs {
    #[command(flatten)]
    wallet: WalletArg,
    #[arg(action = clap::ArgAction::Set)]
    included: bool,
}

#[derive(Args)]
pub struct HoldingArgs {
    #[command(flatten)]
    wallet: WalletArg,
    /// The holding's deployment id (`ethereum:erc-20:0x…`), as `wallet show`
    /// lists it.
    asset: String,
}

#[derive(Args)]
pub struct DeleteArgs {
    #[command(flatten)]
    wallet: WalletArg,
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
    let wallet = args.wallet.find(ctx)?;
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
    let wallet = args.wallet.find(ctx)?;
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
    let wallet = args.wallet.find(ctx)?;
    let asset = args.asset.trim();
    // Showing a holding again names it by the id it was hidden under.
    let known = wallet
        .holdings
        .iter()
        .any(|holding| holding.deployment_id() == asset)
        || (!hidden && wallet.hidden_holdings.iter().any(|id| id == asset));
    if !known {
        return Err(CliError::rejected(format!(
            "\"{}\" holds no {asset}",
            wallet.name
        )));
    }
    let deployment_id = asset.to_string();
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
    let wallet = args.wallet.find(ctx)?;
    if !args.yes {
        let secrets = if wallet.is_watch_only() {
            ""
        } else {
            " and its stored signing material"
        };
        return Err(CliError::unconfirmed(format!(
            "deletes \"{}\" ({}), its history{secrets}",
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

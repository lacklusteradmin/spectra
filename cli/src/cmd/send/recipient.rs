//! Where a send would go: the address typed, what it resolves to, and what
//! the network says about it.

use clap::Args;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::{SendDestinationActivity, WalletService};

use crate::cmd::chain::service_for_chain;
use crate::cmd::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Args)]
pub struct SelfCheckArgs {
    /// Wallet id, name or address.
    wallet: String,
    #[arg(long)]
    holding: String,
    #[arg(long)]
    destination: String,
}

#[derive(Args)]
pub struct ProbeArgs {
    /// The wallet the send would come from: id, name or address.
    wallet: String,
    /// Asset symbol being sent. Defaults to the wallet chain's own asset.
    #[arg(long)]
    asset: Option<String>,
    /// Narrows the asset to one chain, for a symbol the wallet holds on several.
    #[arg(long)]
    chain: Option<String>,
    /// Recipient, as the user would type it.
    #[arg(long)]
    to: String,
}

/// What the composer does with the destination field, on the command line.
///
/// Whether a `.eth` name is looked up is the chain's, not the caller's, so
/// this needs no flag to say "try ENS": ask any other chain and the name is
/// refused without a request leaving the machine.
#[derive(Args)]
pub struct DestinationArgs {
    /// Chain the send is on.
    #[arg(long)]
    chain: String,
    /// What the user typed: an address, or a name on a chain that resolves one.
    #[arg(long)]
    to: String,
    /// Address shown in a previous review; refuse if the destination changed.
    #[arg(long)]
    expected: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfCheckReport {
    own_address: bool,
}

impl Report for SelfCheckReport {
    fn render(&self) {
        println!(
            "  {}",
            if self.own_address {
                "one of your addresses"
            } else {
                "not one of your addresses"
            }
        );
    }
}

pub fn self_check(ctx: &Ctx, out: Out, args: SelfCheckArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let own_address = ctx.rt.block_on(ctx.service()?.is_own_send_destination(
        wallet.id,
        args.holding,
        args.destination,
    ))?;
    out.report(&SelfCheckReport { own_address })
}

/// The composer's recipient check: whether the address has been used and
/// holds anything, on the network of the asset being sent.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeReport {
    wallet: String,
    chain: Chain,
    destination: String,
    asset: String,
    activity: SendDestinationActivity,
    balance_is_zero: bool,
    has_history: bool,
}

impl Report for ProbeReport {
    fn render(&self) {
        println!();
        out::field("to", &self.destination);
        out::field("asset", &self.asset);
        out::field("network", &out::chain(self.chain).to_string());
        out::field(
            "balance",
            if self.balance_is_zero {
                "zero"
            } else {
                "non-zero"
            },
        );
        out::field("history", if self.has_history { "yes" } else { "none" });
    }
}

/// Core resolves the holding's contract and returns typed verdict flags;
/// the CLI supplies the wording.
pub fn probe(ctx: &Ctx, out: Out, args: ProbeArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let symbol = args.asset.clone().unwrap_or_else(|| {
        wallet
            .chain_id
            .mainnet_counterpart()
            .coin_symbol()
            .to_string()
    });
    let on_chain = args.chain.as_deref().map(resolve_chain).transpose()?;

    let candidates: Vec<&spectra_core::store::wallet_domain::AssetHolding> = wallet
        .holdings
        .iter()
        .filter(|h| h.symbol.eq_ignore_ascii_case(&symbol))
        .filter(|h| on_chain.is_none_or(|c| c == h.chain_id))
        .collect();
    let holding = match candidates.as_slice() {
        [] => {
            return Err(CliError::rejected(format!(
                "wallet {} holds no {symbol}",
                wallet.id
            )));
        }
        [one] => *one,
        many => {
            let chains: Vec<&str> = many
                .iter()
                .map(|h| h.chain_id.chain_display_name())
                .collect();
            return Err(CliError::usage(format!(
                "{symbol} is held on {} — narrow it with --chain",
                chains.join(", ")
            )));
        }
    };

    let risk = ctx.rt.block_on(ctx.service()?.send_destination_risk(
        wallet.id.clone(),
        holding.deployment_id(),
        args.to.clone(),
    ))?;
    out.report(&ProbeReport {
        wallet: wallet.id,
        chain: holding.chain_id,
        destination: args.to,
        asset: holding.symbol.clone(),
        activity: risk.activity,
        balance_is_zero: risk.balance_is_zero,
        has_history: risk.has_history,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DestinationReport {
    chain: &'static str,
    typed: String,
    address: String,
    used_ens: bool,
}

impl Report for DestinationReport {
    fn render(&self) {
        println!();
        out::field("typed", &self.typed);
        out::field("address", &self.address);
        out::field("via", if self.used_ens { "ENS" } else { "typed" });
    }
}

pub fn destination(ctx: &Ctx, out: Out, args: DestinationArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    // ENS needs the registry-selected EVM API; other chains validate locally.
    let service = if chain.resolves_ens_names() {
        service_for_chain(ctx, chain, &[])?
    } else {
        WalletService::new(Vec::new())?
    };
    let resolved = ctx.rt.block_on(async {
        match args.expected {
            Some(expected) => {
                service
                    .verify_send_destination(chain, args.to.clone(), expected)
                    .await
            }
            None => {
                service
                    .resolve_send_destination(chain, args.to.clone())
                    .await
            }
        }
    })?;
    out.report(&DestinationReport {
        chain: chain.str_id(),
        typed: args.to,
        address: resolved.address,
        used_ens: resolved.used_ens,
    })
}

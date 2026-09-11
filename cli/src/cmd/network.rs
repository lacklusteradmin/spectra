//! Which network of a family a chain is on.
//!
//! A "network mode" is not a separate concept: it is which `Chain` of a family
//! the user selected, stored as `network_chain_by_family` and read back through
//! `AppSettings::network_chain`. Absent means mainnet.
//!
//! This had no command until now, and that is how a reset that put three
//! families back to mainnet where twenty-nine have a choice went unnoticed —
//! the axis was reachable only from the iOS picker.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::registry::Chain;
use spectra_core::store::state::StateCommand;

use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum NetworkCommand {
    /// Every family that has a choice, and which network it is on.
    List,
    /// Put a family on one of its networks.
    Set(SetArgs),
}

#[derive(Args)]
pub struct SetArgs {
    /// Registry id of the network to select, as `network list` prints it —
    /// a family's own id selects its mainnet.
    chain_id: String,
}

pub fn run(ctx: &Ctx, out: Out, command: NetworkCommand) -> CliResult<()> {
    match command {
        NetworkCommand::List => list(ctx, out),
        NetworkCommand::Set(args) => set(ctx, out, args),
    }
}

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let settings = ctx.state()?.settings;
    let families: Vec<(Chain, Chain)> = Chain::mainnets()
        .filter(|chain| chain.has_network_choice())
        .map(|family| (family, settings.network_chain(family)))
        .collect();

    out.text(|| {
        println!();
        for (family, selected) in &families {
            let mark = if selected == family {
                out::hint("mainnet")
            } else {
                out::accent(selected.chain_display_name())
            };
            println!(
                "  {:<20} {:<24} {mark}",
                family.str_id().bold(),
                selected.str_id()
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "families": families
            .iter()
            .map(|(family, selected)| {
                serde_json::json!({
                    "family": family.str_id(),
                    "selected": selected.str_id(),
                    "isTestnet": selected.is_testnet(),
                    "choices": family
                        .network_choices()
                        .iter()
                        .map(|c| c.str_id())
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

fn set(ctx: &Ctx, out: Out, args: SetArgs) -> CliResult<()> {
    let chain = Chain::from_str_id(&args.chain_id)
        .ok_or_else(|| CliError::rejected(format!("no chain with id {}", args.chain_id)))?;
    let family = chain.mainnet_counterpart();
    let transition = ctx.apply(StateCommand::SelectNetworkChain {
        chain_id: chain.str_id().to_string(),
    })?;
    let selected = transition.state.settings.network_chain(family);

    // A switch invalidates what the family derived on the network it left:
    // reserved keypool indices belong to that network, and so do the addresses
    // discovered under them. iOS did this and the CLI did not, which is the
    // divergence this command exists to close — the reset is part of the
    // switch, not something a front end remembers to do afterwards.
    let cleared = reset_derivation_state(ctx, family, selected)?;

    out.text(|| {
        println!(
            "  {} {} is on {}",
            out::ok_mark(),
            family.str_id(),
            selected.chain_display_name().bold()
        );
        for name in &cleared {
            println!("  {} cleared derivation state for {name}", out::ok_mark());
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "family": family.str_id(),
        "selected": selected.str_id(),
        "isTestnet": selected.is_testnet(),
        "clearedDerivationState": cleared,
    }));
    Ok(())
}

/// Drop the derivation state of every chain in the family whose network the
/// switch just changed, and report which ones were cleared.
///
/// Both sides of the switch are cleared: the network being left holds indices
/// that no longer describe anything, and the one being entered may hold stale
/// rows from the last time it was selected.
fn reset_derivation_state(ctx: &Ctx, family: Chain, selected: Chain) -> CliResult<Vec<String>> {
    let service = ctx.service()?;
    let mut cleared = Vec::new();
    let mut names: Vec<String> = family
        .network_choices()
        .iter()
        .map(|chain| chain.chain_display_name().to_string())
        .collect();
    if !names.iter().any(|n| n == selected.chain_display_name()) {
        names.push(selected.chain_display_name().to_string());
    }
    names.sort();
    names.dedup();
    for name in names {
        ctx.rt
            .block_on(service.reset_chain_derivation_state(name.clone()))
            .map_err(CliError::from)?;
        cleared.push(name);
    }
    Ok(cleared)
}

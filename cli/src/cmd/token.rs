//! Tokens: the built-in catalog, and which ones this wallet tracks.
//!
//! Tracked tokens live in `CoreAppState.token_preferences` and go in through
//! `SetTokenPreferences`, which clamps the display decimals — a token cannot
//! show more places than it has. That rule was the reason the list moved into
//! core; until now only Swift could exercise it.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::StateCommand;
use spectra_core::store::wallet_domain::{
    CoreTokenPreferenceCategory, CoreTokenPreferenceEntry, CoreTokenTrackingChain,
};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Tokens the build knows about for a chain.
    Catalog(CatalogArgs),
    /// Tokens this wallet tracks.
    List,
    /// Track a token from the catalog, by symbol.
    Track(TrackArgs),
    /// Stop tracking a token.
    Untrack(UntrackArgs),
}

#[derive(Args)]
pub struct CatalogArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
}

#[derive(Args)]
pub struct TrackArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
    /// Token symbol as the catalog spells it.
    symbol: String,
    /// Decimal places to display. Core clamps this to what the token has.
    #[arg(long)]
    display_decimals: Option<i32>,
}

#[derive(Args)]
pub struct UntrackArgs {
    /// Token symbol.
    symbol: String,
}

pub fn run(ctx: &Ctx, out: Out, command: TokenCommand) -> CliResult<()> {
    match command {
        TokenCommand::Catalog(args) => catalog(out, args),
        TokenCommand::List => list(ctx, out),
        TokenCommand::Track(args) => track(ctx, out, args),
        TokenCommand::Untrack(args) => untrack(ctx, out, args),
    }
}

fn catalog(out: Out, args: CatalogArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let tokens = spectra_core::tokens::list_tokens(chain.str_id().to_string());

    out.text(|| {
        println!();
        if tokens.is_empty() {
            println!("  {}", out::hint("no tokens in the catalog for this chain"));
            return;
        }
        for token in &tokens {
            println!(
                "  {}  {:<8} {:<24} {}",
                out::tint("●", chain.chain_display_name()).bold(),
                token.symbol.bold(),
                token.name,
                out::hint(&format!("{} decimals", token.decimals)),
            );
            if !token.contract.is_empty() {
                println!("     {}", out::hint(&token.contract));
            }
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.chain_display_name(),
        "tokens": tokens
            .iter()
            .map(|token| serde_json::json!({
                "symbol": token.symbol,
                "name": token.name,
                "contract": token.contract,
                "decimals": token.decimals,
                "standard": token.token_standard,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let tracked = ctx.state()?.token_preferences;
    out.text(|| {
        println!();
        if tracked.is_empty() {
            println!("  {}", out::hint("no tracked tokens"));
            return;
        }
        for entry in &tracked {
            println!(
                "  {}  {:<8} {:<22} {}",
                out::accent("●").bold(),
                entry.symbol.bold(),
                entry.name,
                out::hint(&format!(
                    "{} of {} decimals shown",
                    entry.display_decimals.unwrap_or(entry.decimals),
                    entry.decimals
                )),
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "tokens": tracked
            .iter()
            .map(|entry| serde_json::json!({
                "id": entry.id,
                "symbol": entry.symbol,
                "name": entry.name,
                "contract": entry.contract_address,
                "decimals": entry.decimals,
                "displayDecimals": entry.display_decimals,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

fn track(ctx: &Ctx, out: Out, args: TrackArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let tokens = spectra_core::tokens::list_tokens(chain.str_id().to_string());
    let token = tokens
        .iter()
        .find(|token| token.symbol.eq_ignore_ascii_case(&args.symbol))
        .ok_or_else(|| {
            CliError::rejected(format!(
                "{} has no token {:?} in the catalog",
                chain.chain_display_name(),
                args.symbol
            ))
        })?;

    let tracking_chain = CoreTokenTrackingChain::from_chain_name(chain.chain_display_name())
        .ok_or_else(|| {
            CliError::rejected(format!(
                "{} does not support tracked tokens",
                chain.chain_display_name()
            ))
        })?;

    let mut entries = ctx.state()?.token_preferences;
    if entries
        .iter()
        .any(|entry| entry.symbol.eq_ignore_ascii_case(&token.symbol))
    {
        return Err(CliError::rejected(format!(
            "{} is already tracked",
            token.symbol
        )));
    }
    entries.push(CoreTokenPreferenceEntry {
        id: uuid::Uuid::new_v4().to_string().to_uppercase(),
        chain: tracking_chain,
        name: token.name.clone(),
        symbol: token.symbol.clone(),
        token_standard: token.token_standard.clone(),
        contract_address: token.contract.clone(),
        coin_gecko_id: token.coingecko_id.clone(),
        decimals: token.decimals as i32,
        // Deliberately unclamped: core clamps it, and the point of asking here
        // is to see it do so.
        display_decimals: args.display_decimals,
        category: CoreTokenPreferenceCategory::Custom,
        is_built_in: true,
        is_enabled: true,
    });

    let transition = ctx.apply(StateCommand::SetTokenPreferences { entries })?;
    let stored = transition
        .state
        .token_preferences
        .iter()
        .find(|entry| entry.symbol.eq_ignore_ascii_case(&token.symbol))
        .cloned()
        .ok_or_else(|| CliError::failure("core accepted the token but did not store it"))?;

    out.text(|| {
        println!("  {} tracking {}", out::ok_mark(), stored.symbol.bold());
        out::field(
            "decimals",
            &format!(
                "{} of {}",
                stored.display_decimals.unwrap_or(stored.decimals),
                stored.decimals
            ),
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "symbol": stored.symbol,
        "decimals": stored.decimals,
        "displayDecimals": stored.display_decimals,
    }));
    Ok(())
}

fn untrack(ctx: &Ctx, out: Out, args: UntrackArgs) -> CliResult<()> {
    let entries = ctx.state()?.token_preferences;
    let remaining: Vec<_> = entries
        .iter()
        .filter(|entry| !entry.symbol.eq_ignore_ascii_case(&args.symbol))
        .cloned()
        .collect();
    if remaining.len() == entries.len() {
        return Err(CliError::rejected(format!(
            "{} is not tracked",
            args.symbol
        )));
    }

    ctx.apply(StateCommand::SetTokenPreferences { entries: remaining })?;
    out.text(|| println!("  {} untracked {}", out::ok_mark(), args.symbol.bold()));
    out.emit(serde_json::json!({ "ok": true, "untracked": args.symbol }));
    Ok(())
}

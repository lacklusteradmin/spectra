//! `SetTokenPreferences` clamps the display decimals — a token cannot show
//! more places than it has — which is the rule the list moved into core for.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::StateCommand;
use spectra_core::store::wallet_domain::{
    CoreTokenPreferenceEntry, CoreTokenHostingChain,
};

use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx};
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
    /// Ask the chain what a wallet actually holds.
    Discover(DiscoverArgs),
    /// How an amount renders, and why that many places.
    Format(FormatArgs),
}

#[derive(Args)]
pub struct FormatArgs {
    /// Amount in the asset's own units, as a person would type it.
    amount: f64,
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
    /// Token symbol. Omit for the chain's native asset.
    #[arg(long)]
    symbol: Option<String>,
}

#[derive(Args)]
pub struct DiscoverArgs {
    /// Wallet to look at (id, name or address).
    #[arg(long)]
    wallet: String,
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
        TokenCommand::Discover(args) => discover(ctx, out, args),
        TokenCommand::Format(args) => format_amount(ctx, out, args),
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
            println!("  {}", out::hint("no known tokens"));
            return;
        }
        for entry in &tracked {
            println!(
                "  {}  {:<8} {:<22} {}",
                out::accent("●").bold(),
                entry.token.symbol.bold(),
                entry.token.name,
                out::hint(&format!("{} decimals", entry.token.decimals)),
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "tokens": tracked
            .iter()
            .map(|entry| serde_json::json!({
                "id": entry.id(),
                "symbol": entry.token.symbol,
                "name": entry.token.name,
                "contract": entry.token.contract,
                "decimals": entry.token.decimals,
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

    let _ = CoreTokenHostingChain::from_chain_name(chain.chain_display_name())
        .ok_or_else(|| {
            CliError::rejected(format!(
                "{} does not support known tokens",
                chain.chain_display_name()
            ))
        })?;

    let mut entries = ctx.state()?.token_preferences;
    if entries
        .iter()
        .any(|entry| entry.token.symbol.eq_ignore_ascii_case(&token.symbol))
    {
        return Err(CliError::rejected(format!(
            "{} is already tracked",
            token.symbol
        )));
    }
    entries.push(CoreTokenPreferenceEntry {
        category: CoreTokenPreferenceEntry::category_from_tags(&token.tags),
        is_built_in: true,
        is_enabled: true,
        token: token.clone(),
    });

    let transition = ctx.apply(StateCommand::SetTokenPreferences { entries })?;
    let stored = transition
        .state
        .token_preferences
        .iter()
        .find(|entry| entry.token.symbol.eq_ignore_ascii_case(&token.symbol))
        .cloned()
        .ok_or_else(|| CliError::failure("core accepted the token but did not store it"))?;

    out.text(|| {
        println!("  {} tracking {}", out::ok_mark(), stored.token.symbol.bold());
        out::field("decimals", &stored.token.decimals.to_string());
    });
    out.emit(serde_json::json!({
        "ok": true,
        "symbol": stored.token.symbol,
        "decimals": stored.token.decimals,
    }));
    Ok(())
}

fn untrack(ctx: &Ctx, out: Out, args: UntrackArgs) -> CliResult<()> {
    let entries = ctx.state()?.token_preferences;
    let remaining: Vec<_> = entries
        .iter()
        .filter(|entry| !entry.token.symbol.eq_ignore_ascii_case(&args.symbol))
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

/// What the chain says this wallet holds, rather than what the catalog says it
/// might.
///
/// A token the catalog does not vouch for prints its **contract address** and
/// no name. That is deliberate: a discovered token's on-chain symbol is chosen
/// by whoever deployed it, so an airdrop can call itself USDC, and the address
/// is the one string it cannot forge.
fn discover(ctx: &Ctx, out: Out, args: DiscoverArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = resolve_chain(&wallet.chain_name)?;
    let address = wallet_address(&wallet).to_string();
    if address.is_empty() {
        return Err(CliError::rejected(format!(
            "{} has no address on {}",
            wallet.name,
            chain.chain_display_name()
        )));
    }
    let service = ctx.service()?;
    let held = ctx
        .rt
        .block_on(service.discover_token_balances(chain.str_id().to_string(), address))
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        if held.is_empty() {
            println!("  {}", out::hint("this address holds no tokens"));
            return;
        }
        for token in &held {
            let name = if token.is_known {
                token.symbol.clone().bold().to_string()
            } else {
                out::hint("unrecognised").to_string()
            };
            println!("  {:<24} {name}", token.balance_display.bold());
            println!("     {}", out::hint(&token.contract_address));
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "holdings": held
            .iter()
            .map(|t| serde_json::json!({
                "contract": t.contract_address,
                "symbol": t.symbol,
                "isKnown": t.is_known,
                "decimals": t.decimals,
                "balance": t.balance_display,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

/// The display rule, from outside core.
///
/// Places follow the amount, not a per-chain setting: a small balance keeps its
/// significant digits instead of rounding to nothing, and a large one does not
/// print six zeros it does not have.
fn format_amount(ctx: &Ctx, out: Out, args: FormatArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let asset_decimals = match &args.symbol {
        Some(symbol) => {
            let symbol_upper = symbol.to_uppercase();
            let entry = spectra_core::tokens::list_tokens(chain.str_id().to_string())
                .into_iter()
                .find(|t| t.symbol.eq_ignore_ascii_case(&symbol_upper))
                .ok_or_else(|| {
                    CliError::rejected(format!(
                        "{} has no token {symbol_upper} in the catalog",
                        chain.chain_display_name()
                    ))
                })?;
            entry.decimals
        }
        None => spectra_core::formatting::supported_decimal_places(
            chain.chain_display_name(),
            None,
        ),
    };
    let display = spectra_core::formatting::asset_amount_display(args.amount, asset_decimals);
    let rendered = if display.below_threshold {
        format!("<{:.*}", display.places as usize, display.threshold)
    } else {
        let full = format!("{:.*}", display.places as usize, args.amount);
        let trimmed = if full.contains('.') {
            full.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            full
        };
        trimmed
    };
    let _ = ctx;

    out.text(|| {
        out::field("shows", &rendered);
        out::field("places", &display.places.to_string());
        out::field("asset decimals", &asset_decimals.to_string());
    });
    out.emit(serde_json::json!({
        "ok": true,
        "shows": rendered,
        "places": display.places,
        "assetDecimals": asset_decimals,
        "belowThreshold": display.below_threshold,
    }));
    Ok(())
}

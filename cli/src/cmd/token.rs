//! Token commands update core-owned preference rows. Core validates custom
//! contracts and duplicates; tracking toggles `is_enabled`.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::{
    CoreTokenPreferenceKey, StateCommand, StateEvent, StateTransition, TokenPreferenceRejection,
};
use spectra_core::store::wallet_domain::CoreTokenHostingChain;

use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Tokens the build knows about for a chain.
    Catalog(CatalogArgs),
    /// Resolve bundled artwork by catalog identity; unknown identities have no mark.
    Artwork(ArtworkArgs),
    /// Tokens this wallet tracks.
    List,
    /// Track a token: turn on the row core holds for it.
    Track(TrackArgs),
    /// Stop tracking a token.
    Untrack(TrackArgs),
    /// Teach the wallet a token the catalog does not ship.
    Add(AddArgs),
    /// Edit a custom token without changing its network or identifier.
    Edit(AddArgs),
    /// Forget a custom token.
    Remove(RemoveArgs),
    /// Change a custom token's display precision.
    Decimals(DecimalsArgs),
    /// Back to the catalog's own list, dropping every custom token.
    Reset(ResetArgs),
    /// Ask the chain what a wallet actually holds.
    Discover(DiscoverArgs),
    /// How an amount renders, and why that many places.
    Format(FormatArgs),
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct ArtworkArgs {
    #[arg(long)]
    token_id: Option<String>,
    #[arg(long)]
    chain_id: Option<String>,
    #[arg(long)]
    deployment_id: Option<String>,
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
    /// Token symbol as the list spells it.
    symbol: String,
}

#[derive(Args)]
pub struct AddArgs {
    /// Chain that hosts the token.
    #[arg(long)]
    chain: String,
    /// Symbol, as it should be displayed.
    #[arg(long)]
    symbol: String,
    /// Token name.
    #[arg(long)]
    name: String,
    /// Contract address, mint, jetton master or coin type, per the chain.
    #[arg(long)]
    contract: String,
    /// How many decimal places the token has.
    #[arg(long)]
    decimals: u32,
    /// CoinGecko id, when the token has a quoted price.
    #[arg(long, default_value = "")]
    coingecko_id: String,
    /// CoinPaprika id, independently optional.
    #[arg(long, default_value = "")]
    coinpaprika_id: String,
}

#[derive(Args)]
pub struct RemoveArgs {
    /// Chain the token is on.
    #[arg(long)]
    chain: String,
    /// Contract address, mint, jetton master or coin type.
    #[arg(long)]
    contract: String,
}

#[derive(Args)]
pub struct DecimalsArgs {
    /// Chain the token is on.
    #[arg(long)]
    chain: String,
    /// Contract address, mint, jetton master or coin type.
    #[arg(long)]
    contract: String,
    /// How many decimal places the token has.
    #[arg(long)]
    decimals: u32,
}

#[derive(Args)]
pub struct ResetArgs {
    /// Required: this drops every custom token.
    #[arg(long)]
    yes: bool,
}

pub fn run(ctx: &Ctx, out: Out, command: TokenCommand) -> CliResult<()> {
    match command {
        TokenCommand::Catalog(args) => catalog(out, args),
        TokenCommand::Artwork(args) => {
            let name = if let Some(id) = args.token_id {
                spectra_core::store::token_artwork_name(id)
            } else if let Some(id) = args.chain_id {
                spectra_core::store::chain_artwork_name(id)
            } else {
                spectra_core::store::deployment_artwork_name(args.deployment_id)
            };
            out.emit(serde_json::json!({ "artworkName": name }));
            Ok(())
        }
        TokenCommand::List => list(ctx, out),
        TokenCommand::Track(args) => set_tracked(ctx, out, args, true),
        TokenCommand::Untrack(args) => set_tracked(ctx, out, args, false),
        TokenCommand::Add(args) => add(ctx, out, args),
        TokenCommand::Edit(args) => edit(ctx, out, args),
        TokenCommand::Remove(args) => remove(ctx, out, args),
        TokenCommand::Decimals(args) => decimals(ctx, out, args),
        TokenCommand::Reset(args) => reset(ctx, out, args),
        TokenCommand::Discover(args) => discover(ctx, out, args),
        TokenCommand::Format(args) => format_amount(ctx, out, args),
    }
}

fn catalog(out: Out, args: CatalogArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let tokens = spectra_core::tokens::list_token_deployments(chain.str_id().to_string());

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
                "deployment_id": token.deployment_id,
                "token_id": token.token_id,
                "kind": token.kind,
                "coingecko_id": token.coingecko_id,
                "coinpaprika_id": token.coinpaprika_id,
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
                "id": entry.token.deployment_id,
                "token_id": entry.token.token_id,
                "chain_id": entry.token.chain_id,
                "isEnabled": entry.is_enabled,
                "isBuiltIn": entry.is_built_in,
                "coingecko_id": entry.token.coingecko_id,
                "coinpaprika_id": entry.token.coinpaprika_id,
                "symbol": entry.token.symbol,
                "name": entry.token.name,
                "contract": entry.token.contract,
                "decimals": entry.token.decimals,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

/// Enable or disable a core-owned token preference.
fn set_tracked(ctx: &Ctx, out: Out, args: TrackArgs, is_enabled: bool) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let chain_name = chain.chain_display_name().to_string();
    CoreTokenHostingChain::from_chain_name(&chain_name)
        .ok_or_else(|| CliError::rejected(format!("{chain_name} does not support known tokens")))?;

    let matches: Vec<_> = ctx
        .state()?
        .token_preferences
        .into_iter()
        .filter(|entry| {
            entry
                .hosting_chain()
                .is_some_and(|h| h.chain_name() == chain_name)
                && (entry.token.deployment_id == args.symbol
                    || entry.token.symbol.eq_ignore_ascii_case(&args.symbol))
        })
        .collect();
    if matches.len() > 1 {
        return Err(CliError::usage(
            "ambiguous token symbol; use the deployment id from token catalog/list",
        ));
    }
    let entry = matches.into_iter().next().ok_or_else(|| {
        CliError::rejected(format!("{chain_name} has no token {:?}", args.symbol))
    })?;
    if entry.is_enabled == is_enabled {
        return Err(CliError::rejected(format!(
            "{} is already {}",
            entry.token.symbol,
            if is_enabled { "tracked" } else { "untracked" }
        )));
    }

    let transition = ctx.apply(StateCommand::SetTokenPreferencesEnabled {
        tokens: vec![CoreTokenPreferenceKey {
            chain_name: chain_name.clone(),
            contract: entry.token.contract.clone(),
        }],
        is_enabled,
    })?;
    reject_on_event(&transition)?;

    let verb = if is_enabled { "tracking" } else { "untracked" };
    out.text(|| {
        println!("  {} {verb} {}", out::ok_mark(), entry.token.symbol.bold());
        out::field("decimals", &entry.token.decimals.to_string());
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain_name,
        "id": entry.token.deployment_id,
                "symbol": entry.token.symbol,
        "contract": entry.token.contract,
        "decimals": entry.token.decimals,
        "isEnabled": is_enabled,
    }));
    Ok(())
}

/// Teach the wallet a token the catalog does not ship.
///
/// Every rule here is the reducer's: the symbol is trimmed and upper-cased,
/// the contract is judged by the hosting chain's own validator, a duplicate is
/// refused and the list comes back sorted. The composer held all four and this
/// command held none of them.
fn edit(ctx: &Ctx, out: Out, args: AddArgs) -> CliResult<()> {
    let transition = ctx.apply(StateCommand::UpdateCustomToken {
        chain_name: resolve_chain(&args.chain)?.chain_display_name().to_string(),
        contract: args.contract,
        symbol: args.symbol,
        name: args.name,
        coingecko_id: args.coingecko_id,
        coinpaprika_id: args.coinpaprika_id,
        decimals: args.decimals,
    })?;
    reject_on_event(&transition)?;
    out.emit(serde_json::json!({"ok": true}));
    Ok(())
}

fn add(ctx: &Ctx, out: Out, args: AddArgs) -> CliResult<()> {
    let chain_name = resolve_chain(&args.chain)?.chain_display_name().to_string();
    let transition = ctx.apply(StateCommand::AddCustomToken {
        chain_name: chain_name.clone(),
        symbol: args.symbol.clone(),
        name: args.name,
        contract: args.contract.clone(),
        coingecko_id: args.coingecko_id,
        coinpaprika_id: args.coinpaprika_id,
        decimals: args.decimals,
    })?;
    reject_on_event(&transition)?;

    let stored = transition
        .state
        .token_preferences
        .iter()
        .find(|entry| entry.token.contract.eq_ignore_ascii_case(&args.contract))
        .ok_or_else(|| CliError::failure("core accepted the token but did not store it"))?;
    out.text(|| {
        println!("  {} added {}", out::ok_mark(), stored.token.symbol.bold());
        out::field("chain", &stored.token.chain_id);
        out::field("contract", &stored.token.contract);
        out::field("decimals", &stored.token.decimals.to_string());
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": stored.token.chain_id,
        "symbol": stored.token.symbol,
        "contract": stored.token.contract,
        "decimals": stored.token.decimals,
    }));
    Ok(())
}

fn remove(ctx: &Ctx, out: Out, args: RemoveArgs) -> CliResult<()> {
    let chain_name = resolve_chain(&args.chain)?.chain_display_name().to_string();
    let transition = ctx.apply(StateCommand::RemoveCustomToken {
        chain_name: chain_name.clone(),
        contract: args.contract.clone(),
    })?;
    reject_on_event(&transition)?;
    out.text(|| println!("  {} removed {}", out::ok_mark(), args.contract.bold()));
    out.emit(serde_json::json!({
        "ok": true, "chain": chain_name, "contract": args.contract
    }));
    Ok(())
}

fn decimals(ctx: &Ctx, out: Out, args: DecimalsArgs) -> CliResult<()> {
    let chain_name = resolve_chain(&args.chain)?.chain_display_name().to_string();
    let transition = ctx.apply(StateCommand::SetCustomTokenDecimals {
        chain_name: chain_name.clone(),
        contract: args.contract.clone(),
        decimals: args.decimals,
    })?;
    reject_on_event(&transition)?;
    out.text(|| {
        println!(
            "  {} {} now shows {} places",
            out::ok_mark(),
            args.contract.bold(),
            args.decimals
        );
    });
    out.emit(serde_json::json!({
        "ok": true, "chain": chain_name, "contract": args.contract, "decimals": args.decimals
    }));
    Ok(())
}

fn reset(ctx: &Ctx, out: Out, args: ResetArgs) -> CliResult<()> {
    if !args.yes {
        return Err(CliError::usage("pass --yes: this drops every custom token"));
    }
    let transition = ctx.apply(StateCommand::ResetTokenPreferences)?;
    let count = transition.state.token_preferences.len();
    out.text(|| println!("  {} back to the catalog's {count} tokens", out::ok_mark()));
    out.emit(serde_json::json!({ "ok": true, "tokens": count }));
    Ok(())
}

/// A refusal core reported, as a command failure the shell can see.
///
/// The reducer answers with an event rather than an error because a front end
/// shows it beside the field; a command line has one exit code, so the reason
/// becomes the message.
fn reject_on_event(transition: &StateTransition) -> CliResult<()> {
    let reason = transition.events.iter().find_map(|event| match event {
        StateEvent::TokenPreferenceRejected { reason } => Some(*reason),
        _ => None,
    });
    use TokenPreferenceRejection as R;
    match reason {
        None => Ok(()),
        Some(reason) => Err(CliError::rejected(match reason {
            R::UnknownChain => "that chain does not host tokens",
            R::EmptySymbol => "a token needs a symbol",
            R::SymbolTooLong => "that symbol is too long to be one",
            R::InvalidPriceId => "use a provider ID, not a URL or name",
            R::EmptyName => "a token needs a name",
            R::EmptyContract => "a token needs a contract",
            R::InvalidContract => "that is not a valid contract for the chain",
            R::DuplicateToken => "that chain already knows this contract",
            R::TooManyDecimals => "more decimal places than any token has",
            R::BuiltInToken => "the catalog ships that token, so it is not yours to edit",
            R::UnknownToken => "no token with that contract on that chain",
        })),
    }
}

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
            let entry = spectra_core::tokens::list_token_deployments(chain.str_id().to_string())
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
        None => spectra_core::tokens::token_display_decimals(
            Some(chain.entry().native_deployment_id.clone()),
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

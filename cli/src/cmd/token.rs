//! Token commands update core-owned preference rows. Core validates custom
//! contracts and duplicates.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::{Chain, TokenStandard};
use spectra_core::store::state::{
    StateCommand, StateEvent, StateTransition, TokenPreferenceRejection,
};

use super::resolve_chain;
use crate::ctx::{Ctx, WalletArg, wallet_address};
use crate::error::{CliError, CliResult};
use crate::out::{self, Done, Out, Report};

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Tokens the build knows about for a chain.
    Catalog(CatalogArgs),
    /// Resolve bundled artwork by catalog identity; unknown identities have no mark.
    Artwork(ArtworkArgs),
    /// List stored preferences for built-in and custom tokens.
    List,
    /// Add a custom token to stored preferences.
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
    Discover(WalletArg),
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
    amount: String,
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
    /// Token symbol. Omit for the chain's native asset.
    #[arg(long)]
    symbol: Option<String>,
}

#[derive(Args)]
pub struct CatalogArgs {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
}

#[derive(Args)]
pub struct AddArgs {
    /// Chain that hosts the token.
    #[arg(long)]
    chain: String,
    /// The token's protocol, as `chains` lists it (TRC-10 or TRC-20, ERC-20
    /// or BEP-20, …). Required where the network has more than one.
    #[arg(long, value_parser = parse_standard)]
    standard: Option<TokenStandard>,
    /// Symbol, as it should be displayed.
    #[arg(long)]
    symbol: String,
    /// Token name.
    #[arg(long)]
    name: String,
    /// Contract address, TRC-10 numeric ID, mint, jetton master, coin type,
    /// XRP Ledger `CODE.rIssuer` or Stellar `CODE:ISSUER`.
    #[arg(long)]
    contract: String,
    /// How many decimal places the token has; the network's own number where
    /// its protocol fixes one.
    #[arg(long)]
    decimals: Option<u32>,
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
            let artwork_name = if let Some(id) = args.token_id {
                spectra_core::store::token_artwork_name(id)
            } else if let Some(id) = args.chain_id {
                spectra_core::store::chain_artwork_name(resolve_chain(&id)?)
            } else {
                spectra_core::store::deployment_artwork_name(args.deployment_id)
            };
            out.report(&ArtworkReport { artwork_name })
        }
        TokenCommand::List => list(ctx, out),
        TokenCommand::Add(args) => add(ctx, out, args),
        TokenCommand::Edit(args) => edit(ctx, out, args),
        TokenCommand::Remove(args) => remove(ctx, out, args),
        TokenCommand::Decimals(args) => decimals(ctx, out, args),
        TokenCommand::Reset(args) => reset(ctx, out, args),
        TokenCommand::Discover(args) => discover(ctx, out, args),
        TokenCommand::Format(args) => format_amount(out, args),
    }
}

/// The bundled mark for a catalog identity; none for an unknown one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtworkReport {
    artwork_name: String,
}

impl Report for ArtworkReport {
    fn render(&self) {
        if self.artwork_name.is_empty() {
            println!("  {}", out::hint("no artwork"));
        } else {
            println!("  {}", self.artwork_name);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogRow {
    deployment_id: String,
    token_id: String,
    coingecko_id: String,
    coinpaprika_id: String,
    symbol: String,
    name: String,
    contract: String,
    decimals: u32,
    standard: Option<TokenStandard>,
    tags: Vec<&'static str>,
}

/// The tokens this build ships for a chain.
#[derive(Serialize)]
struct CatalogReport {
    chain: Chain,
    tokens: Vec<CatalogRow>,
}

impl Report for CatalogReport {
    fn render(&self) {
        println!();
        if self.tokens.is_empty() {
            println!("  {}", out::hint("no tokens in the catalog for this chain"));
            return;
        }
        for token in &self.tokens {
            println!(
                "  {}  {:<8} {:<24} {}",
                out::tint("●", self.chain).bold(),
                token.symbol.bold(),
                token.name,
                out::hint(&format!("{} decimals", token.decimals)),
            );
            if !token.contract.is_empty() {
                println!("     {}", out::hint(&token.contract));
            }
        }
    }
}

fn catalog(out: Out, args: CatalogArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    out.report(&CatalogReport {
        chain,
        tokens: spectra_core::catalog::tokens::list_token_deployments(Some(chain))
            .into_iter()
            .map(|token| CatalogRow {
                tags: token.tags.iter().map(|tag| tag.as_str()).collect(),
                deployment_id: token.deployment_id,
                token_id: token.token_id,
                coingecko_id: token.coingecko_id,
                coinpaprika_id: token.coinpaprika_id,
                symbol: token.symbol,
                name: token.name,
                contract: token.contract,
                decimals: token.decimals,
                standard: token.token_standard,
            })
            .collect(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreferenceRow {
    id: String,
    token_id: String,
    chain: Chain,
    is_built_in: bool,
    coingecko_id: String,
    coinpaprika_id: String,
    symbol: String,
    name: String,
    contract: String,
    decimals: u32,
}

/// The tokens the wallet tracks: the catalog's and the ones added here.
#[derive(Serialize)]
struct PreferencesReport {
    tokens: Vec<PreferenceRow>,
}

impl Report for PreferencesReport {
    fn render(&self) {
        println!();
        if self.tokens.is_empty() {
            println!("  {}", out::hint("no known tokens"));
            return;
        }
        for token in &self.tokens {
            println!(
                "  {}  {:<8} {:<22} {}{}",
                out::tint("●", token.chain).bold(),
                token.symbol.bold(),
                token.name,
                out::hint(&format!("{} decimals", token.decimals)),
                if token.is_built_in {
                    String::new()
                } else {
                    format!("  {}", out::hint("custom"))
                },
            );
        }
    }
}

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let tokens = ctx
        .state()?
        .token_preferences
        .into_iter()
        .map(|entry| PreferenceRow {
            id: entry.token.deployment_id,
            token_id: entry.token.token_id,
            chain: entry.token.chain_id,
            is_built_in: entry.is_built_in,
            coingecko_id: entry.token.coingecko_id,
            coinpaprika_id: entry.token.coinpaprika_id,
            symbol: entry.token.symbol,
            name: entry.token.name,
            contract: entry.token.contract,
            decimals: entry.token.decimals,
        })
        .collect();
    out.report(&PreferencesReport { tokens })
}

/// Teach the wallet a token the catalog does not ship.
///
/// Every rule here is the reducer's: the symbol is trimmed and upper-cased,
/// the contract is judged by the hosting chain's own validator, a duplicate is
/// refused and the list comes back sorted.
fn edit(ctx: &Ctx, out: Out, args: AddArgs) -> CliResult<()> {
    if args.standard.is_some() {
        return Err(CliError::usage(
            "token edit cannot change the deployment standard",
        ));
    }
    let chain_id = resolve_chain(&args.chain)?;
    // The token keeps the protocol it was added under; its places follow it.
    let stored_standard = ctx
        .state()?
        .token_preferences
        .into_iter()
        .find_map(|entry| {
            (entry.token.chain_id == chain_id
                && entry.token.token_standard.is_some_and(|standard| {
                    spectra_core::catalog::tokens::validate_protocol_identifier(
                        chain_id,
                        standard,
                        &args.contract,
                    )
                    .is_ok_and(|contract| contract == entry.token.contract)
                }))
            .then_some(entry.token.token_standard)
            .flatten()
        });
    let decimals = args
        .decimals
        .or_else(|| stored_standard.and_then(|standard| standard.fixed_decimals()))
        .ok_or_else(|| CliError::usage("--decimals is required for this token's standard"))?;
    let transition = ctx.apply(StateCommand::UpdateCustomToken {
        chain_id,
        contract: args.contract.clone(),
        symbol: args.symbol,
        name: args.name,
        coingecko_id: args.coingecko_id,
        coinpaprika_id: args.coinpaprika_id,
        decimals,
    })?;
    reject_on_event(&transition)?;
    out.report(&Done::new(format!("updated {}", args.contract)))
}

fn add(ctx: &Ctx, out: Out, args: AddArgs) -> CliResult<()> {
    let chain_id = resolve_chain(&args.chain)?;
    // The places a token takes: its protocol's where the protocol fixes them,
    // otherwise the ones given. Which protocol is core's to settle: with
    // several and none named it refuses before storing anything, so the
    // places sent then are never read.
    let standards = spectra_core::catalog::chains::list_all_chains()
        .into_iter()
        .find(|entry| entry.id == chain_id.str_id())
        .map(|entry| entry.token_standards)
        .unwrap_or_default();
    let named = match args.standard {
        Some(standard) => standards.iter().find(|s| s.standard == standard),
        None if standards.len() == 1 => standards.first(),
        None => None,
    };
    let unsettled = args.standard.is_none() && standards.len() > 1;
    let decimals = match (args.decimals, named.and_then(|s| s.fixed_decimals)) {
        (Some(given), _) => given,
        (None, Some(fixed)) => fixed,
        (None, None) if unsettled => 0,
        (None, None) => {
            return Err(CliError::usage(
                "--decimals is required for this token's standard",
            ));
        }
    };
    let transition = ctx.apply(StateCommand::AddCustomToken {
        standard: args.standard,
        chain_id,
        symbol: args.symbol.clone(),
        name: args.name,
        contract: args.contract.clone(),
        coingecko_id: args.coingecko_id,
        coinpaprika_id: args.coinpaprika_id,
        decimals,
    })?;
    reject_on_event(&transition)?;

    let stored = transition
        .state
        .token_preferences
        .iter()
        .find(|entry| {
            entry.token.chain_id == chain_id
                && entry.token.token_standard.is_some_and(|standard| {
                    spectra_core::catalog::tokens::validate_protocol_identifier(
                        chain_id,
                        standard,
                        &args.contract,
                    )
                    .is_ok_and(|contract| contract == entry.token.contract)
                })
        })
        .ok_or_else(|| CliError::failure("core accepted the token but did not store it"))?;
    out.report(&AddedReport {
        chain: stored.token.chain_id,
        symbol: stored.token.symbol.clone(),
        contract: stored.token.contract.clone(),
        decimals: stored.token.decimals,
        standard: stored.token.token_standard,
    })
}

/// A token as core stored it: its symbol upper-cased, its contract in the
/// network's canonical form.
#[derive(Serialize)]
struct AddedReport {
    chain: Chain,
    symbol: String,
    contract: String,
    decimals: u32,
    standard: Option<TokenStandard>,
}

impl Report for AddedReport {
    fn render(&self) {
        println!("  {} added {}", out::ok_mark(), self.symbol.bold());
        out::field("chain", &out::chain(self.chain).to_string());
        out::field("contract", &self.contract);
        out::field("decimals", &self.decimals.to_string());
    }
}

fn remove(ctx: &Ctx, out: Out, args: RemoveArgs) -> CliResult<()> {
    let chain_id = resolve_chain(&args.chain)?;
    let transition = ctx.apply(StateCommand::RemoveCustomToken {
        chain_id,
        contract: args.contract.clone(),
    })?;
    reject_on_event(&transition)?;
    out.report(&TokenChangeReport {
        chain: chain_id,
        contract: args.contract.clone(),
        decimals: None,
        what: format!("removed {}", args.contract),
    })
}

/// A custom token changed, by its network and contract.
#[derive(Serialize)]
struct TokenChangeReport {
    chain: Chain,
    contract: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    decimals: Option<u32>,
    #[serde(skip)]
    what: String,
}

impl Report for TokenChangeReport {
    fn render(&self) {
        println!("  {} {}", out::ok_mark(), self.what);
    }
}

fn decimals(ctx: &Ctx, out: Out, args: DecimalsArgs) -> CliResult<()> {
    let chain_id = resolve_chain(&args.chain)?;
    let transition = ctx.apply(StateCommand::SetCustomTokenDecimals {
        chain_id,
        contract: args.contract.clone(),
        decimals: args.decimals,
    })?;
    reject_on_event(&transition)?;
    out.report(&TokenChangeReport {
        chain: chain_id,
        contract: args.contract.clone(),
        decimals: Some(args.decimals),
        what: format!("{} now shows {} places", args.contract, args.decimals),
    })
}

fn reset(ctx: &Ctx, out: Out, args: ResetArgs) -> CliResult<()> {
    if !args.yes {
        return Err(CliError::usage("pass --yes: this drops every custom token"));
    }
    let transition = ctx.apply(StateCommand::ResetTokenPreferences)?;
    out.report(&ResetReport {
        tokens: transition.state.token_preferences.len(),
    })
}

#[derive(Serialize)]
struct ResetReport {
    tokens: usize,
}

impl Report for ResetReport {
    fn render(&self) {
        println!(
            "  {} back to the catalog's {} tokens",
            out::ok_mark(),
            self.tokens
        );
    }
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
            R::StandardRequired => {
                "that chain has more than one token standard: name one with --standard"
            }
            R::UnsupportedStandard => "that chain does not support that token standard",
        })),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HeldRow {
    contract: String,
    symbol: String,
    is_known: bool,
    decimals: u8,
    balance: String,
}

/// The tokens an address holds, as its network reports them.
#[derive(Serialize)]
struct DiscoveredReport {
    holdings: Vec<HeldRow>,
}

impl Report for DiscoveredReport {
    fn render(&self) {
        println!();
        if self.holdings.is_empty() {
            println!("  {}", out::hint("this address holds no tokens"));
            return;
        }
        for token in &self.holdings {
            let name = if token.is_known {
                token.symbol.bold().to_string()
            } else {
                out::hint("unrecognised").to_string()
            };
            println!("  {:<24} {name}", token.balance.bold());
            println!("     {}", out::hint(&token.contract));
        }
    }
}

fn discover(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = wallet.chain_id;
    let address = wallet_address(&wallet).to_string();
    if address.is_empty() {
        return Err(CliError::rejected(format!(
            "{} has no address on {}",
            wallet.name,
            chain.chain_display_name()
        )));
    }
    let held = ctx
        .rt
        .block_on(ctx.service()?.discover_token_balances(chain, address))?;
    out.report(&DiscoveredReport {
        holdings: held
            .into_iter()
            .map(|token| HeldRow {
                contract: token.contract_address,
                symbol: token.symbol,
                is_known: token.is_known,
                decimals: token.decimals,
                balance: token.balance_display,
            })
            .collect(),
    })
}

/// The display rule, from outside core.
///
/// Places follow the amount, not a per-chain setting: a small balance keeps its
/// significant digits instead of rounding to nothing, and a large one does not
/// print six zeros it does not have.
fn format_amount(out: Out, args: FormatArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let asset_decimals = match &args.symbol {
        Some(symbol) => {
            let symbol_upper = symbol.to_uppercase();
            let entry = spectra_core::catalog::tokens::list_token_deployments(Some(chain))
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
        None => u32::from(chain.native_decimals()),
    };
    let text = spectra_core::amount::presentation::format_asset_amount(
        args.amount.clone(),
        asset_decimals,
    )
    .ok_or_else(|| CliError::usage("amount must be an unsigned decimal"))?;
    out.report(&FormatReport {
        shows: if text.below_threshold {
            format!("<{}", text.value)
        } else {
            text.value
        },
        asset_decimals,
        below_threshold: text.below_threshold,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FormatReport {
    shows: String,
    asset_decimals: u32,
    below_threshold: bool,
}

impl Report for FormatReport {
    fn render(&self) {
        out::wide_field(14, "shows", &self.shows);
        out::wide_field(14, "asset decimals", &self.asset_decimals.to_string());
    }
}

/// A token protocol named as `chains` spells it.
fn parse_standard(name: &str) -> Result<TokenStandard, String> {
    TokenStandard::parse(name).ok_or_else(|| {
        let names: Vec<&str> = TokenStandard::ALL
            .iter()
            .map(|standard| standard.as_str())
            .collect();
        format!("unknown token standard; one of: {}", names.join(", "))
    })
}

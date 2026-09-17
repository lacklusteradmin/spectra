//! Wallets enter through `WalletService::import_wallets` and change through
//! `StateCommand` — never by assembling a `WalletState` here. The previous
//! CLI did assemble them, and so skipped every rule core applies on the way
//! in, including address validation.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::derivation::import::{
    WalletImportCommit, WalletImportOutcome, WalletImportRequest, WalletImportWatchOnlyEntries,
};
use spectra_core::registry::Chain;
use spectra_core::store::state::{StateCommand, WalletState};
use spectra_core::store::wallet_domain::CoreSeedDerivationPaths;
use spectra_core::store::wallet_secrets;

use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx, SecretSource};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum WalletCommand {
    /// Core-derived portfolio and signing capabilities.
    Derived,
    /// Generate a new wallet and its seed phrase.
    New(NewArgs),
    /// Import a wallet from an existing seed phrase.
    Import(ImportArgs),
    /// Track an address without its keys.
    Watch(WatchArgs),
    /// List stored wallets.
    List,
    /// Show one wallet in detail.
    Show(SelectArgs),
    /// Show a wallet's receive address.
    Receive(SelectArgs),
    /// Rename a wallet.
    Rename(RenameArgs),
    /// Include or exclude a wallet from portfolio totals.
    Inclusion {
        wallet: String,
        #[arg(action = clap::ArgAction::Set)]
        included: bool,
    },
    /// Delete a wallet, its history and its secrets.
    Delete(DeleteArgs),
    /// Decrypt and print a wallet's seed phrase.
    Export(ExportArgs),
}

#[derive(Args)]
pub struct CreationArgs {
    /// Chain display name, registry id or symbol. Repeat it to import one
    /// seed across several chains; `new` takes exactly one.
    #[arg(long, required = true)]
    chain: Vec<String>,
    /// Wallet name (default: core assigns an available "Wallet N").
    #[arg(long)]
    name: Option<String>,
    /// Derivation path (default: the chain's catalog default).
    #[arg(long)]
    path: Option<String>,
    /// JSON file containing raw derivation fields (including exact passphrase text).
    #[arg(long)]
    derivation_input_file: Option<String>,
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
    /// Store the seed without a password. The material is not encrypted, so
    /// anything that can read the secret store can read the phrase.
    #[arg(long, conflicts_with_all = ["password_file"])]
    no_password: bool,
}

impl CreationArgs {
    /// `None` means store unsealed — the state the iOS app has always had for
    /// a wallet the user gave no password, and which core could not represent
    /// until it grew one.
    fn optional_password(&self) -> CliResult<Option<String>> {
        if self.no_password {
            return Ok(None);
        }
        self.password().map(Some)
    }

    fn password(&self) -> CliResult<String> {
        // The env default only counts when set, so an interactive run still
        // reaches the prompt.
        let env = self
            .password_env
            .clone()
            .filter(|name| std::env::var_os(name).is_some());
        SecretSource {
            file: self.password_file.clone(),
            env,
        }
        .resolve("password")
    }
}

#[derive(Args)]
pub struct NewArgs {
    #[command(flatten)]
    creation: CreationArgs,
    /// Seed phrase length: 12, 15, 18, 21 or 24.
    #[arg(long, default_value_t = 12)]
    words: u32,
}

#[derive(Args)]
pub struct ImportArgs {
    #[command(flatten)]
    creation: CreationArgs,
    /// Read the seed phrase from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    seed_file: Option<String>,
    /// Read the seed phrase from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_SEED")]
    seed_env: Option<String>,
    /// Import a raw private key instead of a phrase. Reads from this file;
    /// `-` means stdin.
    #[arg(long, value_name = "PATH", conflicts_with = "seed_file")]
    private_key_file: Option<String>,
    /// Import a raw private key instead of a phrase, from this variable.
    #[arg(long, value_name = "VAR")]
    private_key_env: Option<String>,
}

#[derive(Args)]
pub struct WatchArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
    /// Address to track. Repeat it to watch several: an import creates one
    /// wallet per address, which is what the app's multi-line input does.
    #[arg(long, required = true)]
    address: Vec<String>,
    /// Wallet name (default: core assigns an available "Wallet N").
    #[arg(long)]
    name: Option<String>,
}

#[derive(Args)]
pub struct SelectArgs {
    /// Wallet id, name or address.
    wallet: String,
}

#[derive(Args)]
pub struct RenameArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// New name.
    name: String,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Delete without asking for confirmation.
    #[arg(long)]
    yes: bool,
}

#[derive(Args)]
pub struct ExportArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Print the phrase without asking for confirmation.
    #[arg(long)]
    yes: bool,
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
}

pub fn run(ctx: &Ctx, out: Out, command: WalletCommand) -> CliResult<()> {
    match command {
        WalletCommand::New(args) => new(ctx, out, args),
        WalletCommand::Import(args) => import(ctx, out, args),
        WalletCommand::Watch(args) => watch(ctx, out, args),
        WalletCommand::List => list(ctx, out),
        WalletCommand::Derived => {
            let d = ctx
                .rt
                .block_on(ctx.service()?.wallet_derived_state())
                .map_err(CliError::from)?;
            out.emit(serde_json::to_value(d).map_err(|e| CliError::failure(e.to_string()))?);
            Ok(())
        }
        WalletCommand::Show(args) => show(ctx, out, args),
        WalletCommand::Receive(args) => receive(ctx, out, args),
        WalletCommand::Rename(args) => rename(ctx, out, args),
        WalletCommand::Inclusion { wallet, included } => {
            let wallet = ctx.find_wallet(&wallet)?;
            ctx.apply(StateCommand::SetWalletPortfolioInclusion {
                wallet_id: wallet.id,
                included,
            })?;
            out.emit(serde_json::json!({"ok":true}));
            Ok(())
        }
        WalletCommand::Delete(args) => delete(ctx, out, args),
        WalletCommand::Export(args) => export(ctx, out, args),
    }
}

// ─── Creating ───────────────────────────────────────────────────────────────

fn new(ctx: &Ctx, out: Out, args: NewArgs) -> CliResult<()> {
    let chain = only_chain(&args.creation)?;
    // The accepted lengths are core's, and core refuses the rest rather than
    // substituting twelve words. This used to hold its own `12 | 24` list
    // because it could not tell a generated eighteen-word phrase from the
    // twelve it would silently have been handed instead.
    let seed_phrase = spectra_core::service::generate_mnemonic(args.words)
        .map_err(|error| CliError::usage(error.to_string()))?;
    let outcome = seal_and_import(ctx, &args.creation, &[chain], &seed_phrase)?;

    let wallet = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!(
            "  {}  {}",
            out::accent("!").bold(),
            "save these words — anyone holding them can spend your funds".bold()
        );
        println!();
        print_words(&seed_phrase);
        println!();
        println!("  {} wallet created", out::ok_mark());
        print_wallet(&wallet);
    });
    out.emit(serde_json::json!({
        "ok": true,
        "seedPhrase": seed_phrase,
        "wallet": wallet_json(&wallet),
    }));
    Ok(())
}

fn import(ctx: &Ctx, out: Out, args: ImportArgs) -> CliResult<()> {
    let chains = resolve_chains(&args.creation.chain)?;
    let chain = chains[0];
    if args.private_key_file.is_some() || args.private_key_env.is_some() {
        return import_private_key(ctx, out, args, chain);
    }
    let env = args
        .seed_env
        .clone()
        .filter(|name| std::env::var_os(name).is_some());
    let seed_phrase = SecretSource {
        file: args.seed_file.clone(),
        env,
    }
    .resolve("seed phrase")?;

    crate::cmd::reject_bad_seed_phrase(&seed_phrase)?;

    let outcome = seal_and_import(ctx, &args.creation, &chains, &seed_phrase)?;
    let wallet = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!(
            "  {} imported a {}-word phrase",
            out::ok_mark(),
            seed_phrase.split_whitespace().count()
        );
        print_wallet(&wallet);
    });
    out.emit(serde_json::json!({ "ok": true, "wallet": wallet_json(&wallet) }));
    Ok(())
}

/// Import a wallet from a raw private key.
///
/// The last wallet operation the CLI could not drive. Core has dispatched
/// private-key derivation by chain since `core_derive_from_private_key`, so
/// what was missing was this command, not the derivation.
fn import_private_key(ctx: &Ctx, out: Out, args: ImportArgs, chain: Chain) -> CliResult<()> {
    if args.creation.derivation_input_file.is_some() {
        return Err(CliError::rejected(
            "Derivation overrides require a mnemonic wallet",
        ));
    }
    let env = args
        .private_key_env
        .clone()
        .filter(|name| std::env::var_os(name).is_some());
    let private_key = SecretSource {
        file: args.private_key_file.clone(),
        env,
    }
    .resolve("private key")?;
    let private_key = private_key.trim().trim_start_matches("0x").to_string();

    // Refuse before sealing anything: a chain with no private-key derivation
    // must not leave a key stored for a wallet that can never sign with it.
    // Core does the deriving — this call is the same rule the commit below
    // applies, asked early enough to keep the key out of the store.
    spectra_core::derivation::import::derive_private_key_import_address(
        &private_key,
        &[chain.chain_display_name().to_string()],
    )
    .map_err(CliError::rejected)?;

    let password = args.creation.password()?;

    let name = args.creation.name.clone().unwrap_or_default();
    let mut commit = commit_for(
        request_for(&[chain], &name),
        CoreSeedDerivationPaths::default(),
    );
    commit.request.is_private_key_import = true;
    commit.private_key = Some(private_key.clone());
    commit.password = Some(password);

    let service = ctx.service()?;
    let outcome = ctx.rt.block_on(service.import_wallets(commit))?;

    let wallet = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!("  {} imported a private key", out::ok_mark());
        print_wallet_of_kind(&wallet, Some("private key"));
    });
    out.emit(serde_json::json!({ "ok": true, "wallet": wallet_json(&wallet) }));
    Ok(())
}

/// Sealing first is the safer order: a failure afterwards leaves an orphan
/// secret under an id no wallet references, where the other order leaves a
/// wallet that looks spendable and is not.
/// Every chain the caller named, in the order they named them.
fn resolve_chains(names: &[String]) -> CliResult<Vec<Chain>> {
    names.iter().map(|n| resolve_chain(n)).collect()
}

/// The one chain a command that takes exactly one was given.
fn only_chain(args: &CreationArgs) -> CliResult<Chain> {
    let chains = resolve_chains(&args.chain)?;
    if chains.len() > 1 {
        return Err(CliError::usage(
            "this command takes one --chain; repeating it is for `wallet import`",
        ));
    }
    Ok(chains[0])
}

/// Seal the seed once and import a wallet on every chain named.
///
/// The addresses are core's: `import_wallets` derives one per selected chain
/// from the seed on the commit. This used to derive one here and pass it in,
/// which is why `--chain` could only be given once.
fn seal_and_import(
    ctx: &Ctx,
    args: &CreationArgs,
    chains: &[Chain],
    seed_phrase: &str,
) -> CliResult<WalletImportOutcome> {
    let password = args.optional_password()?;

    let name = args.name.clone().unwrap_or_default();
    let mut paths = CoreSeedDerivationPaths::default();
    for c in chains {
        let path = derivation_path(*c, args.path.as_deref())?;
        paths
            .by_chain
            .insert(c.mainnet_counterpart().str_id().to_string(), path);
    }
    let mut commit = commit_for(request_for(chains, &name), paths);
    commit.seed_phrase = Some(seed_phrase.to_string());
    commit.password = password;
    if let Some(path) = &args.derivation_input_file {
        let input = serde_json::from_str(
            &std::fs::read_to_string(path).map_err(|e| CliError::usage(e.to_string()))?,
        )
        .map_err(|e| CliError::usage(format!("invalid derivation input: {e}")))?;
        commit.derivation_overrides =
            spectra_core::derivation::input::core_parse_wallet_derivation_input(input);
    }

    let service = ctx.service()?;
    ctx.rt
        .block_on(service.import_wallets(commit))
        .map_err(CliError::from)
}

fn watch(ctx: &Ctx, out: Out, args: WatchArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    // Refuse here rather than let the planner refuse: this is the same flag the
    // app's watch-addresses picker is built from, so the two answer alike, and
    // "core considered it and said no" is exit 3 rather than the exit 1 an
    // error escaping the planner produced.
    if !chain.supports_watch_only_import() {
        return Err(CliError::rejected(format!(
            "{} cannot be watched without its keys",
            chain.chain_display_name()
        )));
    }
    let name = args.name.clone().unwrap_or_default();

    // Core mints one id per wallet it plans, which for a watch-only import is
    // one per address entry.
    let mut request = request_for(&[chain], &name);
    request.is_watch_only_import = true;
    request.watch_only_entries = WalletImportWatchOnlyEntries {
        by_slot: [(chain.address_slot().to_string(), args.address.clone())]
            .into_iter()
            .collect(),
        bitcoin_xpub: None,
    };

    let service = ctx.service()?;
    let outcome = ctx
        .rt
        .block_on(service.import_wallets(commit_for(request, CoreSeedDerivationPaths::default())))
        .map_err(CliError::from)?;

    // One wallet per address entry, which is what the planner expanded them
    // into — printing only the first hid the rest.
    let created: Vec<WalletState> = outcome
        .wallets
        .iter()
        .map(|wallet| wallet.to_wallet_state(true))
        .collect();
    let first = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!(
            "  {} {} watch-only wallet{} added",
            out::ok_mark(),
            created.len(),
            if created.len() == 1 { "" } else { "s" }
        );
        for wallet in &created {
            print_wallet(wallet);
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "count": created.len(),
        "wallet": wallet_json(&first),
        "wallets": created.iter().map(wallet_json).collect::<Vec<_>>(),
    }));
    Ok(())
}

// ─── Reading ────────────────────────────────────────────────────────────────

fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let wallets = ctx.state()?.wallets;
    out.text(|| {
        if wallets.is_empty() {
            println!();
            println!("  {}", out::hint("no wallets yet"));
            println!(
                "  {} {}",
                out::hint("add one with"),
                out::info("spectra wallet new --chain Bitcoin")
            );
            return;
        }
        println!();
        for wallet in &wallets {
            println!(
                "  {}  {}  {}{}",
                out::wallet_dot(&wallet.chain_name, wallet.is_watch_only),
                wallet.name.bold(),
                out::tint(&wallet.chain_name, &wallet.chain_name).bold(),
                if wallet.is_watch_only {
                    out::hint(" watch").to_string()
                } else {
                    String::new()
                },
            );
            println!("     {}", out::info(wallet_address(wallet)));
        }
        println!();
        println!(
            "  {} {}",
            out::accent(&wallets.len().to_string()).bold(),
            out::hint(if wallets.len() == 1 {
                "wallet"
            } else {
                "wallets"
            }),
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallets": wallets.iter().map(wallet_json).collect::<Vec<_>>(),
    }));
    Ok(())
}

fn show(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    // Asked of the secret store, which is what decides whether this wallet can
    // sign and with what.
    let signing = wallet_secrets::is_private_key_backed(ctx.secrets.as_ref(), &wallet.id)?
        .then_some("private key");
    out.text(|| {
        println!();
        print_wallet_of_kind(&wallet, signing);
        out::field("id", &out::hint(&wallet.id).to_string());
    });
    out.emit(serde_json::json!({ "ok": true, "wallet": wallet_json(&wallet) }));
    Ok(())
}

fn receive(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let symbol = resolve_chain(&wallet.chain_name)
        .map(|chain| chain.coin_symbol().to_string())
        .unwrap_or_default();
    out.text(|| {
        println!();
        println!("  {}", wallet_address(&wallet).bold());
        println!();
        out::field(
            "chain",
            &out::tint(&wallet.chain_name, &wallet.chain_name).to_string(),
        );
        out::field("symbol", &symbol);
    });
    out.emit(serde_json::json!({
        "ok": true,
        "address": wallet_address(&wallet),
        "chain": wallet.chain_name,
        "symbol": symbol,
    }));
    Ok(())
}

// ─── Mutating ───────────────────────────────────────────────────────────────

fn rename(ctx: &Ctx, out: Out, args: RenameArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let new_name = args.name.trim().to_string();
    if new_name.is_empty() {
        return Err(CliError::rejected("a wallet name cannot be empty"));
    }
    let previous = wallet.name.clone();

    // Through the reducer, not by editing state and saving it. Core decides
    // whether a wallet may change and persists the result itself.
    ctx.apply(StateCommand::RenameWallet {
        wallet_id: wallet.id,
        name: new_name.clone(),
    })?;

    out.text(|| {
        println!(
            "  {} {} {} {}",
            out::ok_mark(),
            out::hint(&previous),
            out::hint("→"),
            new_name.bold()
        )
    });
    out.emit(serde_json::json!({ "ok": true, "from": previous, "to": new_name }));
    Ok(())
}

fn delete(ctx: &Ctx, out: Out, args: DeleteArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    if !args.yes {
        return Err(CliError::usage(format!(
            "this deletes \"{}\" ({}), its history and its seed — re-run with --yes",
            wallet.name, wallet.chain_name
        )));
    }

    ctx.apply(StateCommand::RemoveWallet {
        wallet_id: wallet.id.clone(),
    })?;

    out.text(|| println!("  {} deleted \"{}\"", out::ok_mark(), wallet.name));
    out.emit(serde_json::json!({ "ok": true, "deleted": wallet.id }));
    Ok(())
}

fn export(ctx: &Ctx, out: Out, args: ExportArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    if wallet.is_watch_only {
        return Err(CliError::rejected("a watch-only wallet has no seed phrase"));
    }
    // A wallet imported from a raw key has no phrase, and the store is what
    // knows which it is. Reporting "no sealed secret" for one was accurate
    // about the phrase and wrong about the wallet.
    let is_private_key = wallet_secrets::is_private_key_backed(ctx.secrets.as_ref(), &wallet.id)?;
    let what = if is_private_key {
        "private key"
    } else {
        "seed phrase"
    };
    if !args.yes {
        return Err(CliError::usage(format!(
            "this prints your {what} in plain text — re-run with --yes"
        )));
    }

    let env = args
        .password_env
        .clone()
        .filter(|name| std::env::var_os(name).is_some());
    // Asked for only when there is something to unlock: a wallet stored
    // without a password has nothing for it to decrypt.
    let password = if wallet_secrets::is_sealed(ctx.secrets.as_ref(), &wallet.id)? {
        Some(
            SecretSource {
                file: args.password_file.clone(),
                env,
            }
            .resolve("password")?,
        )
    } else {
        None
    };

    // The key a private-key wallet was imported from is the only copy this
    // side holds. Sealing one the CLI could never return would make it a lost
    // key, so export handles both — behind the same gate and the same password.
    if is_private_key {
        let key = wallet_secrets::load_private_key(
            ctx.secrets.as_ref(),
            &wallet.id,
            password.as_deref(),
        )?;
        out.text(|| {
            println!();
            println!("  {}", key.bold());
            println!();
            println!(
                "  {} {}",
                out::accent("!").bold(),
                "store this securely and clear your terminal".bold()
            );
        });
        out.emit(serde_json::json!({ "ok": true, "privateKey": *key }));
        return Ok(());
    }

    let seed_phrase =
        wallet_secrets::load_seed_phrase(ctx.secrets.as_ref(), &wallet.id, password.as_deref())?;

    out.text(|| {
        println!();
        print_words(&seed_phrase);
        println!();
        println!(
            "  {} {}",
            out::accent("!").bold(),
            "store this securely and clear your terminal".bold()
        );
    });
    out.emit(serde_json::json!({ "ok": true, "seedPhrase": *seed_phrase }));
    Ok(())
}

// ─── Building an import ─────────────────────────────────────────────────────

/// The derivation path a wallet is created with: the caller's, or the chain's
/// catalog default resolved by core.
fn derivation_path(chain: Chain, requested: Option<&str>) -> CliResult<String> {
    let resolution = spectra_core::app_core_resolve_derivation_path(
        chain.chain_display_name().to_string(),
        requested.unwrap_or_default().to_string(),
    )
    .map_err(CliError::from)?;
    Ok(resolution)
}

/// An import on `chains`, named `name`. Core mints the wallet ids, derives
/// the addresses and reads the selected networks itself.
fn request_for(chains: &[Chain], name: &str) -> WalletImportRequest {
    WalletImportRequest {
        wallet_name: name.to_string(),
        selected_chain_names: chains
            .iter()
            .map(|c| c.chain_display_name().to_string())
            .collect(),
        is_watch_only_import: false,
        is_private_key_import: false,
        watch_only_entries: WalletImportWatchOnlyEntries::default(),
    }
}

fn commit_for(
    request: WalletImportRequest,
    seed_derivation_paths: CoreSeedDerivationPaths,
) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request,
        seed_derivation_preset: Default::default(),
        seed_derivation_paths,
        derivation_overrides: Default::default(),
        seed_phrase: None,
        private_key: None,
    }
}

fn first_wallet(outcome: &WalletImportOutcome) -> CliResult<WalletState> {
    let is_watch_only = outcome.secret_kind == "watchOnly";
    outcome
        .wallets
        .first()
        .map(|wallet| wallet.to_wallet_state(is_watch_only))
        .ok_or_else(|| CliError::failure("core planned the import but created no wallet"))
}

// ─── Rendering ──────────────────────────────────────────────────────────────

fn print_wallet(wallet: &WalletState) {
    print_wallet_of_kind(wallet, None)
}

/// `signing` overrides the "type" line for a wallet whose key is not a phrase.
fn print_wallet_of_kind(wallet: &WalletState, signing: Option<&str>) {
    out::field("name", &wallet.name.bold().to_string());
    out::field(
        "chain",
        &out::tint(&wallet.chain_name, &wallet.chain_name).to_string(),
    );
    out::field(
        "type",
        if wallet.is_watch_only {
            "watch-only"
        } else {
            signing.unwrap_or("seed phrase")
        },
    );
    if let Some(path) = &wallet.derivation_path {
        out::field("path", &out::hint(path).to_string());
    }
    out::field("address", &out::info(wallet_address(wallet)).to_string());
}

fn print_words(seed_phrase: &str) {
    for (index, word) in seed_phrase.split_whitespace().enumerate() {
        let numbered = format!("{:>2}. {:<12}", index + 1, word);
        if (index + 1) % 4 == 0 {
            println!("  {numbered}");
        } else {
            print!("  {numbered}");
        }
    }
    if !seed_phrase.split_whitespace().count().is_multiple_of(4) {
        println!();
    }
}

fn wallet_json(wallet: &WalletState) -> serde_json::Value {
    serde_json::json!({
        "id": wallet.id,
        "name": wallet.name,
        "chain": wallet.chain_name,
        "address": wallet_address(wallet),
        // Every network of this wallet's family, by chain name. A wallet on a
        // family with testnets holds one address per network: the app used to
        // re-derive the testnet one from the seed on every read, so nothing
        // outside that app could see it and a sealed wallet could not produce
        // it at all.
        "addresses": wallet
            .addresses
            .iter()
            .map(|entry| (entry.chain_name.clone(), serde_json::json!(entry.address)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "derivationPath": wallet.derivation_path,
        "isWatchOnly": wallet.is_watch_only,
    })
}

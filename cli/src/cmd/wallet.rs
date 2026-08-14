//! Wallet lifecycle: create, import, watch, inspect, rename, delete, export.
//!
//! Every wallet that lands in the store gets there through
//! `WalletService::import_wallets`, and every mutation afterwards through a
//! `StateCommand`. The previous CLI built `WalletSummary` values by hand and
//! wrote `CoreAppState` straight to disk, which meant it skipped every rule
//! core applies on the way in — including, after the last change, address
//! validation. A front end that assembles domain records itself is not a test
//! of core; it is a second implementation of it.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::derivation::import::{
    WalletImportAddresses, WalletImportCommit, WalletImportOutcome, WalletImportRequest,
    WalletImportWatchOnlyEntries,
};
use spectra_core::registry::Chain;
use spectra_core::store::state::{StateCommand, WalletSummary};
use spectra_core::store::wallet_domain::CoreSeedDerivationPaths;
use spectra_core::store::{wallet_db, wallet_secrets};

use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx, SecretSource};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum WalletCommand {
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
    /// Delete a wallet, its history and its secrets.
    Delete(DeleteArgs),
    /// Decrypt and print a wallet's seed phrase.
    Export(ExportArgs),
}

/// How to name a wallet and where to read its password.
#[derive(Args)]
pub struct CreationArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
    /// Wallet name (default: "My <chain> Wallet").
    #[arg(long)]
    name: Option<String>,
    /// Derivation path (default: the chain's catalog default).
    #[arg(long)]
    path: Option<String>,
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
}

impl CreationArgs {
    fn password(&self) -> CliResult<String> {
        // The env default only counts when it is actually set; otherwise fall
        // through to the prompt so an interactive run still works.
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
    /// Seed phrase length: 12 or 24.
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
}

#[derive(Args)]
pub struct WatchArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
    /// Address to track.
    #[arg(long)]
    address: String,
    /// Wallet name (default: "<chain> (watch)").
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
        WalletCommand::Show(args) => show(ctx, out, args),
        WalletCommand::Receive(args) => receive(ctx, out, args),
        WalletCommand::Rename(args) => rename(ctx, out, args),
        WalletCommand::Delete(args) => delete(ctx, out, args),
        WalletCommand::Export(args) => export(ctx, out, args),
    }
}

// ─── Creating ───────────────────────────────────────────────────────────────

fn new(ctx: &Ctx, out: Out, args: NewArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.creation.chain)?;
    // Checked here rather than left to `generate_mnemonic`, which maps every
    // count that is not 24 to twelve words. Asking for 18 and silently getting
    // 12 is the kind of quiet substitution a wallet should never make.
    if !matches!(args.words, 12 | 24) {
        return Err(CliError::usage("--words must be 12 or 24"));
    }
    // Mnemonic generation is core's — the app calls the same function. A CLI
    // that generated its own would be a second source of entropy policy.
    let seed_phrase = spectra_core::service::generate_mnemonic(args.words);
    let outcome = seal_and_import(ctx, &args.creation, chain, &seed_phrase)?;

    let wallet = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!("  {}  {}", out::accent("!").bold(), "save these words — anyone holding them can spend your funds".bold());
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
    let chain = resolve_chain(&args.creation.chain)?;
    let env = args
        .seed_env
        .clone()
        .filter(|name| std::env::var_os(name).is_some());
    let seed_phrase = SecretSource {
        file: args.seed_file.clone(),
        env,
    }
    .resolve("seed phrase")?;

    // Core owns what counts as a mnemonic; the CLI only reports the verdict.
    if !spectra_core::service::validate_mnemonic(seed_phrase.clone()) {
        return Err(CliError::rejected(
            "not a valid BIP-39 English mnemonic (check the words and the count)",
        ));
    }

    let outcome = seal_and_import(ctx, &args.creation, chain, &seed_phrase)?;
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

/// Seal the seed, then hand the import to core; undo the seal if it refuses.
///
/// Sealing first is the safer order. A failure after sealing leaves an orphan
/// secret under an id no wallet references — invisible and inert. The other
/// order leaves a wallet listed that has no key behind it, which reads as
/// spendable and is not.
fn seal_and_import(
    ctx: &Ctx,
    args: &CreationArgs,
    chain: Chain,
    seed_phrase: &str,
) -> CliResult<WalletImportOutcome> {
    let path = derivation_path(chain, args.path.as_deref())?;
    let address = derive_address(chain, seed_phrase, &path)?;
    let password = args.password()?;
    let wallet_id = new_wallet_id();

    wallet_secrets::seal(ctx.secrets.as_ref(), &wallet_id, seed_phrase, &password)?;

    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("My {} Wallet", chain.chain_display_name()));
    let commit = signing_commit(chain, &wallet_id, &name, &path, &address);

    let service = ctx.service()?;
    match ctx.rt.block_on(service.import_wallets(commit)) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            let _ = wallet_secrets::delete(ctx.secrets.as_ref(), &wallet_id);
            Err(CliError::from(error))
        }
    }
}

fn watch(ctx: &Ctx, out: Out, args: WatchArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("{} (watch)", chain.chain_display_name()));

    let request = WalletImportRequest {
        wallet_name: name,
        default_wallet_name_start_index: 0,
        primary_selected_chain_name: chain.chain_display_name().to_string(),
        selected_chain_names: vec![chain.chain_display_name().to_string()],
        planned_wallet_ids: vec![new_wallet_id()],
        is_watch_only_import: true,
        is_private_key_import: false,
        has_wallet_password: false,
        resolved_addresses: WalletImportAddresses::default(),
        watch_only_entries: WalletImportWatchOnlyEntries {
            by_slot: [(chain.address_slot().to_string(), vec![args.address.clone()])]
                .into_iter()
                .collect(),
            bitcoin_xpub: None,
        },
    };

    let service = ctx.service()?;
    let outcome = ctx
        .rt
        .block_on(service.import_wallets(commit_for(request, CoreSeedDerivationPaths::default())))
        .map_err(CliError::from)?;

    let wallet = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!("  {} watch-only wallet added", out::ok_mark());
        print_wallet(&wallet);
    });
    out.emit(serde_json::json!({ "ok": true, "wallet": wallet_json(&wallet) }));
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
    out.text(|| {
        println!();
        print_wallet(&wallet);
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
        out::field("chain", &out::tint(&wallet.chain_name, &wallet.chain_name).to_string());
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
    let mut wallet = ctx.find_wallet(&args.wallet)?;
    let new_name = args.name.trim().to_string();
    if new_name.is_empty() {
        return Err(CliError::rejected("a wallet name cannot be empty"));
    }
    let previous = wallet.name.clone();
    wallet.name = new_name.clone();

    // Through the reducer, not by editing state and saving it. Core decides
    // whether a wallet may change and persists the result itself.
    ctx.apply(StateCommand::UpdateWalletIfPresent { wallet })?;

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
    // Keypool, owned addresses and history rows go too: removing the wallet
    // from the resident state only rewrites the wallet list.
    wallet_db::delete_wallet_data(&ctx.db_path(), &wallet.id).map_err(CliError::failure)?;
    if !wallet.is_watch_only {
        wallet_secrets::delete(ctx.secrets.as_ref(), &wallet.id)?;
    }

    out.text(|| println!("  {} deleted \"{}\"", out::ok_mark(), wallet.name));
    out.emit(serde_json::json!({ "ok": true, "deleted": wallet.id }));
    Ok(())
}

fn export(ctx: &Ctx, out: Out, args: ExportArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    if wallet.is_watch_only {
        return Err(CliError::rejected(
            "a watch-only wallet has no seed phrase",
        ));
    }
    if !args.yes {
        return Err(CliError::usage(
            "this prints your seed phrase in plain text — re-run with --yes",
        ));
    }

    let env = args
        .password_env
        .clone()
        .filter(|name| std::env::var_os(name).is_some());
    let password = SecretSource {
        file: args.password_file.clone(),
        env,
    }
    .resolve("password")?;

    let seed_phrase = wallet_secrets::unlock(ctx.secrets.as_ref(), &wallet.id, &password)?;

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

fn new_wallet_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// The derivation path a wallet is created with: the caller's, or the chain's
/// catalog default resolved by core.
fn derivation_path(chain: Chain, requested: Option<&str>) -> CliResult<String> {
    let resolution = spectra_core::app_core_resolve_derivation_path(
        chain.chain_display_name().to_string(),
        requested.unwrap_or_default().to_string(),
    )
    .map_err(CliError::from)?;
    Ok(resolution.normalized_path)
}

fn derive_address(chain: Chain, seed_phrase: &str, path: &str) -> CliResult<String> {
    spectra_core::derivation::dispatch::derive_for_chain_name(
        chain.chain_display_name(),
        seed_phrase,
        path,
        None,
        None,
        None,
        true,
        false,
        false,
    )
    .map_err(CliError::from)?
    .address
    .ok_or_else(|| {
        CliError::failure(format!(
            "no address could be derived for {}",
            chain.chain_display_name()
        ))
    })
}

/// A one-chain, one-wallet signing import.
fn signing_commit(
    chain: Chain,
    wallet_id: &str,
    name: &str,
    path: &str,
    address: &str,
) -> WalletImportCommit {
    // The path is carried in the derivation-path table rather than beside the
    // address: `to_summary` reads it from there, keyed by the chain's mainnet
    // counterpart, so a testnet wallet keeps its mainnet's path.
    let mut paths = CoreSeedDerivationPaths::default();
    paths.by_chain.insert(
        chain.mainnet_counterpart().str_id().to_string(),
        path.to_string(),
    );

    let request = WalletImportRequest {
        wallet_name: name.to_string(),
        default_wallet_name_start_index: 0,
        primary_selected_chain_name: chain.chain_display_name().to_string(),
        selected_chain_names: vec![chain.chain_display_name().to_string()],
        planned_wallet_ids: vec![wallet_id.to_string()],
        is_watch_only_import: false,
        is_private_key_import: false,
        has_wallet_password: true,
        resolved_addresses: WalletImportAddresses {
            by_slot: [(chain.address_slot().to_string(), address.to_string())]
                .into_iter()
                .collect(),
            bitcoin_xpub: None,
        },
        watch_only_entries: WalletImportWatchOnlyEntries::default(),
    };
    commit_for(request, paths)
}

fn commit_for(
    request: WalletImportRequest,
    seed_derivation_paths: CoreSeedDerivationPaths,
) -> WalletImportCommit {
    WalletImportCommit {
        request,
        holdings: Vec::new(),
        seed_derivation_preset: Default::default(),
        seed_derivation_paths,
        derivation_overrides: Default::default(),
        bitcoin_network_mode: Default::default(),
        dogecoin_network_mode: Default::default(),
    }
}

/// The wallet an import produced, as core stored it.
fn first_wallet(outcome: &WalletImportOutcome) -> CliResult<WalletSummary> {
    let is_watch_only = outcome.secret_kind == "watchOnly";
    outcome
        .wallets
        .first()
        .map(|wallet| wallet.to_summary(is_watch_only))
        .ok_or_else(|| CliError::failure("core planned the import but created no wallet"))
}

// ─── Rendering ──────────────────────────────────────────────────────────────

fn print_wallet(wallet: &WalletSummary) {
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
            "seed phrase"
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

fn wallet_json(wallet: &WalletSummary) -> serde_json::Value {
    serde_json::json!({
        "id": wallet.id,
        "name": wallet.name,
        "chain": wallet.chain_name,
        "address": wallet_address(wallet),
        "derivationPath": wallet.derivation_path,
        "isWatchOnly": wallet.is_watch_only,
    })
}


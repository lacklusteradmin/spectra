//! Wallets enter through `WalletService::import_wallets` and change through
//! `StateCommand` — never by assembling a `WalletState` here. The previous
//! CLI did assemble them, and so skipped every rule core applies on the way
//! in, including address validation.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::derivation::import::{
    WalletImportCommit, WalletImportKind, WalletImportOutcome, WalletImportRequest,
};
use spectra_core::registry::Chain;
use spectra_core::store::state::{StateCommand, WalletState};
use spectra_core::store::wallet_secrets;

use super::resolve_chain;
use crate::ctx::{Ctx, SecretSource, wallet_address};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum WalletCommand {
    /// Validate password and confirmation from environment variables; never print either.
    CheckPassword {
        #[arg(long, default_value = "SPECTRA_PASSWORD")]
        password_env: String,
        #[arg(long, default_value = "SPECTRA_PASSWORD_CONFIRMATION")]
        confirmation_env: String,
    },
    /// Judge a seed phrase from an environment variable as the import page
    /// does: its length and wordlist are inferred unless given. Prints the
    /// verdict, never the phrase.
    CheckSeed {
        #[arg(long, default_value = "SPECTRA_SEED")]
        seed_env: String,
        /// Fix the length instead of inferring it.
        #[arg(long)]
        words: Option<u32>,
        /// Fix the wordlist (`en`, `zh-hans`, `ja`, …) instead of detecting it.
        #[arg(long)]
        language: Option<String>,
        /// Judge the phrase in this chain's formats; BIP-39 when absent.
        #[arg(long)]
        chain: Option<String>,
    },
    /// Core-derived portfolio and signing capabilities.
    Derived,
    /// Generate a new wallet and its seed phrase.
    New(NewArgs),
    /// Import a wallet from a seed phrase or raw private key.
    Import(ImportArgs),
    /// Track addresses or a Bitcoin account xpub without its keys.
    Watch(WatchArgs),
    /// The ways a wallet can be added on a network, and what each accepts.
    Methods {
        /// Chain display name or registry id.
        #[arg(long)]
        chain: String,
    },
    /// What a wallet on a network will do, its limits, and the endpoints its
    /// first refresh reads from. Contacts nothing.
    Capabilities {
        /// Chain display name or registry id.
        #[arg(long)]
        chain: String,
    },
    /// The least a new account must receive to exist, read from the network
    /// (XRP and Stellar).
    Reserve {
        /// Chain display name or registry id.
        #[arg(long)]
        chain: String,
    },
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
    /// Decrypt and print a wallet's seed phrase or private key.
    Export(ExportArgs),
}

#[derive(Args)]
pub struct CreationArgs {
    /// Chain display name or registry id. A wallet is on one network; a
    /// phrase used on another network is a second import.
    #[arg(long)]
    chain: String,
    /// Wallet name (default: core assigns an available "Wallet N").
    #[arg(long)]
    name: Option<String>,
    /// Derivation path, for a path no profile names (default: the chain's
    /// default profile at account 0).
    #[arg(long, conflicts_with_all = ["profile", "account"])]
    path: Option<String>,
    /// Derivation profile, as `wallet methods` lists it: `standard`,
    /// `legacy`, `nestedSegWit`, `nativeSegWit` or `taproot` (default: the
    /// chain's first).
    #[arg(long)]
    profile: Option<String>,
    /// Account index on the profile (default 0).
    #[arg(long)]
    account: Option<u32>,
    /// JSON file containing raw derivation fields (including exact passphrase text).
    #[arg(long)]
    derivation_input_file: Option<String>,
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
    /// Encrypt the seed with a local key stored in this data directory, without a wallet password.
    #[arg(long, conflicts_with_all = ["password_file"])]
    no_password: bool,
    /// Monero only: the block height the wallet's scan starts from. A new
    /// wallet starts near now; an import from its Polyseed's birthday, or
    /// from the start of the chain.
    #[arg(long)]
    restore_height: Option<u64>,
}

impl CreationArgs {
    /// `None` means store unsealed, and only `--no-password` says so. A
    /// password read from a file, the environment or the prompt is passed as
    /// `Some` even when blank, so core refuses it rather than storing the
    /// wallet in the clear.
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
        .resolve("password", "password-file")
    }
}

#[derive(Args)]
pub struct NewArgs {
    #[command(flatten)]
    creation: CreationArgs,
    /// Phrase length, one the chain's created format has: 12, 15, 18, 21
    /// or 24 for BIP-39, 25 for Monero, 24 for TON. Defaults to the shortest.
    #[arg(long)]
    words: Option<u32>,
}

#[derive(Args)]
pub struct ImportArgs {
    #[command(flatten)]
    creation: CreationArgs,
    /// Print the address the import would store, and store nothing.
    #[arg(long)]
    preview: bool,
    /// NEAR only: a named account (`alice.near`) the key controls. The
    /// import confirms on the network that the key is one of its full-access
    /// keys, and the wallet holds the named account.
    #[arg(long)]
    named_account: Option<String>,
    /// TON only: the wallet contract the key's account is under, `w5`
    /// (default) or `v4R2` for a wallet created before 2024.
    #[arg(long, value_name = "VERSION")]
    ton_wallet: Option<String>,
    /// Read the seed phrase from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    seed_file: Option<String>,
    /// Read the seed phrase from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_SEED")]
    seed_env: Option<String>,
    /// Import a private key instead of a phrase, in hex or the chain's own
    /// encoding (`wallet methods` lists them). Reads from this file;
    /// `-` means stdin.
    #[arg(long, value_name = "PATH", conflicts_with = "seed_file")]
    private_key_file: Option<String>,
    /// Import a private key instead of a phrase, from this variable.
    #[arg(long, value_name = "VAR")]
    private_key_env: Option<String>,
}

#[derive(Args)]
pub struct WatchArgs {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
    /// Address to track. Repeat it to watch several: an import creates one
    /// wallet per address, which is what the app's multi-line input does.
    #[arg(long, required_unless_present = "xpub", conflicts_with = "xpub")]
    address: Vec<String>,
    /// Bitcoin account public key: xpub/ypub/zpub or testnet tpub/upub/vpub.
    #[arg(long, required_unless_present = "address", conflicts_with = "address")]
    xpub: Option<String>,
    /// Wallet name (default: core assigns an available "Wallet N").
    #[arg(long)]
    name: Option<String>,
    /// Print the addresses the watch would store, and store nothing.
    #[arg(long)]
    preview: bool,
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
    /// Confirm printing the wallet's secret in plain text.
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
        WalletCommand::CheckPassword {
            password_env,
            confirmation_env,
        } => {
            let password = std::env::var(password_env)
                .map_err(|_| CliError::usage("Password environment variable is missing"))?;
            let confirmation = std::env::var(confirmation_env)
                .map_err(|_| CliError::usage("Confirmation environment variable is missing"))?;
            let rejection =
                spectra_core::validation::validate_wallet_password(password, confirmation);
            out.emit(serde_json::json!({"valid": rejection.is_none(), "rejection": rejection}));
            out.text(|| println!("{}", serde_json::to_string(&rejection).unwrap()));
            Ok(())
        }
        WalletCommand::CheckSeed {
            seed_env,
            words,
            language,
            chain,
        } => {
            let phrase = std::env::var(&seed_env)
                .map_err(|_| CliError::usage("Seed environment variable is missing"))?;
            let chain = chain.as_deref().map(resolve_chain).transpose()?;
            if let Some(code) = &language
                && !spectra_core::validation::seed_phrase_languages(chain)
                    .iter()
                    .any(|offered| &offered.code == code)
            {
                return Err(CliError::usage(format!("unknown wordlist {code:?}")));
            }
            let verdict = spectra_core::validation::check_seed_phrase(
                spectra_core::validation::SeedPhraseCheck {
                    words: phrase.split_whitespace().map(str::to_string).collect(),
                    language,
                    word_count: words,
                    chain,
                },
            );
            out.emit(serde_json::json!({
                "wordCount": verdict.word_count,
                "wordCountInferred": verdict.word_count_inferred,
                "language": verdict.language.as_ref().map(|l| &l.code),
                "languageDetected": verdict.language_detected,
                "format": verdict.format,
                "isComplete": verdict.is_complete,
                "isValid": verdict.is_valid,
                "invalidWordCount": verdict.invalid_words.len(),
                "problem": verdict.problem,
            }));
            out.text(|| {
                let language = verdict
                    .language
                    .as_ref()
                    .map_or("unknown", |l| l.name.as_str());
                let state = if verdict.is_valid {
                    "valid"
                } else {
                    "not valid"
                };
                println!("{} words, {language}: {state}", verdict.word_count);
                if let Some(problem) = verdict.problem.clone() {
                    println!("{}", super::seed_phrase_problem_text(problem));
                }
            });
            Ok(())
        }
        WalletCommand::New(args) => new(ctx, out, args),
        WalletCommand::Import(args) => import(ctx, out, args),
        WalletCommand::Watch(args) => watch(ctx, out, args),
        WalletCommand::Methods { chain } => methods(out, &chain),
        WalletCommand::Capabilities { chain } => capabilities(ctx, out, &chain),
        WalletCommand::Reserve { chain } => {
            let chain = resolve_chain(&chain)?;
            let reserve = ctx
                .rt
                .block_on(ctx.service()?.account_reserve(chain))
                .map_err(CliError::from)?;
            out.text(|| println!("  {} {}", reserve.amount, reserve.symbol));
            out.emit(serde_json::json!({
                "ok": true,
                "chain": reserve.chain,
                "amount": reserve.amount,
                "symbol": reserve.symbol,
            }));
            Ok(())
        }
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
    let chain = resolve_chain(&args.creation.chain)?;
    // Core generates in the chain's own format and validates the length.
    let words = match args.words {
        Some(words) => words,
        None => {
            let format = spectra_core::derivation::setup::wallet_setup_descriptor(chain)
                .option(spectra_core::derivation::setup::WalletSetupMethod::CreatePhrase)
                .and_then(|option| option.formats.first().copied())
                .ok_or_else(|| CliError::failure("the chain creates no phrase"))?;
            spectra_core::validation::seed_phrase_lengths(Some(chain))
                .into_iter()
                .find(|length| length.format == format)
                .map(|length| length.word_count)
                .ok_or_else(|| CliError::failure("the chain's format has no length"))?
        }
    };
    let seed_phrase = spectra_core::service::generate_seed_phrase(chain, words)
        .map_err(|error| CliError::usage(error.to_string()))?;
    // A new Monero wallet has no outputs before now, so its scan starts here.
    let restore_height = match args.creation.restore_height {
        Some(height) => Some(height),
        None if chain.mainnet_counterpart() == Chain::Monero => Some(
            spectra_core::monero_heights::monero_new_wallet_restore_height(chain)
                .map_err(CliError::from)?,
        ),
        None => None,
    };
    let outcome = seal_and_import(ctx, &args.creation, chain, &seed_phrase, restore_height)?;

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
    let chain = resolve_chain(&args.creation.chain)?;
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
    .resolve("seed phrase", "seed-file")?;

    crate::cmd::reject_bad_seed_phrase(Some(chain), &seed_phrase)?;

    let mut commit = phrase_commit(
        &args.creation,
        chain,
        &seed_phrase,
        args.creation.restore_height,
    )?;
    commit.named_account = args.named_account.clone();
    commit.ton_wallet_version = ton_wallet_version(args.ton_wallet.as_deref())?;
    if args.preview {
        return preview(ctx, out, commit);
    }
    commit.password = args.creation.optional_password()?;
    let outcome = ctx
        .rt
        .block_on(ctx.service()?.import_wallets(commit))
        .map_err(CliError::from)?;
    let wallet = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!(
            "  {} imported a {}-word phrase{}",
            out::ok_mark(),
            seed_phrase.split_whitespace().count(),
            upgraded_note(&outcome)
        );
        print_wallet(&wallet);
    });
    out.emit(serde_json::json!({
        "ok": true,
        "upgraded": outcome.upgraded,
        "wallet": wallet_json(&wallet),
    }));
    Ok(())
}

/// Import a wallet from a raw private key.
///
/// The last wallet operation the CLI could not drive. Core has dispatched
/// private-key derivation by chain since `derive_from_private_key`, so
/// what was missing was this command, not the derivation.
fn import_private_key(ctx: &Ctx, out: Out, args: ImportArgs, chain: Chain) -> CliResult<()> {
    let creation = &args.creation;
    if creation.derivation_input_file.is_some()
        || creation.path.is_some()
        || creation.profile.is_some()
        || creation.account.is_some()
    {
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
    .resolve("private key", "private-key-file")?;
    // Core reads the key in any of the chain's own encodings.
    let private_key = private_key.trim().to_string();

    // Core derives the key's address on the chain before sealing anything.
    let name = args.creation.name.clone().unwrap_or_default();
    let mut commit = commit_for(request_for(chain, &name, WalletImportKind::PrivateKey));
    commit.private_key = Some(private_key.clone());
    commit.named_account = args.named_account.clone();
    commit.ton_wallet_version = ton_wallet_version(args.ton_wallet.as_deref())?;
    if args.preview {
        return preview(ctx, out, commit);
    }
    commit.password = args.creation.optional_password()?;

    let service = ctx.service()?;
    let outcome = ctx.rt.block_on(service.import_wallets(commit))?;

    let wallet = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!(
            "  {} imported a private key{}",
            out::ok_mark(),
            upgraded_note(&outcome)
        );
        print_wallet_of_kind(&wallet, Some("private key"));
    });
    out.emit(serde_json::json!({
        "ok": true,
        "upgraded": outcome.upgraded,
        "wallet": wallet_json(&wallet),
    }));
    Ok(())
}

/// Import a phrase on `chain`. Core derives the address during the import
/// commit, and seals the phrase before it stores the wallet: a failure
/// afterwards leaves an orphan secret under an id no wallet references, where
/// the other order leaves a wallet that looks spendable and is not.
fn seal_and_import(
    ctx: &Ctx,
    args: &CreationArgs,
    chain: Chain,
    seed_phrase: &str,
    restore_height: Option<u64>,
) -> CliResult<WalletImportOutcome> {
    let mut commit = phrase_commit(args, chain, seed_phrase, restore_height)?;
    commit.password = args.optional_password()?;

    let service = ctx.service()?;
    ctx.rt
        .block_on(service.import_wallets(commit))
        .map_err(CliError::from)
}

/// A phrase import on `chain` as the arguments describe it, before a password.
fn phrase_commit(
    args: &CreationArgs,
    chain: Chain,
    seed_phrase: &str,
    restore_height: Option<u64>,
) -> CliResult<WalletImportCommit> {
    let name = args.name.clone().unwrap_or_default();
    let mut commit = commit_for(request_for(chain, &name, WalletImportKind::Phrase));
    commit.derivation_path = derivation_path(chain, args)?;
    commit.seed_phrase = Some(seed_phrase.to_string());
    commit.restore_height = restore_height;
    if let Some(path) = &args.derivation_input_file {
        let input = serde_json::from_str(
            &std::fs::read_to_string(path).map_err(|e| CliError::usage(e.to_string()))?,
        )
        .map_err(|e| CliError::usage(format!("invalid derivation input: {e}")))?;
        commit.derivation_overrides =
            spectra_core::derivation::input::parse_wallet_derivation_input(input);
    }
    Ok(commit)
}

/// Print what `commit` would store, from core's preview: the planning the
/// import runs, with nothing sealed or stored.
fn preview(ctx: &Ctx, out: Out, commit: WalletImportCommit) -> CliResult<()> {
    let service = ctx.service()?;
    let preview = ctx
        .rt
        .block_on(service.preview_wallet_import(commit))
        .map_err(CliError::from)?;
    out.text(|| {
        println!();
        for address in &preview.addresses {
            println!("  {}  {address}", out::hint("would store"));
        }
        for address in &preview.rejected_addresses {
            println!("  {}  {address}", out::hint("would refuse"));
        }
        if let Some(name) = &preview.upgrades_wallet {
            println!("  {}  {name}", out::hint("would give keys to"));
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "preview": true,
        "addresses": preview.addresses,
        "rejectedAddresses": preview.rejected_addresses,
        "upgradesWallet": preview.upgrades_wallet,
    }));
    Ok(())
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
    let kind = match args.xpub {
        Some(xpub) => WalletImportKind::WatchAccountXpub { xpub },
        None => WalletImportKind::WatchAddresses {
            addresses: args.address,
        },
    };
    let request = request_for(chain, &name, kind);
    if args.preview {
        return preview(ctx, out, commit_for(request));
    }

    let service = ctx.service()?;
    let outcome = ctx
        .rt
        .block_on(service.import_wallets(commit_for(request)))
        .map_err(CliError::from)?;

    // One wallet per address entry, which is what the planner expanded them
    // into — printing only the first hid the rest.
    let created: Vec<WalletState> = outcome
        .wallets
        .iter()
        .map(|wallet| wallet.to_wallet_state())
        .collect::<Result<_, _>>()?;
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

/// Print the network's setup descriptor: what `import_wallets` will accept.
fn methods(out: Out, chain: &str) -> CliResult<()> {
    use spectra_core::derivation::setup::{WalletSecretFormat, WalletSetupMethod};
    let chain = resolve_chain(chain)?;
    let descriptor = spectra_core::derivation::setup::wallet_setup_descriptor(chain);
    let method_name = |method: WalletSetupMethod| match method {
        WalletSetupMethod::CreatePhrase => "create a phrase",
        WalletSetupMethod::ImportPhrase => "import a phrase",
        WalletSetupMethod::ImportPrivateKey => "import a private key",
        WalletSetupMethod::WatchAddresses => "watch addresses",
        WalletSetupMethod::WatchAccountXpub => "watch an account xpub",
    };
    let format_name = |format: WalletSecretFormat| match format {
        WalletSecretFormat::Bip39Phrase => "BIP-39 phrase (12–24 words)",
        WalletSecretFormat::MoneroPhrase => "Monero seed (25 words)",
        WalletSecretFormat::Polyseed => "Polyseed (16 words)",
        WalletSecretFormat::TonMnemonic => "TON mnemonic (24 words)",
        WalletSecretFormat::HexSecret32 => "32-byte hex secret",
        WalletSecretFormat::CardanoExtendedKey => "64-byte extended key (hex)",
        WalletSecretFormat::Wif => "WIF",
        WalletSecretFormat::SolanaKeypair => "base58 or JSON keypair",
        WalletSecretFormat::StellarSecretSeed => "secret seed (S…)",
        WalletSecretFormat::SuiPrivateKey => "suiprivkey1…",
        WalletSecretFormat::AptosPrivateKey => "AIP-80 key (ed25519-priv-0x…)",
        WalletSecretFormat::NearSecretKey => "key string (ed25519:…)",
        WalletSecretFormat::Address => "address",
        WalletSecretFormat::AccountXpub => "account xpub",
    };
    out.text(|| {
        println!();
        println!("  {}", out::tint(&super::chain_name(chain), chain).bold());
        for option in &descriptor.options {
            let formats: Vec<&str> = option.formats.iter().map(|f| format_name(*f)).collect();
            let fields: Vec<&str> = option
                .fields
                .iter()
                .map(|field| match field {
                    spectra_core::derivation::setup::WalletSetupField::RestoreHeight => {
                        "--restore-height"
                    }
                    spectra_core::derivation::setup::WalletSetupField::NamedAccount => {
                        "--named-account"
                    }
                    spectra_core::derivation::setup::WalletSetupField::TonWalletVersion => {
                        "--ton-wallet"
                    }
                })
                .collect();
            println!(
                "  {}  {}{}",
                out::hint(&format!("{:<22}", method_name(option.method))),
                formats.join(", "),
                if fields.is_empty() {
                    String::new()
                } else {
                    format!("  {}", out::hint(&format!("[{}]", fields.join(", "))))
                }
            );
            if !option.profiles.is_empty() {
                // The ids `--profile` takes, default first, each with an
                // `--account` index.
                let profiles: Vec<String> = option
                    .profiles
                    .iter()
                    .filter_map(|profile| serde_json::to_value(profile).ok())
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect();
                println!(
                    "  {}  {}",
                    " ".repeat(22),
                    out::hint(&format!("--profile {} --account N", profiles.join("|")))
                );
            }
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.str_id(),
        "options": serde_json::to_value(&descriptor.options)
            .map_err(|e| CliError::failure(e.to_string()))?,
    }));
    Ok(())
}

/// Print core's setup summary for a network: what the setup page's last
/// step shows before anything is committed.
fn capabilities(ctx: &Ctx, out: Out, chain: &str) -> CliResult<()> {
    let chain = resolve_chain(chain)?;
    let summary = ctx.rt.block_on(ctx.service()?.wallet_setup_summary(chain));
    out.text(|| {
        println!();
        println!("  {}", out::tint(&super::chain_name(chain), chain).bold());
        let coverage = |c: &spectra_core::service::CapabilityCoverage| {
            serde_json::to_value(c)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        };
        println!(
            "  {}  {}",
            out::hint(&format!("{:<16}", "balance")),
            coverage(&summary.balance)
        );
        println!(
            "  {}  {}",
            out::hint(&format!("{:<16}", "history")),
            coverage(&summary.history)
        );
        if let Some(tokens) = &summary.token_discovery {
            println!(
                "  {}  {}",
                out::hint(&format!("{:<16}", "token discovery")),
                coverage(tokens)
            );
        }
        println!(
            "  {}  {}",
            out::hint(&format!("{:<16}", "staking")),
            summary.staking
        );
        for limit in &summary.limits {
            println!("  {}  {:?}", out::hint(&format!("{:<16}", "limit")), limit);
        }
        for endpoint in &summary.endpoints {
            println!(
                "  {}  {}{}",
                out::hint(&format!("{:<16}", "reads from")),
                endpoint.endpoint,
                if endpoint.is_built_in {
                    ""
                } else {
                    "  (custom)"
                }
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "summary": serde_json::to_value(&summary).map_err(|e| CliError::failure(e.to_string()))?,
    }));
    Ok(())
}

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
                out::wallet_dot(wallet.chain_id, wallet.is_watch_only()),
                wallet.name.bold(),
                out::tint(&super::chain_name(wallet.chain_id), wallet.chain_id).bold(),
                if wallet.is_watch_only() {
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
    // Recorded on the wallet when its secret was stored.
    let signing = matches!(
        wallet.signing,
        spectra_core::store::state::WalletSigning::PrivateKey { .. }
    )
    .then_some("private key");
    out.text(|| {
        println!();
        print_wallet_of_kind(&wallet, signing);
        out::field("id", &out::hint(&wallet.id).to_string());
    });
    out.emit(serde_json::json!({ "ok": true, "wallet": wallet_json(&wallet) }));
    Ok(())
}

/// Ask core for the receive address on the wallet's network.
/// Core reserves UTXO indices and registers derived addresses as owned.
/// Repeated calls reuse the reserved index.
fn receive(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let network = wallet.chain_id;
    let chain_name = network.chain_display_name().to_string();
    let symbol = network.coin_symbol().to_string();
    let address = ctx
        .rt
        .block_on(
            ctx.service()?
                .receive_address(wallet.id.clone(), network, true),
        )
        .map_err(CliError::from)?
        .ok_or_else(|| {
            CliError::failure(format!(
                "no receive address for {:?} on {chain_name}",
                wallet.name
            ))
        })?;
    out.text(|| {
        println!();
        println!("  {}", address.bold());
        println!();
        out::field("chain", &out::tint(&chain_name, network).to_string());
        out::field("symbol", &symbol);
    });
    out.emit(serde_json::json!({
        "ok": true,
        "address": address,
        "chain": network.str_id(),
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
        let secrets = if wallet.is_watch_only() {
            ""
        } else {
            " and its stored signing material"
        };
        return Err(CliError::usage(format!(
            "this deletes \"{}\" ({}), its history{secrets} — re-run with --yes",
            wallet.name,
            super::chain_name(wallet.chain_id)
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
    if wallet.is_watch_only() {
        return Err(CliError::rejected(
            "a watch-only wallet has no signing material to export",
        ));
    }
    // A wallet imported from a raw key has no phrase; the wallet records which
    // it signs with.
    let is_private_key = matches!(
        wallet.signing,
        spectra_core::store::state::WalletSigning::PrivateKey { .. }
    );
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
    // Ask for a password only when the wallet has password protection.
    let password = if wallet.signing.requires_password() {
        Some(
            SecretSource {
                file: args.password_file.clone(),
                env,
            }
            .resolve("password", "password-file")?,
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

    // Core answers why a phrase is not revealed; the words are the CLI's.
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());
    use spectra_core::service::SeedPhraseReveal as Reveal;
    let seed_phrase = match service.reveal_seed_phrase(wallet.id.clone(), password)? {
        Reveal::Phrase { phrase } => phrase,
        Reveal::NotStored => return Err(CliError::rejected("this wallet stores no seed phrase")),
        Reveal::PasswordRequired => return Err(CliError::usage("this wallet needs its password")),
        Reveal::IncorrectPassword => return Err(CliError::rejected("incorrect password")),
        Reveal::PasswordNotRequired => {
            return Err(CliError::rejected("this wallet has no password"));
        }
    };

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
    out.emit(serde_json::json!({ "ok": true, "seedPhrase": seed_phrase }));
    Ok(())
}

// ─── Building an import ─────────────────────────────────────────────────────

/// The derivation path a phrase wallet is created with: `--path` as typed, a
/// profile's path from core, or `None` for core's default. Core refuses a path
/// that does not parse, and one on a chain that derives without a path.
fn derivation_path(chain: Chain, args: &CreationArgs) -> CliResult<Option<String>> {
    if args.path.is_some() {
        return Ok(args.path.clone());
    }
    if args.profile.is_none() && args.account.is_none() {
        return Ok(None);
    }
    let profile = match &args.profile {
        Some(name) => serde_json::from_value(serde_json::Value::String(name.clone()))
            .map_err(|_| CliError::usage(format!("unknown derivation profile {name:?}")))?,
        None => *chain.derivation_profiles().first().ok_or_else(|| {
            CliError::rejected(format!(
                "{} derives without a derivation path",
                chain.chain_display_name()
            ))
        })?,
    };
    spectra_core::derivation::path::derivation_profile_path(
        chain,
        profile,
        args.account.unwrap_or(0),
    )
    .map(Some)
    .map_err(CliError::from)
}

/// The TON wallet version `--ton-wallet` names, by its serialized name.
fn ton_wallet_version(
    name: Option<&str>,
) -> CliResult<Option<spectra_core::derivation::ton::TonWalletVersion>> {
    name.map(|name| {
        serde_json::from_value(serde_json::Value::String(name.to_string()))
            .map_err(|_| CliError::usage(format!("unknown TON wallet version {name:?}")))
    })
    .transpose()
}

/// An import on `chain`, named `name`. Core mints the wallet ids and derives
/// or validates the addresses itself.
fn request_for(chain: Chain, name: &str, kind: WalletImportKind) -> WalletImportRequest {
    WalletImportRequest {
        wallet_name: name.to_string(),
        chain,
        kind,
    }
}

fn commit_for(request: WalletImportRequest) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request,
        derivation_path: None,
        derivation_overrides: Default::default(),
        seed_phrase: None,
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
    }
}

/// What a signing import that upgraded a watched wallet adds to its line.
fn upgraded_note(outcome: &WalletImportOutcome) -> &'static str {
    if outcome.upgraded {
        " into the watched wallet"
    } else {
        ""
    }
}

fn first_wallet(outcome: &WalletImportOutcome) -> CliResult<WalletState> {
    outcome
        .wallets
        .first()
        .map(|wallet| wallet.to_wallet_state())
        .ok_or_else(|| CliError::failure("import completed without creating a wallet"))?
        .map_err(CliError::from)
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
        &out::tint(&super::chain_name(wallet.chain_id), wallet.chain_id).to_string(),
    );
    out::field(
        "type",
        if wallet.is_watch_only() {
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
        "chain": wallet.chain_id,
        "address": wallet_address(wallet),
        // Stored addresses for each network in the wallet's family.
        "addresses": wallet
            .addresses
            .iter()
            .map(|entry| (entry.chain_id.str_id().to_string(), serde_json::json!(entry.address)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "derivationPath": wallet.derivation_path,
        "restoreHeight": wallet.restore_height,
        "isWatchOnly": wallet.is_watch_only(),
    })
}

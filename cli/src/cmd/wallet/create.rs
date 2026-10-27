//! Adding a wallet: a new phrase, an imported phrase or key, watched
//! addresses or keys, or a copy of a wallet's key on another network. Core
//! derives every address and mints every id; a preview runs the same
//! planning and stores nothing.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::derivation::import::{
    WalletImportCommit, WalletImportKind, WalletImportOutcome, WalletImportRequest,
};
use spectra_core::registry::Chain;
use spectra_core::store::state::WalletState;

use super::{WalletView, render_words};
use crate::cmd::resolve_chain;
use crate::ctx::{Ctx, PasswordArgs, SecretSource, WalletArg};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

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
    /// default profile at account 0). On Polkadot and Bittensor, Substrate
    /// junctions such as `//polkadot//0/1` (default: the root key).
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
    #[command(flatten)]
    password: PasswordArgs,
    /// Encrypt the seed with a local key stored in this data directory, without a wallet password.
    #[arg(long, conflicts_with = "password_file")]
    no_password: bool,
    /// Monero, and a Zcash wallet's shielded pools: the block height the
    /// wallet's scan starts from. A new wallet starts near now; an import
    /// from its Polyseed's birthday, or from the start of the chain
    /// (Sapling's activation for Zcash).
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
        self.password.resolve().map(Some)
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
    /// Give this watched wallet (id, name or address) its keys. The import
    /// must hold its address: anything else is refused rather than added as
    /// a wallet of its own.
    #[arg(long, value_name = "WALLET")]
    upgrade: Option<String>,
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
    #[arg(long, required_unless_present_any = ["xpub", "multisig"], conflicts_with = "xpub")]
    address: Vec<String>,
    /// Account public key, in an encoding the network's wallets write
    /// (`wallet methods --chain` lists them): xpub/ypub/zpub, Ltub, dgub, kpub….
    #[arg(long, required_unless_present_any = ["address", "multisig"], conflicts_with = "address")]
    xpub: Option<String>,
    /// A Monero wallet's private view key, with its primary address as
    /// `--address`: the wallet scans what it receives and cannot spend.
    #[arg(long, requires = "address", conflicts_with = "xpub")]
    view_key: Option<String>,
    /// A multisig account's policy, in its network's form (`wallet methods
    /// --chain` names it): a UTXO network's sortedmulti descriptor, keys
    /// with their origins, whose cosigner phrase imported with `--upgrade`
    /// lets the wallet sign its share.
    #[arg(long, conflicts_with_all = ["address", "xpub", "view_key"])]
    multisig: Option<String>,
    /// Block height a view-only Monero wallet's scan starts at.
    #[arg(long, requires = "view_key")]
    restore_height: Option<u64>,
    /// Wallet name (default: core assigns an available "Wallet N").
    #[arg(long)]
    name: Option<String>,
    /// Print the addresses the watch would store, and store nothing.
    #[arg(long)]
    preview: bool,
}

#[derive(Args)]
pub struct CopyArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The network to add it to, as `wallet copy-targets` lists them.
    #[arg(long)]
    chain: String,
    /// The new wallet's name (default: core assigns an available "Wallet N").
    #[arg(long)]
    name: Option<String>,
    /// A phrase's derivation path on the new network, for a path no profile
    /// names (default: the network's default profile at account 0). On
    /// Polkadot and Bittensor, Substrate junctions (default: the root key).
    #[arg(long, conflicts_with_all = ["profile", "account"])]
    path: Option<String>,
    /// A phrase's derivation profile on the new network.
    #[arg(long)]
    profile: Option<String>,
    /// Account index on the profile (default 0).
    #[arg(long)]
    account: Option<u32>,
    /// Monero and Zcash: where the new wallet's scan starts.
    #[arg(long)]
    restore_height: Option<u64>,
    /// TON only: the wallet contract, `w5` (default) or `v4R2`.
    #[arg(long, value_name = "VERSION")]
    ton_wallet: Option<String>,
    /// Print the address the copy would store, and store nothing.
    #[arg(long)]
    preview: bool,
    // The source wallet's password, which opens its seal and seals the copy.
    #[command(flatten)]
    password: PasswordArgs,
}

// ─── Reports ────────────────────────────────────────────────────────────────

/// A new wallet and the phrase it was made from, shown once.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedReport {
    seed_phrase: String,
    wallet: WalletView,
}

impl Report for CreatedReport {
    fn render(&self) {
        println!();
        println!(
            "  {}  {}",
            out::accent("!").bold(),
            "save these words — anyone holding them can spend your funds".bold()
        );
        println!();
        render_words(&self.seed_phrase);
        println!();
        println!("  {} wallet created", out::ok_mark());
        self.wallet.render();
    }
}

/// A wallet an import added, or the watched wallet it gave keys to.
#[derive(Serialize)]
struct ImportedReport {
    #[serde(skip)]
    what: String,
    upgraded: bool,
    wallet: WalletView,
}

impl Report for ImportedReport {
    fn render(&self) {
        println!();
        println!(
            "  {} {}{}",
            out::ok_mark(),
            self.what,
            if self.upgraded {
                " into the watched wallet"
            } else {
                ""
            }
        );
        self.wallet.render();
    }
}

/// The wallets a watch added: one per address entry.
#[derive(Serialize)]
struct WatchedReport {
    count: usize,
    wallet: WalletView,
    wallets: Vec<WalletView>,
}

impl Report for WatchedReport {
    fn render(&self) {
        println!();
        println!(
            "  {} {} added",
            out::ok_mark(),
            out::count(self.count, "watch-only wallet")
        );
        for wallet in &self.wallets {
            println!();
            wallet.render();
        }
    }
}

/// What an import would store, with nothing sealed or stored.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewReport {
    preview: bool,
    addresses: Vec<String>,
    rejected_addresses: Vec<String>,
    upgrades_wallet: Option<String>,
}

impl Report for PreviewReport {
    fn render(&self) {
        println!();
        for address in &self.addresses {
            println!("  {}  {address}", out::hint("would store"));
        }
        for address in &self.rejected_addresses {
            println!("  {}  {address}", out::hint("would refuse"));
        }
        if let Some(name) = &self.upgrades_wallet {
            println!("  {}  {name}", out::hint("would give keys to"));
        }
    }
}

/// The networks a wallet can be copied to.
#[derive(Serialize)]
struct CopyTargetsReport {
    targets: Vec<Chain>,
}

impl Report for CopyTargetsReport {
    fn render(&self) {
        println!();
        for chain in &self.targets {
            println!("  {}  {}", out::chain(*chain), out::hint(chain.str_id()));
        }
    }
}

// ─── Commands ───────────────────────────────────────────────────────────────

pub fn new(ctx: &Ctx, out: Out, args: NewArgs) -> CliResult<()> {
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
    let seed_phrase = spectra_core::derivation::phrase::generate_seed_phrase(chain, words)
        .map_err(|error| CliError::usage(error.to_string()))?;
    // A new wallet on a scanning network has no outputs before now, so its
    // scan starts here.
    let restore_height = args
        .creation
        .restore_height
        .or_else(|| spectra_core::catalog::restore_heights::new_wallet_restore_height(chain));
    let mut commit = phrase_commit(&args.creation, chain, &seed_phrase, restore_height)?;
    commit.password = args.creation.optional_password()?;
    let outcome = ctx.rt.block_on(ctx.service()?.import_wallets(commit))?;
    let wallet = first_wallet(&outcome)?;
    out.report(&CreatedReport {
        seed_phrase,
        wallet: WalletView::from(&wallet),
    })
}

pub fn import(ctx: &Ctx, out: Out, args: ImportArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.creation.chain)?;
    let private_key = args.private_key_file.is_some() || args.private_key_env.is_some();
    let (mut commit, what) = if private_key {
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
        // Core reads the key in any of the chain's own encodings, and derives
        // its address on the chain before sealing anything.
        let key = SecretSource {
            file: args.private_key_file.clone(),
            env: args.private_key_env.clone(),
        }
        .resolve("private key", "private-key-file")?;
        let name = creation.name.clone().unwrap_or_default();
        let mut commit = commit_for(chain, &name, WalletImportKind::PrivateKey);
        commit.private_key = Some(key.trim().to_string());
        (commit, "imported a private key".to_string())
    } else {
        let seed_phrase = SecretSource {
            file: args.seed_file.clone(),
            env: args.seed_env.clone(),
        }
        .resolve("seed phrase", "seed-file")?;
        crate::cmd::reject_bad_seed_phrase(Some(chain), &seed_phrase)?;
        let commit = phrase_commit(
            &args.creation,
            chain,
            &seed_phrase,
            args.creation.restore_height,
        )?;
        let words = seed_phrase.split_whitespace().count();
        (commit, format!("imported a {words}-word phrase"))
    };
    commit.named_account = args.named_account.clone();
    commit.ton_wallet_version = ton_wallet_version(args.ton_wallet.as_deref())?;
    commit.upgrade_wallet_id = args
        .upgrade
        .as_deref()
        .map(|wallet| ctx.find_wallet(wallet).map(|wallet| wallet.id))
        .transpose()?;
    if args.preview {
        return preview(ctx, out, commit);
    }
    commit.password = args.creation.optional_password()?;
    let outcome = ctx.rt.block_on(ctx.service()?.import_wallets(commit))?;
    let wallet = first_wallet(&outcome)?;
    out.report(&ImportedReport {
        what,
        upgraded: outcome.upgraded,
        wallet: WalletView::from(&wallet),
    })
}

/// Print what `commit` would store, from core's preview: the planning the
/// import runs, with nothing sealed or stored.
fn preview(ctx: &Ctx, out: Out, commit: WalletImportCommit) -> CliResult<()> {
    let preview = ctx
        .rt
        .block_on(ctx.service()?.preview_wallet_import(commit))?;
    out.report(&PreviewReport {
        preview: true,
        addresses: preview.addresses,
        rejected_addresses: preview.rejected_addresses,
        upgrades_wallet: preview.upgrades_wallet,
    })
}

pub fn watch(ctx: &Ctx, out: Out, args: WatchArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    // Refused here with the flag the app's watch-addresses picker is built
    // from, so the two answer alike, and as a refusal (exit 3) rather than
    // the failure an error escaping the planner would be.
    if args.view_key.is_none()
        && args.xpub.is_none()
        && args.multisig.is_none()
        && !chain.supports_watch_only_import()
    {
        return Err(CliError::rejected(format!(
            "{} cannot be watched without its keys",
            chain.chain_display_name()
        )));
    }
    let name = args.name.clone().unwrap_or_default();
    // Core mints one id per wallet it plans, which for a watch-only import is
    // one per address entry.
    let kind = match (args.xpub, args.view_key) {
        _ if args.multisig.is_some() => WalletImportKind::WatchMultisig {
            policy: args.multisig.unwrap_or_default(),
        },
        (Some(xpub), _) => WalletImportKind::WatchAccountXpub { xpub },
        (None, Some(view_key)) => {
            let [address] = <[String; 1]>::try_from(args.address).map_err(|_| {
                CliError::rejected("A view key watches one address: its wallet's primary address")
            })?;
            WalletImportKind::WatchViewKey { address, view_key }
        }
        (None, None) => WalletImportKind::WatchAddresses {
            addresses: args.address,
        },
    };
    let mut commit = commit_for(chain, &name, kind);
    commit.restore_height = args.restore_height;
    if args.preview {
        return preview(ctx, out, commit);
    }
    let outcome = ctx.rt.block_on(ctx.service()?.import_wallets(commit))?;
    let wallets: Vec<WalletView> = outcome
        .wallets
        .iter()
        .map(|wallet| {
            wallet
                .to_wallet_state()
                .map(|state| WalletView::from(&state))
        })
        .collect::<Result<_, _>>()?;
    let first = first_wallet(&outcome)?;
    out.report(&WatchedReport {
        count: wallets.len(),
        wallet: WalletView::from(&first),
        wallets,
    })
}

pub fn copy_targets(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let targets = ctx
        .rt
        .block_on(ctx.service()?.wallet_copy_targets(wallet.id))?;
    out.report(&CopyTargetsReport { targets })
}

pub fn copy(ctx: &Ctx, out: Out, args: CopyArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = resolve_chain(&args.chain)?;
    let path = chosen_path(
        chain,
        args.path.as_deref(),
        args.profile.as_deref(),
        args.account,
    )?;
    // The source's password opens its seal and seals the copy; a watched
    // wallet has neither.
    let password = args.password.for_wallet(&wallet)?;
    let commit = spectra_core::service::WalletCopyCommit {
        source_wallet_id: wallet.id.clone(),
        chain,
        wallet_name: args.name.clone().unwrap_or_default(),
        password,
        derivation_path: path,
        restore_height: args.restore_height,
        ton_wallet_version: ton_wallet_version(args.ton_wallet.as_deref())?,
    };
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());
    if args.preview {
        let preview = ctx.rt.block_on(service.preview_wallet_copy(commit))?;
        return out.report(&PreviewReport {
            preview: true,
            addresses: preview.addresses,
            rejected_addresses: preview.rejected_addresses,
            upgrades_wallet: preview.upgrades_wallet,
        });
    }
    let outcome = ctx.rt.block_on(service.copy_wallet_to_network(commit))?;
    let copied = first_wallet(&outcome)?;
    out.report(&ImportedReport {
        what: format!("added to {}", chain.chain_display_name()),
        upgraded: outcome.upgraded,
        wallet: WalletView::from(&copied),
    })
}

// ─── Building an import ─────────────────────────────────────────────────────

/// A phrase import on `chain` as the arguments describe it, before a password.
fn phrase_commit(
    args: &CreationArgs,
    chain: Chain,
    seed_phrase: &str,
    restore_height: Option<u64>,
) -> CliResult<WalletImportCommit> {
    let name = args.name.clone().unwrap_or_default();
    let mut commit = commit_for(chain, &name, WalletImportKind::Phrase);
    commit.derivation_path = chosen_path(
        chain,
        args.path.as_deref(),
        args.profile.as_deref(),
        args.account,
    )?;
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

/// The path `--path`, or `--profile` and `--account`, name on `chain`;
/// `None` takes the chain's default. Core judges a typed path, and refuses
/// one on a chain that derives without a path.
fn chosen_path(
    chain: Chain,
    path: Option<&str>,
    profile: Option<&str>,
    account: Option<u32>,
) -> CliResult<Option<String>> {
    if let Some(path) = path {
        return Ok(Some(path.to_string()));
    }
    if profile.is_none() && account.is_none() {
        return Ok(None);
    }
    let profile = match profile {
        Some(name) => serde_json::from_value(serde_json::Value::String(name.to_string()))
            .map_err(|_| CliError::usage(format!("unknown derivation profile {name:?}")))?,
        None => *chain.derivation_profiles().first().ok_or_else(|| {
            CliError::rejected(if chain.derives_along_junctions() {
                format!(
                    "{} derives along a junction path (--path //hard/soft), not a profile",
                    chain.chain_display_name()
                )
            } else {
                format!(
                    "{} derives without a derivation path",
                    chain.chain_display_name()
                )
            })
        })?,
    };
    spectra_core::derivation::path::derivation_profile_path(chain, profile, account.unwrap_or(0))
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
fn commit_for(chain: Chain, name: &str, kind: WalletImportKind) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: name.to_string(),
            chain,
            kind,
        },
        derivation_path: None,
        derivation_overrides: Default::default(),
        seed_phrase: None,
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
        upgrade_wallet_id: None,
    }
}

fn first_wallet(outcome: &WalletImportOutcome) -> CliResult<WalletState> {
    outcome
        .wallets
        .first()
        .ok_or_else(|| CliError::failure("import completed without creating a wallet"))?
        .to_wallet_state()
        .map_err(CliError::from)
}

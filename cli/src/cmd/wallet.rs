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
    /// Track addresses, an account public key, or a Monero wallet's address
    /// with its view key, without the keys that spend.
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
    /// What a wallet's page offers, with what each action does, and its
    /// network's capabilities and limits. Contacts nothing.
    Actions(SelectArgs),
    /// Show a wallet's receive address.
    Receive(SelectArgs),
    /// The addresses an account-discovery UTXO wallet has handed out and the
    /// coins they hold, read from the network.
    Coins(SelectArgs),
    /// Rename a wallet.
    Rename(RenameArgs),
    /// Include or exclude a wallet from portfolio totals.
    Inclusion {
        wallet: String,
        #[arg(action = clap::ArgAction::Set)]
        included: bool,
    },
    /// Hide one of a wallet's holdings from its total and the portfolio; it
    /// stays sendable.
    Hide(HoldingArgs),
    /// Show a hidden holding again.
    Unhide(HoldingArgs),
    /// Sign a plain-text message with a wallet's key, in its network's
    /// scheme, to prove it holds its address.
    SignMessage(SignMessageArgs),
    /// The ERC-20 allowances an EVM wallet has granted that still stand,
    /// from its indexer's approval logs and a live read of each.
    Approvals(SelectArgs),
    /// Build the transaction that sets an allowance back to zero; sign and
    /// broadcast it with `send sign` and `send broadcast-signed`.
    Revoke(RevokeArgs),
    /// The ERC-721 and ERC-1155 tokens an EVM wallet holds, from its
    /// network's Blockscout inventory.
    Nfts(SelectArgs),
    /// Build the transaction that sends an NFT: one ERC-721 token, or a
    /// quantity of an ERC-1155 id. The standard and the wallet's ownership
    /// are read from the contract; sign and broadcast it with `send sign`
    /// and `send broadcast-signed`.
    SendNft(SendNftArgs),
    /// Scan a Zcash wallet's shielded pools from a lightwalletd server, a
    /// batch at a time until the scan is complete (one batch with `--once`).
    /// The first batch creates the shielded account from the seed.
    ZcashSync(ScanSyncArgs),
    /// Where a Zcash wallet's shielded scan stands, what it holds and the
    /// unified address it receives at.
    ZcashStatus(SelectArgs),
    /// Build the transaction that moves a Zcash wallet's transparent funds
    /// into its shielded pool; sign and broadcast it with `send sign` and
    /// `send broadcast-signed` to a lightwalletd server.
    Shield(SelectArgs),
    /// Build a payment from a Zcash wallet's shielded funds; sign and
    /// broadcast it with `send sign` and `send broadcast-signed` to a
    /// lightwalletd server.
    SendShielded(SendShieldedArgs),
    /// Scan for a Litecoin wallet's MWEB funds from a Litecoin node, a batch
    /// at a time until the scan is complete (one batch with `--once`). The
    /// first batch derives the MWEB keys from the seed.
    MwebSync(ScanSyncArgs),
    /// Where a Litecoin wallet's MWEB scan stands, what it holds and the
    /// stealth address it receives at.
    MwebStatus(SelectArgs),
    /// Build the peg-in that moves a Litecoin wallet's transparent LTC into
    /// its MWEB funds; sign and broadcast it with `send sign` and
    /// `send broadcast-signed`.
    MwebPegin(MwebPeginArgs),
    /// Build a payment from a Litecoin wallet's MWEB funds, to an MWEB
    /// address or by a peg-out to any other; sign and broadcast it with
    /// `send sign` and `send broadcast-signed`.
    SendMweb(SendMwebArgs),
    /// An Ethereum wallet's ENS primary name, where it resolves back.
    Ens(SelectArgs),
    /// What the wallet's account on its network holds and needs: Tron's
    /// resources, XRP's and Stellar's reserve, a TON contract's state, a
    /// Substrate balance's parts, NEAR's storage, a Cardano base address's
    /// stake address with its rewards and delegation.
    Account(SelectArgs),
    /// Build the transaction that closes an XRP or Stellar account into
    /// another existing account, recovering its reserve: everything it
    /// holds goes there and the account is deleted. Sign and broadcast it
    /// with `send sign` and `send broadcast-signed`.
    Close(CloseArgs),
    /// A NEAR account's access keys, the one Spectra signs with marked.
    Keys(SelectArgs),
    /// Build the transaction that deletes one of a NEAR account's
    /// function-call keys; sign and broadcast it with `send sign` and
    /// `send broadcast-signed`.
    DeleteKey(DeleteKeyArgs),
    /// The NEAR token contracts holding a storage deposit the account can
    /// have back: registered, with none of their token left in it.
    TokenStorage(SelectArgs),
    /// Build the transaction that unregisters a NEAR account from a token
    /// contract it holds none of, returning the storage deposit; sign and
    /// broadcast it with `send sign` and `send broadcast-signed`.
    RefundStorage(RefundStorageArgs),
    /// A Sui wallet's coin types and how many objects hold each.
    Objects(SelectArgs),
    /// Build the transaction that merges a Sui coin type's objects into
    /// one; sign and broadcast it with `send sign` and `send
    /// broadcast-signed`.
    Merge(MergeArgs),
    /// A Solana wallet's empty token accounts and the rent each returns.
    TokenAccounts(SelectArgs),
    /// The proof of a Monero payment this wallet sent: the transaction key
    /// monero-wallet-cli's `check_tx_key` verifies.
    ProvePayment(ProvePaymentArgs),
    /// Build the transaction that closes empty Solana token accounts for
    /// their rent: the named ones, or every closable one (at most 20); sign
    /// and broadcast it with `send sign` and `send broadcast-signed`.
    CloseTokenAccounts(CloseTokenAccountsArgs),
    /// List an XRP Ledger or Stellar wallet's trust lines, each saying
    /// whether it can be removed.
    TrustLines(SelectArgs),
    /// Build the transaction that opens a trust line to an issued asset, so
    /// the wallet can hold it; sign and broadcast it with `send`.
    Trust(TrustArgs),
    /// Build the transaction that removes an empty trust line, freeing its
    /// reserve; sign and broadcast it with `send`.
    Untrust(TrustArgs),
    /// The networks a wallet's key, or watched address, can be added to.
    CopyTargets(SelectArgs),
    /// Add a wallet's key, or watched address, to another network as a
    /// wallet of its own, sealed under the same password.
    Copy(CopyArgs),
    /// Delete a wallet, its history and its secrets.
    Delete(DeleteArgs),
    /// Decrypt and print a wallet's seed phrase or private key, or with
    /// `--key` one of its keys as other wallets import it.
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
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
    /// Encrypt the seed with a local key stored in this data directory, without a wallet password.
    #[arg(long, conflicts_with_all = ["password_file"])]
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
    #[arg(long, required_unless_present_any = ["xpub", "descriptor"], conflicts_with = "xpub")]
    address: Vec<String>,
    /// Account public key, in an encoding the network's wallets write
    /// (`wallet methods --chain` lists them): xpub/ypub/zpub, Ltub, dgub, kpub….
    #[arg(long, required_unless_present_any = ["address", "descriptor"], conflicts_with = "address")]
    xpub: Option<String>,
    /// A Monero wallet's private view key, with its primary address as
    /// `--address`: the wallet scans what it receives and cannot spend.
    #[arg(long, requires = "address", conflicts_with = "xpub")]
    view_key: Option<String>,
    /// A Bitcoin multisig account's wsh(sortedmulti(…)) descriptor, keys
    /// with their origins. A cosigner's phrase imported with `--upgrade`
    /// lets the wallet sign its share.
    #[arg(long, conflicts_with_all = ["address", "xpub", "view_key"])]
    descriptor: Option<String>,
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
pub struct RevokeArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The token contract.
    #[arg(long)]
    token: String,
    /// The spender whose allowance goes to zero.
    #[arg(long)]
    spender: String,
}

/// A wallet whose private funds this device scans for: a Zcash wallet's
/// shielded pools or a Litecoin wallet's MWEB funds.
#[derive(Args)]
pub struct ScanSyncArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Scan one batch and stop.
    #[arg(long)]
    once: bool,
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
}

#[derive(Args)]
pub struct SendShieldedArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The recipient: a unified, Sapling or transparent Zcash address.
    #[arg(long)]
    to: String,
    /// How much ZEC, as an exact decimal.
    #[arg(long)]
    amount: String,
    /// A memo for a shielded recipient, at most 512 bytes.
    #[arg(long)]
    memo: Option<String>,
}

#[derive(Args)]
pub struct MwebPeginArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// How much LTC arrives in MWEB, as an exact decimal.
    #[arg(long)]
    amount: String,
}

#[derive(Args)]
pub struct SendMwebArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The recipient: an MWEB address, or any other Litecoin address for a
    /// peg-out.
    #[arg(long)]
    to: String,
    /// How much LTC, as an exact decimal.
    #[arg(long)]
    amount: String,
}

#[derive(Args)]
pub struct SendNftArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The collection's contract.
    #[arg(long)]
    contract: String,
    /// The token's id, a whole number in decimal.
    #[arg(long)]
    token_id: String,
    /// How many of an ERC-1155 token; an ERC-721 token is one.
    #[arg(long, default_value = "1")]
    quantity: String,
    /// The address that receives it.
    #[arg(long)]
    to: String,
}

#[derive(Args)]
pub struct CloseArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The existing account that receives everything.
    #[arg(long)]
    to: String,
    #[command(flatten)]
    memo: super::tx::MemoArgs,
}

#[derive(Args)]
pub struct ProvePaymentArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The transaction's id.
    #[arg(long)]
    txid: String,
}

#[derive(Args)]
pub struct TrustArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The asset: `CODE.rIssuer` on the XRP Ledger, `CODE:ISSUER` on
    /// Stellar.
    #[arg(long)]
    asset: String,
}

#[derive(Args)]
pub struct CloseTokenAccountsArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// A token account to close; repeat for several. None: every closable
    /// empty one.
    #[arg(long = "account")]
    accounts: Vec<String>,
}

#[derive(Args)]
pub struct MergeArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The coin type, such as `0x2::sui::SUI`.
    #[arg(long)]
    coin_type: String,
}

#[derive(Args)]
pub struct DeleteKeyArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The function-call key, as `ed25519:…`.
    #[arg(long)]
    key: String,
}

#[derive(Args)]
pub struct RefundStorageArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The token contract, such as `usdt.tether-token.near`.
    #[arg(long)]
    contract: String,
}

#[derive(Args)]
pub struct HoldingArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The holding: its deployment id (`ethereum:erc-20:0x…`) or a symbol
    /// only one of the wallet's holdings has.
    asset: String,
}

#[derive(Args)]
pub struct SignMessageArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The message, as text.
    #[arg(long)]
    message: String,
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
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
    /// Read the source wallet's password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the source wallet's password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
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
    /// The key to print, in the encoding its network's wallets import:
    /// `private-key`, `spend-key` or `view-key` (Monero), or `account-key`
    /// (an account's public key). `wallet actions` lists what a wallet has.
    #[arg(long, value_name = "KIND")]
    key: Option<String>,
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

fn print_zcash_status(status: &spectra_core::service::ZcashShieldedStatus) {
    println!();
    println!(
        "  scanned     {} / {}",
        status.scanned_height, status.chain_tip_height
    );
    println!("  spendable   {} ZEC", status.spendable);
    println!("  pending     {} ZEC", status.pending);
    println!("  shieldable  {} ZEC", status.shieldable);
    if let Some(address) = &status.address {
        println!("  address     {address}");
    }
}

fn print_mweb_status(status: &spectra_core::service::LitecoinMwebStatus) {
    println!();
    println!(
        "  scanned     block {} ({}‰)",
        status.scanned_height, status.progress_permille
    );
    println!("  spendable   {} LTC", status.spendable);
    println!("  pending     {} LTC", status.pending);
    if let Some(address) = &status.address {
        println!("  address     {address}");
    }
}

/// Build the trust line change `wallet trust` or `wallet untrust` asks for.
fn trust_change(ctx: &Ctx, out: Out, args: TrustArgs, remove: bool) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let service = ctx.service()?;
    let artifact = ctx
        .rt
        .block_on(async {
            if remove {
                service.build_remove_trust_line(wallet.id, args.asset).await
            } else {
                service.build_trust_asset(wallet.id, args.asset).await
            }
        })
        .map_err(CliError::from)?;
    super::tx::emit_artifact(out, &artifact);
    Ok(())
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
        WalletCommand::Actions(args) => actions(ctx, out, args),
        WalletCommand::Receive(args) => receive(ctx, out, args),
        WalletCommand::Coins(args) => coins(ctx, out, args),
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
        WalletCommand::SignMessage(args) => sign_message(ctx, out, args),
        WalletCommand::Approvals(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let approvals = ctx
                .rt
                .block_on(ctx.service()?.wallet_token_approvals(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| {
                println!();
                for approval in &approvals.approvals {
                    println!(
                        "  {} {}  {}  {}",
                        out::info(&approval.token),
                        approval.symbol,
                        out::hint(&approval.spender),
                        if approval.unlimited {
                            "unlimited".to_string()
                        } else {
                            approval.allowance.clone()
                        }
                    );
                }
                if !approvals.complete {
                    println!(
                        "  {}",
                        out::hint("the indexer scan stopped short; more may stand")
                    );
                }
            });
            out.emit(serde_json::json!({
                "ok": true,
                "approvals": serde_json::to_value(&approvals).map_err(|e| CliError::failure(e.to_string()))?,
            }));
            Ok(())
        }
        WalletCommand::Revoke(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.build_approval_revocation(
                    wallet.id,
                    args.token,
                    args.spender,
                ))
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::Nfts(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let nfts = ctx
                .rt
                .block_on(ctx.service()?.wallet_nfts(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| {
                println!();
                for nft in &nfts.nfts {
                    println!(
                        "  {:<8} {}  #{}  ×{}  {}",
                        nft.standard.label(),
                        out::info(&nft.contract),
                        nft.token_id,
                        nft.quantity,
                        nft.name.as_deref().unwrap_or(&nft.collection)
                    );
                }
                if !nfts.complete {
                    println!(
                        "  {}",
                        out::hint("the inventory was not read to its end; more may be held")
                    );
                }
            });
            out.emit(serde_json::json!({ "ok": true, "nfts": nfts }));
            Ok(())
        }
        WalletCommand::SendNft(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.build_nft_transfer(
                    wallet.id,
                    args.contract,
                    args.token_id,
                    args.quantity,
                    args.to,
                ))
                .map_err(CliError::from)?;
            out.text(|| {
                if let Some(spectra_core::send::stages::WalletOperation::TransferNft {
                    contract,
                    standard,
                    token_id,
                    quantity,
                    collection,
                    network_fee,
                }) = &artifact.operation
                {
                    println!();
                    println!("  token       {collection} #{token_id}");
                    println!("  standard    {}", standard.label());
                    println!("  contract    {contract}");
                    println!("  quantity    {quantity}");
                    println!("  to          {}", artifact.recipient);
                    println!("  fee         {network_fee}");
                    println!();
                }
                println!(
                    "{} {:?}\n{}\n{}",
                    artifact.id, artifact.stage, artifact.review_digest, artifact.prepared_details
                )
            });
            out.emit(serde_json::json!({ "artifact": artifact }));
            Ok(())
        }
        WalletCommand::ZcashSync(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let password = super::tx::signing_password(
                ctx,
                &wallet.id,
                args.password_file,
                args.password_env,
            )?;
            let service = ctx.service()?;
            // Each batch moves the scan on or finishes it.
            loop {
                let status = ctx
                    .rt
                    .block_on(service.sync_zcash_shielded(wallet.id.clone(), password.clone()))
                    .map_err(CliError::from)?;
                if args.once || status.complete {
                    out.text(|| print_zcash_status(&status));
                    out.emit(serde_json::json!({ "ok": true, "shielded": status }));
                    break;
                }
                eprintln!(
                    "Zcash scan: {} / {} ({}‰), {} transactions to read",
                    status.scanned_height,
                    status.chain_tip_height,
                    status.progress_permille,
                    status.unread_transactions
                );
            }
            Ok(())
        }
        WalletCommand::ZcashStatus(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let status = ctx
                .rt
                .block_on(ctx.service()?.zcash_shielded_status(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| print_zcash_status(&status));
            out.emit(serde_json::json!({ "ok": true, "shielded": status }));
            Ok(())
        }
        WalletCommand::Shield(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.build_zcash_shielding(wallet.id))
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::SendShielded(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.build_zcash_shielded_send(
                    wallet.id,
                    args.to,
                    args.amount,
                    args.memo,
                ))
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::MwebSync(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let password = super::tx::signing_password(
                ctx,
                &wallet.id,
                args.password_file,
                args.password_env,
            )?;
            let service = ctx.service()?;
            // Each batch moves the scan on or finishes it.
            loop {
                let status = ctx
                    .rt
                    .block_on(service.sync_litecoin_mweb(wallet.id.clone(), password.clone()))
                    .map_err(CliError::from)?;
                if args.once || status.complete {
                    out.text(|| print_mweb_status(&status));
                    out.emit(serde_json::json!({ "ok": true, "mweb": status }));
                    break;
                }
                eprintln!(
                    "MWEB scan: block {} ({}‰)",
                    status.scanned_height, status.progress_permille
                );
            }
            Ok(())
        }
        WalletCommand::MwebStatus(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let status = ctx
                .rt
                .block_on(ctx.service()?.litecoin_mweb_status(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| print_mweb_status(&status));
            out.emit(serde_json::json!({ "ok": true, "mweb": status }));
            Ok(())
        }
        WalletCommand::MwebPegin(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(
                    ctx.service()?
                        .build_litecoin_mweb_pegin(wallet.id, args.amount),
                )
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::SendMweb(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(
                    ctx.service()?
                        .build_litecoin_mweb_send(wallet.id, args.to, args.amount),
                )
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::Ens(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let name = ctx
                .rt
                .block_on(ctx.service()?.wallet_ens_name(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| println!("  {}", name.as_deref().unwrap_or("-")));
            out.emit(serde_json::json!({ "ok": true, "name": name }));
            Ok(())
        }
        WalletCommand::Account(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let account = ctx
                .rt
                .block_on(ctx.service()?.wallet_network_account(wallet.id))
                .map_err(CliError::from)?;
            let value = serde_json::to_value(&account.account)
                .map_err(|e| CliError::failure(e.to_string()))?;
            out.text(|| {
                println!();
                println!(
                    "  {}  {}",
                    out::hint(&format!("{:<22}", "symbol")),
                    account.symbol
                );
                println!(
                    "  {}  {}",
                    out::hint(&format!("{:<22}", "closable")),
                    account.closable
                );
                if let Some(fields) = value.as_object() {
                    for (name, field) in fields {
                        let text = field
                            .as_str()
                            .map_or_else(|| field.to_string(), str::to_string);
                        println!("  {}  {text}", out::hint(&format!("{name:<22}")));
                    }
                }
            });
            out.emit(serde_json::json!({
                "ok": true,
                "symbol": account.symbol,
                "closable": account.closable,
                "account": value,
            }));
            Ok(())
        }
        WalletCommand::Keys(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let keys = ctx
                .rt
                .block_on(ctx.service()?.wallet_access_keys(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| {
                println!();
                for key in &keys.keys {
                    let access = match &key.receiver {
                        None => "full access".to_string(),
                        Some(receiver) => format!(
                            "calls {receiver}{}, allowance {}",
                            if key.method_names.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", key.method_names.join(", "))
                            },
                            key.allowance.as_deref().unwrap_or("unlimited")
                        ),
                    };
                    let mark = if key.signs {
                        " (Spectra signs with it)"
                    } else {
                        ""
                    };
                    println!("  {}  {access}{mark}", key.public_key);
                }
            });
            out.emit(serde_json::json!({ "ok": true, "keys": keys }));
            Ok(())
        }
        WalletCommand::TokenStorage(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let storage = ctx
                .rt
                .block_on(ctx.service()?.wallet_token_storage(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| {
                println!();
                for deposit in &storage.deposits {
                    println!(
                        "  {}  {:<8} {} NEAR",
                        deposit.contract, deposit.symbol, deposit.refund
                    );
                }
                println!("  refundable  {} NEAR", storage.refundable);
            });
            out.emit(serde_json::json!({ "ok": true, "storage": storage }));
            Ok(())
        }
        WalletCommand::RefundStorage(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(
                    ctx.service()?
                        .build_token_storage_refund(wallet.id, args.contract),
                )
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::Objects(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let types = ctx
                .rt
                .block_on(ctx.service()?.wallet_coin_objects(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| {
                println!();
                for entry in &types {
                    println!(
                        "  {:<8} {:>6} objects  {}  {}",
                        entry.symbol, entry.objects, entry.balance, entry.coin_type
                    );
                }
            });
            out.emit(serde_json::json!({ "ok": true, "types": types }));
            Ok(())
        }
        WalletCommand::ProvePayment(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let proof = ctx
                .rt
                .block_on(ctx.service()?.monero_payment_proof(wallet.id, args.txid))
                .map_err(CliError::from)?
                .ok_or_else(|| {
                    CliError::usage("this device did not sign that Monero transaction")
                })?;
            out.text(|| {
                println!();
                println!(
                    "  check_tx_key {} {} {}",
                    proof.txid, proof.tx_key, proof.address
                );
                println!(
                    "  {}",
                    out::hint(&format!("proves {} XMR received", proof.amount))
                );
            });
            out.emit(serde_json::json!({ "ok": true, "proof": proof }));
            Ok(())
        }
        WalletCommand::TokenAccounts(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let empty = ctx
                .rt
                .block_on(ctx.service()?.wallet_empty_token_accounts(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| {
                println!();
                for account in &empty.accounts {
                    println!(
                        "  {}  {:<8} {} SOL  {}",
                        account.address,
                        account.symbol,
                        account.rent,
                        account.blocked.as_deref().unwrap_or("closable")
                    );
                }
                println!("  reclaimable  {} SOL", empty.reclaimable);
            });
            out.emit(serde_json::json!({ "ok": true, "empty": empty }));
            Ok(())
        }
        WalletCommand::TrustLines(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let lines = ctx
                .rt
                .block_on(ctx.service()?.wallet_trust_lines(wallet.id))
                .map_err(CliError::from)?;
            out.text(|| {
                println!();
                for line in &lines.lines {
                    println!(
                        "  {:<10} {}  {} of {}  {}",
                        line.code,
                        line.issuer,
                        line.balance,
                        line.limit,
                        line.removal_blocked.as_deref().unwrap_or("removable")
                    );
                }
                println!("  reserve per line  {}", lines.reserve_per_line);
            });
            out.emit(serde_json::json!({ "ok": true, "trust_lines": lines }));
            Ok(())
        }
        WalletCommand::Trust(args) => trust_change(ctx, out, args, false),
        WalletCommand::Untrust(args) => trust_change(ctx, out, args, true),
        WalletCommand::CloseTokenAccounts(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(
                    ctx.service()?
                        .build_token_account_closure(wallet.id, args.accounts),
                )
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::Merge(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.build_coin_merge(wallet.id, args.coin_type))
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::DeleteKey(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(
                    ctx.service()?
                        .build_access_key_deletion(wallet.id, args.key),
                )
                .map_err(CliError::from)?;
            super::tx::emit_artifact(out, &artifact);
            Ok(())
        }
        WalletCommand::Close(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.build_account_closing(
                    wallet.id,
                    args.to,
                    args.memo.memo(),
                ))
                .map_err(CliError::from)?;
            out.text(|| {
                if let Some(spectra_core::send::stages::WalletOperation::CloseAccount {
                    destination,
                    reserve,
                    removed_objects,
                    network_fee,
                }) = &artifact.operation
                {
                    println!();
                    println!(
                        "  {}  closing deletes {} on the network; this cannot be undone",
                        out::accent("!").bold(),
                        artifact.sender
                    );
                    println!("  to          {destination}");
                    println!("  amount      {} {}", artifact.amount, artifact.symbol);
                    println!("  reserve     {reserve} {}", artifact.symbol);
                    println!("  objects     {removed_objects}");
                    println!("  fee         {network_fee} {}", artifact.symbol);
                    println!();
                }
                println!(
                    "{} {:?}\n{}\n{}",
                    artifact.id, artifact.stage, artifact.review_digest, artifact.prepared_details
                )
            });
            out.emit(serde_json::json!({ "artifact": artifact }));
            Ok(())
        }
        WalletCommand::CopyTargets(args) => copy_targets(ctx, out, args),
        WalletCommand::Copy(args) => copy(ctx, out, args),
        WalletCommand::Hide(args) => set_hidden(ctx, out, args, true),
        WalletCommand::Unhide(args) => set_hidden(ctx, out, args, false),
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
    // A new wallet on a scanning network has no outputs before now, so its
    // scan starts here.
    let restore_height = args
        .creation
        .restore_height
        .or_else(|| spectra_core::restore_heights::new_wallet_restore_height(chain));
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
    commit.upgrade_wallet_id = upgrade_target(ctx, args.upgrade.as_deref())?;
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
    commit.upgrade_wallet_id = upgrade_target(ctx, args.upgrade.as_deref())?;
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
    if args.view_key.is_none()
        && args.xpub.is_none()
        && args.descriptor.is_none()
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
        _ if args.descriptor.is_some() => WalletImportKind::WatchMultisig {
            descriptor: args.descriptor.unwrap_or_default(),
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
    let mut commit = commit_for(request_for(chain, &name, kind));
    commit.restore_height = args.restore_height;
    if args.preview {
        return preview(ctx, out, commit);
    }

    let service = ctx.service()?;
    let outcome = ctx
        .rt
        .block_on(service.import_wallets(commit))
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
        WalletSetupMethod::WatchAccountXpub => "watch an account key",
        WalletSetupMethod::WatchViewKey => "watch with a view key",
        WalletSetupMethod::WatchMultisig => "watch a multisig",
    };
    // An account key is named by the prefixes this network writes it with.
    let account_key = format!(
        "account public key ({})",
        chain.account_key_prefixes().join(", ")
    );
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
        WalletSecretFormat::AccountXpub => account_key.as_str(),
        WalletSecretFormat::MoneroViewKey => "private view key (64 hex)",
        WalletSecretFormat::MultisigDescriptor => "wsh(sortedmulti(…)) descriptor",
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
                    spectra_core::derivation::setup::WalletSetupField::JunctionPath => {
                        "--path //hard/soft"
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
        "account_key_prefixes": chain.account_key_prefixes(),
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

/// Print core's action descriptor for a wallet: what its page offers.
fn actions(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let actions = ctx
        .rt
        .block_on(ctx.service()?.wallet_actions(wallet.id.clone()))
        .map_err(CliError::from)?;
    // The names `--json` carries, so the two outputs agree.
    let name = |value: serde_json::Result<serde_json::Value>| {
        value
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    out.text(|| {
        println!();
        println!(
            "  {}  {}",
            wallet.name.bold(),
            out::tint(&super::chain_name(wallet.chain_id), wallet.chain_id).bold()
        );
        for offer in &actions.actions {
            println!(
                "  {}  {}  {}",
                out::hint(&format!("{:<9}", name(serde_json::to_value(offer.section)))),
                format!("{:<13}", name(serde_json::to_value(offer.action))).bold(),
                offer.note
            );
        }
        for limit in &actions.summary.limits {
            println!("  {}  {:?}", out::hint(&format!("{:<9}", "limit")), limit);
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "actions": serde_json::to_value(&actions).map_err(|e| CliError::failure(e.to_string()))?,
    }));
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

fn coins(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let coins = ctx
        .rt
        .block_on(ctx.service()?.wallet_coins(wallet.id.clone()))
        .map_err(CliError::from)?;
    out.text(|| {
        println!();
        for address in &coins.addresses {
            let place = match (address.branch, address.index) {
                (Some(spectra_core::service::AddressBranch::Receive), Some(i)) => {
                    format!("receive {i}")
                }
                (Some(spectra_core::service::AddressBranch::Change), Some(i)) => {
                    format!("change {i}")
                }
                _ => "seen".to_string(),
            };
            println!(
                "  {}  {}  {} {}{}",
                out::hint(&format!("{place:<11}")),
                out::info(&address.address),
                address.balance,
                coins.symbol,
                if address.used { "" } else { "  (unused)" }
            );
        }
        if let Some(next) = &coins.next_receive_address {
            out::field("next", &out::info(next).to_string());
        }
        println!();
        for output in &coins.outputs {
            println!(
                "  {}:{}  {} {}  {} conf{}",
                output.txid,
                output.vout,
                output.amount,
                coins.symbol,
                output.confirmations,
                if output.spendable { "" } else { "  maturing" }
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "coins": serde_json::to_value(&coins).map_err(|e| CliError::failure(e.to_string()))?,
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

fn sign_message(ctx: &Ctx, out: Out, args: SignMessageArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let password = if wallet.signing.requires_password() {
        let env = args
            .password_env
            .clone()
            .filter(|name| std::env::var_os(name).is_some());
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
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());
    let signed = ctx
        .rt
        .block_on(service.sign_wallet_message(wallet.id.clone(), args.message, password))
        .map_err(CliError::from)?;
    let scheme =
        serde_json::to_value(signed.scheme).map_err(|e| CliError::failure(e.to_string()))?;
    out.text(|| {
        println!();
        out::field("address", &out::info(&signed.address).to_string());
        out::field("scheme", scheme.as_str().unwrap_or_default());
        out::field("signature", &signed.signature.bold().to_string());
    });
    out.emit(serde_json::json!({
        "ok": true,
        "address": signed.address,
        "scheme": scheme,
        "message": signed.message,
        "signature": signed.signature,
    }));
    Ok(())
}

fn copy_targets(ctx: &Ctx, out: Out, args: SelectArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let targets = ctx
        .rt
        .block_on(ctx.service()?.wallet_copy_targets(wallet.id.clone()))
        .map_err(CliError::from)?;
    out.text(|| {
        println!();
        for chain in &targets {
            println!(
                "  {}  {}",
                out::tint(&super::chain_name(*chain), *chain),
                out::hint(chain.str_id())
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "targets": targets.iter().map(|chain| chain.str_id()).collect::<Vec<_>>(),
    }));
    Ok(())
}

fn copy(ctx: &Ctx, out: Out, args: CopyArgs) -> CliResult<()> {
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
    let password = if wallet.signing.requires_password() {
        let env = args
            .password_env
            .clone()
            .filter(|name| std::env::var_os(name).is_some());
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
        let preview = ctx
            .rt
            .block_on(service.preview_wallet_copy(commit))
            .map_err(CliError::from)?;
        out.text(|| {
            for address in &preview.addresses {
                println!("  {}", address.bold());
            }
        });
        out.emit(serde_json::json!({
            "ok": true,
            "chain": chain.str_id(),
            "addresses": preview.addresses,
            "upgradesWallet": preview.upgrades_wallet,
        }));
        return Ok(());
    }
    let outcome = ctx
        .rt
        .block_on(service.copy_wallet_to_network(commit))
        .map_err(CliError::from)?;
    let copied = first_wallet(&outcome)?;
    out.text(|| {
        println!();
        println!(
            "  {} added to {}{}",
            out::ok_mark(),
            out::tint(&super::chain_name(chain), chain).bold(),
            upgraded_note(&outcome)
        );
        println!();
        print_wallet(&copied);
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet_json(&copied),
        "upgraded": outcome.upgraded,
    }));
    Ok(())
}

fn set_hidden(ctx: &Ctx, out: Out, args: HoldingArgs, hidden: bool) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let asset = args.asset.trim();
    let mut matches: Vec<String> = wallet
        .holdings
        .iter()
        .map(|holding| holding.deployment_id())
        .filter(|id| id == asset)
        .collect();
    if matches.is_empty() {
        matches = wallet
            .holdings
            .iter()
            .filter(|holding| holding.symbol.eq_ignore_ascii_case(asset))
            .map(|holding| holding.deployment_id())
            .collect();
    }
    // Showing a holding again names it by the id it was hidden under.
    if matches.is_empty() && !hidden && wallet.hidden_holdings.iter().any(|id| id == asset) {
        matches.push(asset.to_string());
    }
    let deployment_id = match matches.as_slice() {
        [one] => one.clone(),
        [] => {
            return Err(CliError::rejected(format!(
                "\"{}\" holds no {asset}",
                wallet.name
            )));
        }
        _ => {
            return Err(CliError::usage(format!(
                "more than one holding is {asset}; name it by its deployment id"
            )));
        }
    };
    ctx.apply(StateCommand::SetHoldingHidden {
        wallet_id: wallet.id,
        deployment_id: deployment_id.clone(),
        hidden,
    })?;
    out.text(|| {
        println!(
            "  {} {} {}",
            out::ok_mark(),
            if hidden { "hid" } else { "showed" },
            deployment_id
        )
    });
    out.emit(serde_json::json!({ "ok": true, "deploymentId": deployment_id, "hidden": hidden }));
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
    use spectra_core::service::WalletKeyKind as Kind;
    let wallet = ctx.find_wallet(&args.wallet)?;
    // A wallet imported from a raw key has no phrase: its export is its key,
    // written as its network's wallets read it.
    let key = match args.key.as_deref() {
        Some("private-key") => Some(Kind::PrivateKey),
        Some("spend-key") => Some(Kind::MoneroSpendKey),
        Some("view-key") => Some(Kind::MoneroViewKey),
        Some("account-key") => Some(Kind::AccountPublicKey),
        Some(other) => return Err(CliError::usage(format!("unknown key kind {other:?}"))),
        None if matches!(
            wallet.signing,
            spectra_core::store::state::WalletSigning::PrivateKey { .. }
        ) =>
        {
            Some(Kind::PrivateKey)
        }
        None if wallet.is_watch_only() => {
            return Err(CliError::rejected(
                "a watch-only wallet has no signing material to export",
            ));
        }
        None => None,
    };
    let what = match key {
        None => "seed phrase",
        Some(Kind::PrivateKey) => "private key",
        Some(Kind::MoneroSpendKey) => "spend key",
        Some(Kind::MoneroViewKey) => "view key",
        Some(Kind::AccountPublicKey) => "account key",
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
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());

    if let Some(kind) = key {
        let export = ctx
            .rt
            .block_on(service.export_wallet_key(wallet.id.clone(), kind, password))
            .map_err(CliError::from)?;
        out.text(|| {
            println!();
            println!("  {}", export.value.bold());
            println!();
            println!(
                "  {} {}",
                out::accent("!").bold(),
                "store this securely and clear your terminal".bold()
            );
        });
        let field = match kind {
            Kind::PrivateKey => "privateKey",
            Kind::MoneroSpendKey => "spendKey",
            Kind::MoneroViewKey => "viewKey",
            Kind::AccountPublicKey => "accountKey",
        };
        out.emit(serde_json::json!({
            "ok": true,
            field: export.value,
            "format": serde_json::to_value(export.format)
                .map_err(|e| CliError::failure(e.to_string()))?,
        }));
        return Ok(());
    }

    // Core answers why a phrase is not revealed; the words are the CLI's.
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
    chosen_path(
        chain,
        args.path.as_deref(),
        args.profile.as_deref(),
        args.account,
    )
}

/// The path `--path`, or `--profile` and `--account`, name on `chain`;
/// `None` takes the chain's default. Core judges a typed path.
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

/// The watched wallet `--upgrade` names, by id.
fn upgrade_target(ctx: &Ctx, wallet: Option<&str>) -> CliResult<Option<String>> {
    wallet
        .map(|wallet| ctx.find_wallet(wallet).map(|wallet| wallet.id))
        .transpose()
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
        upgrade_wallet_id: None,
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
    if let Some(principal) = &wallet.icp_principal {
        out::field("principal", &out::info(principal).to_string());
    }
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
        "hiddenHoldings": wallet.hidden_holdings,
        "icpPrincipal": wallet.icp_principal,
        "isWatchOnly": wallet.is_watch_only(),
    })
}

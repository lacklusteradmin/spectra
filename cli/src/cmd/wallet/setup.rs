//! What the setup page answers before a wallet exists: whether a password
//! and a phrase would be accepted, how a network takes a wallet, and what a
//! wallet there will do.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::derivation::setup::{
    WalletSecretFormat, WalletSetupField, WalletSetupMethod, WalletSetupOption,
};
use spectra_core::registry::Chain;
use spectra_core::service::{CapabilityCoverage, WalletSetupSummary};
use spectra_core::validation::{SeedPhraseProblem, WalletPasswordRejection};

use crate::cmd::{resolve_chain, seed_phrase_problem_text};
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Args)]
pub struct ChainArg {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
}

#[derive(Args)]
pub struct CheckPasswordArgs {
    #[arg(long, default_value = "SPECTRA_PASSWORD")]
    password_env: String,
    #[arg(long, default_value = "SPECTRA_PASSWORD_CONFIRMATION")]
    confirmation_env: String,
}

#[derive(Args)]
pub struct CheckSeedArgs {
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
}

#[derive(Serialize)]
struct PasswordReport {
    valid: bool,
    rejection: Option<WalletPasswordRejection>,
}

impl Report for PasswordReport {
    fn render(&self) {
        match &self.rejection {
            None => println!("  {} a password core accepts", out::ok_mark()),
            Some(WalletPasswordRejection::TooShort { min_chars }) => {
                println!("  {} shorter than {min_chars} characters", out::fail_mark())
            }
            Some(WalletPasswordRejection::ConfirmationMismatch) => {
                println!("  {} the confirmation does not match", out::fail_mark())
            }
        }
    }
}

pub fn check_password(out: Out, args: CheckPasswordArgs) -> CliResult<()> {
    let password = std::env::var(args.password_env)
        .map_err(|_| CliError::usage("Password environment variable is missing"))?;
    let confirmation = std::env::var(args.confirmation_env)
        .map_err(|_| CliError::usage("Confirmation environment variable is missing"))?;
    let rejection = spectra_core::validation::validate_wallet_password(password, confirmation);
    out.report(&PasswordReport {
        valid: rejection.is_none(),
        rejection,
    })
}

/// Core's verdict on a phrase, never the phrase.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SeedReport {
    word_count: u32,
    word_count_inferred: bool,
    language: Option<String>,
    language_detected: bool,
    format: Option<WalletSecretFormat>,
    is_complete: bool,
    is_valid: bool,
    invalid_word_count: usize,
    problem: Option<SeedPhraseProblem>,
    #[serde(skip)]
    language_name: Option<String>,
}

impl Report for SeedReport {
    fn render(&self) {
        let language = self.language_name.as_deref().unwrap_or("unknown wordlist");
        let (mark, state) = if self.is_valid {
            (out::ok_mark(), "valid")
        } else {
            (out::fail_mark(), "not valid")
        };
        println!("  {mark} {} words, {language}: {state}", self.word_count);
        if self.invalid_word_count > 0 {
            println!(
                "  {}",
                out::hint(&format!(
                    "{} words are not in the wordlist",
                    self.invalid_word_count
                ))
            );
        }
        if let Some(problem) = self.problem.clone() {
            println!("  {}", seed_phrase_problem_text(problem));
        }
    }
}

pub fn check_seed(out: Out, args: CheckSeedArgs) -> CliResult<()> {
    let phrase = std::env::var(&args.seed_env)
        .map_err(|_| CliError::usage("Seed environment variable is missing"))?;
    let chain = args.chain.as_deref().map(resolve_chain).transpose()?;
    if let Some(code) = &args.language
        && !spectra_core::validation::seed_phrase_languages(chain)
            .iter()
            .any(|offered| &offered.code == code)
    {
        return Err(CliError::usage(format!("unknown wordlist {code:?}")));
    }
    let verdict =
        spectra_core::validation::check_seed_phrase(spectra_core::validation::SeedPhraseCheck {
            words: phrase.split_whitespace().map(str::to_string).collect(),
            language: args.language,
            word_count: args.words,
            chain,
        });
    out.report(&SeedReport {
        word_count: verdict.word_count,
        word_count_inferred: verdict.word_count_inferred,
        language: verdict.language.as_ref().map(|l| l.code.clone()),
        language_detected: verdict.language_detected,
        format: verdict.format,
        is_complete: verdict.is_complete,
        is_valid: verdict.is_valid,
        invalid_word_count: verdict.invalid_words.len(),
        problem: verdict.problem,
        language_name: verdict.language.map(|l| l.name),
    })
}

/// The network's setup descriptor: what `import_wallets` will accept.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MethodsReport {
    chain: Chain,
    options: Vec<WalletSetupOption>,
    account_key_prefixes: Vec<&'static str>,
}

impl Report for MethodsReport {
    fn render(&self) {
        let account_key = format!(
            "account public key ({})",
            self.account_key_prefixes.join(", ")
        );
        println!();
        println!("  {}", out::chain(self.chain).bold());
        for option in &self.options {
            let formats: Vec<&str> = option
                .formats
                .iter()
                .map(|format| format_name(*format, &account_key))
                .collect();
            let fields: Vec<&str> = option.fields.iter().map(|field| flag_for(*field)).collect();
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
                let profiles: Vec<String> = option.profiles.iter().map(out::serde_name).collect();
                println!(
                    "  {}  {}",
                    " ".repeat(22),
                    out::hint(&format!("--profile {} --account N", profiles.join("|")))
                );
            }
        }
    }
}

fn method_name(method: WalletSetupMethod) -> &'static str {
    match method {
        WalletSetupMethod::CreatePhrase => "create a phrase",
        WalletSetupMethod::ImportPhrase => "import a phrase",
        WalletSetupMethod::ImportPrivateKey => "import a private key",
        WalletSetupMethod::WatchAddresses => "watch addresses",
        WalletSetupMethod::WatchAccountXpub => "watch an account key",
        WalletSetupMethod::WatchViewKey => "watch with a view key",
        WalletSetupMethod::WatchMultisig => "watch a multisig",
    }
}

fn flag_for(field: WalletSetupField) -> &'static str {
    match field {
        WalletSetupField::RestoreHeight => "--restore-height",
        WalletSetupField::NamedAccount => "--named-account",
        WalletSetupField::TonWalletVersion => "--ton-wallet",
        WalletSetupField::JunctionPath => "--path //hard/soft",
    }
}

/// A secret's format as a person names it; an account key by the prefixes
/// this network writes it with.
fn format_name(format: WalletSecretFormat, account_key: &str) -> &str {
    match format {
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
        WalletSecretFormat::AccountXpub => account_key,
        WalletSecretFormat::MoneroViewKey => "private view key (64 hex)",
        WalletSecretFormat::MultisigDescriptor => "sortedmulti(…) descriptor",
        WalletSecretFormat::SuiMultisigPublicKey => {
            "{\"threshold\", \"publicKeys\": [{\"publicKey\", \"weight\"}]}"
        }
        WalletSecretFormat::CardanoNativeScript => "cardano-cli native script JSON",
        WalletSecretFormat::SubstrateMultisig => "{\"threshold\", \"signatories\": [SS58…]}",
        WalletSecretFormat::AptosMultiKey => {
            "{\"signaturesRequired\", \"publicKeys\": [\"ed25519-pub-0x…\"]}"
        }
    }
}

pub fn methods(out: Out, args: ChainArg) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    out.report(&MethodsReport {
        chain,
        options: spectra_core::derivation::setup::wallet_setup_descriptor(chain).options,
        account_key_prefixes: chain.account_key_prefixes().to_vec(),
    })
}

/// Core's setup summary: what the setup page's last step shows before
/// anything is committed.
#[derive(Serialize)]
struct CapabilitiesReport {
    #[serde(skip)]
    chain: Chain,
    summary: WalletSetupSummary,
}

impl Report for CapabilitiesReport {
    fn render(&self) {
        let summary = &self.summary;
        let line = |label: &str, value: &str| out::wide_field(16, label, value);
        let coverage = |coverage: &CapabilityCoverage| out::serde_name(coverage);
        println!();
        println!("  {}", out::chain(self.chain).bold());
        line("balance", &coverage(&summary.balance));
        line("history", &coverage(&summary.history));
        if let Some(tokens) = &summary.token_discovery {
            line("token discovery", &coverage(tokens));
        }
        line("staking", &summary.staking.to_string());
        for limit in &summary.limits {
            line("limit", crate::cmd::limit_text(limit));
        }
        for endpoint in &summary.endpoints {
            let source = if endpoint.is_built_in {
                ""
            } else {
                "  (custom)"
            };
            line("reads from", &format!("{}{source}", endpoint.endpoint));
        }
    }
}

pub fn capabilities(ctx: &Ctx, out: Out, args: ChainArg) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let summary = ctx.rt.block_on(ctx.service()?.wallet_setup_summary(chain));
    out.report(&CapabilitiesReport { chain, summary })
}

/// The least a new account must hold to exist on its network.
#[derive(Serialize)]
struct ReserveReport {
    chain: Chain,
    amount: String,
    symbol: String,
}

impl Report for ReserveReport {
    fn render(&self) {
        println!(
            "  {} {}  {}",
            self.amount.bold(),
            self.symbol,
            out::hint("to open an account")
        );
    }
}

pub fn reserve(ctx: &Ctx, out: Out, args: ChainArg) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let reserve = ctx.rt.block_on(ctx.service()?.account_reserve(chain))?;
    out.report(&ReserveReport {
        chain: reserve.chain,
        amount: reserve.amount,
        symbol: reserve.symbol,
    })
}

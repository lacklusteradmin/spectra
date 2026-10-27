//! Using a wallet's keys: printing its phrase or one of its keys, and
//! signing a message to prove it holds its address.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::service::{SeedPhraseReveal, WalletKeyKind};
use spectra_core::store::state::WalletSigning;

use super::{render_secret_warning, render_words};
use crate::ctx::{Ctx, PasswordArgs};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

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
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct SignMessageArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The message, as text.
    #[arg(long)]
    message: String,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhraseReport {
    seed_phrase: String,
}

impl Report for PhraseReport {
    fn render(&self) {
        println!();
        render_words(&self.seed_phrase);
        render_secret_warning();
    }
}

/// One of a wallet's keys, under the field that names its kind.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    private_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spend_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    view_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_key: Option<String>,
    format: spectra_core::derivation::setup::WalletSecretFormat,
    #[serde(skip)]
    value: String,
}

impl Report for KeyReport {
    fn render(&self) {
        println!();
        println!("  {}", self.value.bold());
        render_secret_warning();
    }
}

pub fn export(ctx: &Ctx, out: Out, args: ExportArgs) -> CliResult<()> {
    use WalletKeyKind as Kind;
    let wallet = ctx.find_wallet(&args.wallet)?;
    // A wallet imported from a raw key has no phrase: its export is its key,
    // written as its network's wallets read it.
    let key = match args.key.as_deref() {
        Some("private-key") => Some(Kind::PrivateKey),
        Some("spend-key") => Some(Kind::MoneroSpendKey),
        Some("view-key") => Some(Kind::MoneroViewKey),
        Some("account-key") => Some(Kind::AccountPublicKey),
        Some(other) => return Err(CliError::usage(format!("unknown key kind {other:?}"))),
        None if matches!(wallet.signing, WalletSigning::PrivateKey { .. }) => {
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

    let password = args.password.for_wallet(&wallet)?;
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());

    if let Some(kind) = key {
        let export = ctx
            .rt
            .block_on(service.export_wallet_key(wallet.id, kind, password))?;
        let named = |wanted: Kind| (kind == wanted).then(|| export.value.clone());
        return out.report(&KeyReport {
            private_key: named(Kind::PrivateKey),
            spend_key: named(Kind::MoneroSpendKey),
            view_key: named(Kind::MoneroViewKey),
            account_key: named(Kind::AccountPublicKey),
            format: export.format,
            value: export.value,
        });
    }

    // Core answers why a phrase is not revealed; the words are the CLI's.
    let seed_phrase = match service.reveal_seed_phrase(wallet.id, password)? {
        SeedPhraseReveal::Phrase { phrase } => phrase,
        SeedPhraseReveal::NotStored => {
            return Err(CliError::rejected("this wallet stores no seed phrase"));
        }
        SeedPhraseReveal::PasswordRequired => {
            return Err(CliError::usage("this wallet needs its password"));
        }
        SeedPhraseReveal::IncorrectPassword => {
            return Err(CliError::rejected("incorrect password"));
        }
        SeedPhraseReveal::PasswordNotRequired => {
            return Err(CliError::rejected("this wallet has no password"));
        }
    };
    out.report(&PhraseReport { seed_phrase })
}

#[derive(Serialize)]
struct SignedReport {
    address: String,
    scheme: spectra_core::send::message::MessageScheme,
    message: String,
    signature: String,
}

impl Report for SignedReport {
    fn render(&self) {
        println!();
        out::wide_field(9, "address", &out::info(&self.address).to_string());
        out::wide_field(9, "scheme", &out::serde_name(&self.scheme));
        out::wide_field(9, "signature", &self.signature.bold().to_string());
    }
}

pub fn sign_message(ctx: &Ctx, out: Out, args: SignMessageArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let password = args.password.for_wallet(&wallet)?;
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());
    let signed = ctx
        .rt
        .block_on(service.sign_wallet_message(wallet.id, args.message, password))?;
    out.report(&SignedReport {
        address: signed.address,
        scheme: signed.scheme,
        message: signed.message,
        signature: signed.signature,
    })
}

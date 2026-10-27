//! A multisig account: its signers, and its spends — create one, read one
//! a signer or another coordinator wrote (joining its signatures to an
//! open session of the same transaction), sign it as one of the signers,
//! finalize and submit it. Core reviews every session against the
//! account's policy; this only forwards.

use clap::{Args, Subcommand};
use serde::Serialize;
use spectra_core::service::{MultisigAccount, MultisigSession, MultisigSigner, MultisigSpend};

use crate::ctx::{Ctx, PasswordArgs, WalletArg};
use crate::error::{CliError, CliResult};
use crate::out::{self, Done, Out, Report};

#[derive(Subcommand)]
pub enum MultisigCommand {
    /// The account's signers, their weights and thresholds.
    Account(WalletArg),
    /// Spend from a multisig account: a session for its signers to sign.
    Create(CreateArgs),
    /// Read a session a signer or coordinator wrote; one of a transaction
    /// an open session holds joins it.
    Import(ImportArgs),
    /// Sign a session as one of the account's signers.
    Sign(SignArgs),
    /// A session as core reviews it, with its data to hand on.
    Show { id: String },
    /// An account's sessions.
    List(WalletArg),
    /// The finished transaction, once enough signers signed.
    Finalize { id: String },
    /// Submit a session's transaction to its network.
    Submit(SubmitArgs),
    /// Forget a session.
    Discard {
        id: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct CreateArgs {
    /// Multisig wallet id, name or address.
    wallet: String,
    #[arg(long)]
    to: String,
    /// Amount in the network's coin.
    #[arg(long)]
    amount: String,
    /// A UTXO network's fee rate per virtual byte (default: the network's).
    #[arg(long)]
    fee_rate: Option<String>,
    /// How long signatures may be gathered, in seconds, where the network
    /// bounds it (default: the scheme's).
    #[arg(long)]
    expires_in: Option<u64>,
    #[command(flatten)]
    memo: super::send::MemoArgs,
}

#[derive(Args)]
pub struct ImportArgs {
    /// Multisig wallet id, name or address.
    wallet: String,
    /// The session's data: a PSBT (base64), or the network's own form.
    #[arg(long, required_unless_present = "file", conflicts_with = "file")]
    data: Option<String>,
    /// A file holding the session's data.
    #[arg(long)]
    file: Option<String>,
}

#[derive(Args)]
pub struct SignArgs {
    id: String,
    /// The review digest `multisig show` printed: what is signed for.
    #[arg(long)]
    review_digest: String,
    /// The wallet that signs, one of the account's signers; a UTXO
    /// account signs as the cosigner whose phrase was added to it.
    #[arg(long)]
    signer: Option<String>,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct SubmitArgs {
    id: String,
    /// The signer whose own transaction carries the session, where the
    /// network submits through one.
    #[arg(long)]
    executor: Option<String>,
    #[command(flatten)]
    password: PasswordArgs,
    #[arg(long)]
    yes: bool,
}

fn signer_line(signer: &MultisigSigner) -> String {
    format!(
        "{}  weight {}{}{}",
        signer.signer,
        signer.weight,
        if signer.signed { "  signed" } else { "" },
        signer
            .wallet_id
            .as_ref()
            .map_or(String::new(), |id| format!("  (wallet {id})"))
    )
}

/// A multisig account's permissions: who signs, with what weight, toward
/// which threshold.
#[derive(Serialize)]
struct AccountReport {
    account: MultisigAccount,
}

impl Report for AccountReport {
    fn render(&self) {
        let account = &self.account;
        println!();
        println!("  {}", account.address);
        for permission in &account.permissions {
            println!(
                "  {}  threshold {}{}",
                permission.name,
                permission.threshold,
                if permission.covers.is_empty() {
                    String::new()
                } else {
                    format!("  covers {}", permission.covers.join(", "))
                }
            );
            for signer in &permission.signers {
                println!("    {}", signer_line(signer));
            }
        }
        for warning in &account.warnings {
            println!("  {} {warning}", out::accent("!"));
        }
    }
}

/// A session as core reviews it against the account's policy.
#[derive(Serialize)]
struct SessionReport {
    session: MultisigSession,
}

impl Report for SessionReport {
    fn render(&self) {
        let session = &self.session;
        println!();
        println!("  {}  {}", session.id, session.transaction_id);
        for input in &session.inputs {
            println!(
                "  in   {}  {}  {}/{} signed",
                input.address, input.value, input.signatures, session.threshold
            );
        }
        for output in &session.outputs {
            println!(
                "  out  {}  {}{}{}",
                output.address,
                output.value,
                if output.is_change { "  (change)" } else { "" },
                output
                    .data
                    .as_ref()
                    .map_or(String::new(), |data| format!("  data {data}"))
            );
        }
        println!("  fee  {}", session.fee);
        if let Some(sequence) = &session.sequence {
            println!("  sequence  {sequence}");
        }
        if let Some(expires_at) = session.expires_at {
            println!("  expires at  {expires_at}");
        }
        println!(
            "  signed  {}/{}{}",
            session.signed_weight,
            session.threshold,
            if session.complete { "  complete" } else { "" }
        );
        for signer in &session.signers {
            println!("    {}", signer_line(signer));
        }
        if let Some(txid) = &session.submitted_txid {
            println!("  submitted  {txid}");
        }
        println!("  review digest  {}", session.review_digest);
        println!("  {}", session.data);
    }
}

#[derive(Serialize)]
struct SessionsReport {
    sessions: Vec<MultisigSession>,
}

impl Report for SessionsReport {
    fn render(&self) {
        println!();
        if self.sessions.is_empty() {
            println!("  {}", out::hint("no sessions"));
        }
        for session in &self.sessions {
            println!(
                "  {}  {}  {}/{} signed{}",
                session.id,
                session.transaction_id,
                session.signed_weight,
                session.threshold,
                session
                    .submitted_txid
                    .as_ref()
                    .map_or(String::new(), |_| "  submitted".into())
            );
        }
    }
}

/// The finished transaction, in its network's raw form.
#[derive(Serialize)]
struct RawReport {
    raw: String,
}

impl Report for RawReport {
    fn render(&self) {
        println!("{}", self.raw);
    }
}

pub fn run(ctx: &Ctx, out: Out, command: MultisigCommand) -> CliResult<()> {
    let service = ctx.service()?;
    let session = match command {
        MultisigCommand::Account(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let account = ctx.rt.block_on(service.multisig_account(wallet.id))?;
            return out.report(&AccountReport { account });
        }
        MultisigCommand::Create(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            ctx.rt.block_on(service.create_multisig(
                wallet.id,
                MultisigSpend {
                    to_address: args.to,
                    amount: args.amount,
                    fee_rate: args.fee_rate,
                    expires_in_secs: args.expires_in,
                    memo: args.memo.memo(),
                },
            ))?
        }
        MultisigCommand::Import(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let data = match (args.data, args.file) {
                (Some(data), _) => data,
                (None, Some(file)) => std::fs::read_to_string(&file)
                    .map_err(|e| CliError::usage(format!("{file}: {e}")))?,
                (None, None) => return Err(CliError::usage("give --data or --file")),
            };
            ctx.rt.block_on(service.import_multisig(wallet.id, data))?
        }
        MultisigCommand::Sign(args) => {
            let stored = ctx.rt.block_on(service.multisig_session(args.id.clone()))?;
            let signer = match &args.signer {
                Some(signer) => Some(ctx.find_wallet(signer)?),
                None => None,
            };
            let password = match &signer {
                Some(signer) => args.password.for_wallet(signer)?,
                None => args
                    .password
                    .for_wallet(&ctx.find_wallet(&stored.wallet_id)?)?,
            };
            let signer = signer.map(|signer| signer.id);
            ctx.rt
                .block_on(service.sign_multisig(args.id, args.review_digest, signer, password))?
        }
        MultisigCommand::Show { id } => ctx.rt.block_on(service.multisig_session(id))?,
        MultisigCommand::List(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let sessions = ctx.rt.block_on(service.multisig_sessions(wallet.id))?;
            return out.report(&SessionsReport { sessions });
        }
        MultisigCommand::Finalize { id } => {
            let raw = ctx.rt.block_on(service.finalize_multisig(id))?;
            return out.report(&RawReport { raw });
        }
        MultisigCommand::Submit(args) => {
            if !args.yes {
                return Err(CliError::usage("submit requires --yes"));
            }
            let (executor, password) = match &args.executor {
                Some(executor) => {
                    let executor = ctx.find_wallet(executor)?;
                    let password = args.password.for_wallet(&executor)?;
                    (Some(executor.id), password)
                }
                None => (None, None),
            };
            ctx.rt
                .block_on(service.submit_multisig(args.id, executor, password))?
        }
        MultisigCommand::Discard { id, yes } => {
            if !yes {
                return Err(CliError::usage("discard requires --yes"));
            }
            ctx.rt.block_on(service.discard_multisig(id.clone()))?;
            return out.report(&Done::new(format!("discarded {id}")));
        }
    };
    out.report(&SessionReport { session })
}

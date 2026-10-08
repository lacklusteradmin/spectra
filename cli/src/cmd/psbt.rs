//! A multisig wallet's PSBTs: create one, read one a cosigner or another
//! coordinator wrote (joining its signatures to an open session of the same
//! transaction), sign it as this wallet's cosigner, finalize and broadcast.
//! Core reviews every PSBT against the wallet's policy; this only forwards.

use clap::{Args, Subcommand};

use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::Out;

#[derive(Subcommand)]
pub enum PsbtCommand {
    /// Spend from a multisig wallet: a PSBT for its cosigners to sign.
    Create(CreateArgs),
    /// Read a PSBT (base64) for a multisig wallet; one of a transaction an
    /// open session holds joins it.
    Import(ImportArgs),
    /// Sign a session as the wallet's cosigner.
    Sign(SignArgs),
    /// A session as core reviews it, with its PSBT to hand on.
    Show { id: String },
    /// A wallet's sessions.
    List { wallet: String },
    /// The finished transaction, hex, once enough cosigners signed.
    Finalize { id: String },
    /// Finalize and broadcast a session's transaction.
    Broadcast {
        id: String,
        #[arg(long)]
        yes: bool,
    },
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
    #[arg(long)]
    from: String,
    #[arg(long)]
    to: String,
    /// Amount in BTC.
    #[arg(long)]
    amount: String,
    /// Fee rate in sat/vB (default: the network's).
    #[arg(long)]
    fee_rate: Option<String>,
}

#[derive(Args)]
pub struct ImportArgs {
    /// Multisig wallet id, name or address.
    #[arg(long)]
    wallet: String,
    /// The PSBT, base64.
    #[arg(long, required_unless_present = "file", conflicts_with = "file")]
    psbt: Option<String>,
    /// A file holding the PSBT, base64.
    #[arg(long)]
    file: Option<String>,
}

#[derive(Args)]
pub struct SignArgs {
    id: String,
    /// The review digest `psbt show` printed: what is signed for.
    #[arg(long)]
    review_digest: String,
    #[arg(long)]
    password_file: Option<String>,
    #[arg(long, default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
}

pub fn run(ctx: &Ctx, out: Out, command: PsbtCommand) -> CliResult<()> {
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());
    let session = match command {
        PsbtCommand::Create(args) => {
            let wallet = ctx.find_wallet(&args.from)?;
            ctx.rt
                .block_on(service.create_psbt(wallet.id, args.to, args.amount, args.fee_rate))?
        }
        PsbtCommand::Import(args) => {
            let wallet = ctx.find_wallet(&args.wallet)?;
            let psbt = match (args.psbt, args.file) {
                (Some(psbt), _) => psbt,
                (None, Some(file)) => std::fs::read_to_string(&file)
                    .map_err(|e| CliError::usage(format!("{file}: {e}")))?,
                (None, None) => return Err(CliError::usage("give --psbt or --file")),
            };
            ctx.rt.block_on(service.import_psbt(wallet.id, psbt))?
        }
        PsbtCommand::Sign(args) => {
            let stored = ctx.rt.block_on(service.psbt_session(args.id.clone()))?;
            let password = super::tx::signing_password(
                ctx,
                &stored.wallet_id,
                args.password_file,
                args.password_env,
            )?;
            ctx.rt
                .block_on(service.sign_psbt(args.id, args.review_digest, password))?
        }
        PsbtCommand::Show { id } => ctx.rt.block_on(service.psbt_session(id))?,
        PsbtCommand::List { wallet } => {
            let wallet = ctx.find_wallet(&wallet)?;
            let sessions = ctx.rt.block_on(service.psbt_sessions(wallet.id))?;
            out.text(|| {
                for session in &sessions {
                    println!(
                        "  {}  {}  {}/{} signed{}",
                        session.id,
                        session.txid,
                        session.signed_by.len(),
                        session.threshold,
                        session
                            .broadcast_txid
                            .as_ref()
                            .map_or(String::new(), |_| "  broadcast".into())
                    );
                }
            });
            out.emit(serde_json::json!({ "ok": true, "sessions": sessions }));
            return Ok(());
        }
        PsbtCommand::Finalize { id } => {
            let raw = ctx.rt.block_on(service.finalize_psbt(id))?;
            out.text(|| println!("{raw}"));
            out.emit(serde_json::json!({ "ok": true, "raw": raw }));
            return Ok(());
        }
        PsbtCommand::Broadcast { id, yes } => {
            if !yes {
                return Err(CliError::usage("broadcast requires --yes"));
            }
            ctx.rt.block_on(service.broadcast_psbt(id))?
        }
        PsbtCommand::Discard { id, yes } => {
            if !yes {
                return Err(CliError::usage("discard requires --yes"));
            }
            ctx.rt.block_on(service.discard_psbt(id))?;
            out.emit(serde_json::json!({ "ok": true }));
            return Ok(());
        }
    };
    out.text(|| {
        println!();
        println!("  {}  {}", session.id, session.txid);
        for input in &session.inputs {
            println!(
                "  in   {}  {} sat  {}/{} signed",
                input.address, input.value_sat, input.signatures, session.threshold
            );
        }
        for output in &session.outputs {
            println!(
                "  out  {}  {} sat{}",
                output.address,
                output.value_sat,
                if output.is_change { "  (change)" } else { "" }
            );
        }
        println!("  fee  {} sat", session.fee_sat);
        println!("  review digest  {}", session.review_digest);
        println!("  {}", session.psbt);
    });
    out.emit(serde_json::json!({ "ok": true, "session": session }));
    Ok(())
}

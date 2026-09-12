//! Funds finder: derive every (chain, path) a seed could have used, then look
//! for balances on them.
//!
//! Core derives the candidate matrix — four Bitcoin script types across three
//! accounts, and the equivalent for every other chain — and says so in its own
//! doc: "the balance of this address is checked separately by Swift". Which is
//! the half that had no second implementation.

use clap::Args;
use colored::Colorize as _;
use spectra_core::derivation::funds_finder::FundsFinderRequest;

use super::resolve_chain;
use crate::ctx::{Ctx, SecretSource};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Args)]
pub struct RescanArgs {
    /// Read the seed phrase from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    seed_file: Option<String>,
    /// Read the seed phrase from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_SEED")]
    seed_env: Option<String>,
    /// BIP-39 passphrase, if the seed uses one.
    #[arg(long)]
    passphrase: Option<String>,
    /// Only candidates on this chain.
    #[arg(long)]
    chain: Option<String>,
    /// List the candidates without checking any balances.
    #[arg(long)]
    dry_run: bool,
}

pub fn rescan(ctx: &Ctx, out: Out, args: RescanArgs) -> CliResult<()> {
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

    let service = ctx.service()?;
    let scan = service
        .begin_funds_scan(
            FundsFinderRequest {
                seed_phrase,
                passphrase: args.passphrase.clone(),
            },
            args.chain
                .as_ref()
                .map(|n| resolve_chain(n).map(|c| c.str_id().to_string()))
                .transpose()?,
        )
        .map_err(CliError::from)?;
    let candidates = scan.candidates();

    if args.dry_run {
        out.text(|| {
            println!();
            for candidate in &candidates {
                println!(
                    "  {}  {:<26} {}",
                    out::hint("·"),
                    candidate.path_label,
                    out::hint(&candidate.address),
                );
            }
            println!();
            println!(
                "  {} {}",
                out::accent(&candidates.len().to_string()).bold(),
                out::hint("candidates, none checked")
            );
        });
        out.emit(serde_json::json!({
            "ok": true,
            "checked": false,
            "candidates": candidates
                .iter()
                .map(|candidate| serde_json::json!({
                    "chain": candidate.chain_id,
                    "label": candidate.path_label,
                    "address": candidate.address,
                }))
                .collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    out.text(|| {
        println!(
            "  {} checking {} candidate addresses…",
            out::hint("→"),
            candidates.len()
        )
    });

    let mut funded = Vec::new();
    let mut unreachable = 0u32;
    loop {
        let batch = ctx.rt.block_on(scan.next_batch());
        for read in batch.reads {
            if read.error.is_some() {
                unreachable += 1;
            }
            if read.funded {
                let balance = read.balance.unwrap();
                funded.push(serde_json::json!({ "chain": read.candidate.chain_name, "label": read.candidate.path_label, "address": read.candidate.address, "amount": balance.amount_display }));
            }
        }
        if batch.complete {
            break;
        }
    }

    out.text(|| {
        println!();
        if funded.is_empty() {
            println!("  {}", out::hint("no funded addresses found"));
        }
        println!(
            "  {} of {} funded{}",
            out::accent(&funded.len().to_string()).bold(),
            candidates.len(),
            if unreachable > 0 {
                format!(", {unreachable} unreachable")
            } else {
                String::new()
            },
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "checked": true,
        "candidateCount": candidates.len(),
        "unreachable": unreachable,
        "funded": funded,
    }));
    Ok(())
}

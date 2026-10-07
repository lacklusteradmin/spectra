//! Funds finder: derive the addresses a seed could have used — every
//! registry derivation profile at the first accounts, or TON's wallet
//! versions — then look for balances on them.

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
    .resolve("seed phrase", "seed-file")?;

    let chain = args.chain.as_ref().map(|n| resolve_chain(n)).transpose()?;
    // On one network core judges the phrase in that network's formats.
    if chain.is_none() {
        crate::cmd::reject_bad_seed_phrase(None, &seed_phrase)?;
    }

    let service = ctx.service()?;
    let scan = service
        .begin_funds_scan(
            FundsFinderRequest {
                seed_phrase,
                passphrase: args.passphrase.clone(),
            },
            chain,
        )
        .map_err(CliError::from)?;
    let candidates = scan.candidates();
    // Named before anything is sent: whom the candidate addresses go to.
    let endpoints = ctx.rt.block_on(scan.endpoints());
    let endpoints_json: Vec<_> = endpoints
        .iter()
        .map(|endpoint| {
            serde_json::json!({
                "chain": endpoint.chain_id,
                "endpoint": endpoint.endpoint,
                "capabilities": endpoint.capabilities,
            })
        })
        .collect();

    if args.dry_run {
        out.text(|| {
            println!();
            for candidate in &candidates {
                println!(
                    "  {}  {:<16} {:<22} {}",
                    out::hint("·"),
                    candidate.chain_id.str_id(),
                    position(candidate),
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
            "endpoints": endpoints_json,
            "candidates": candidates
                .iter()
                .map(|candidate| serde_json::json!({
                    "chain": candidate.chain_id,
                    "profile": candidate.profile,
                    "account": candidate.account,
                    "path": candidate.derivation_path,
                    "tonWallet": candidate.ton_wallet_version,
                    "address": candidate.address,
                }))
                .collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    out.text(|| {
        for endpoint in &endpoints {
            println!(
                "  {} asks {}",
                out::hint(endpoint.chain_id.str_id()),
                endpoint.endpoint
            );
        }
        println!(
            "  {} checking {} candidate addresses…",
            out::hint("→"),
            candidates.len()
        )
    });

    let mut funded = Vec::new();
    // Every read, in the candidates' order: default profile first, accounts
    // ascending.
    let mut reads = Vec::new();
    let mut unreachable = 0u32;
    loop {
        let batch = ctx.rt.block_on(scan.next_batch());
        for read in batch.reads {
            if read.error.is_some() {
                unreachable += 1;
            }
            let row = serde_json::json!({
                "chain": read.candidate.chain_id,
                "profile": read.candidate.profile,
                "account": read.candidate.account,
                "path": read.candidate.derivation_path,
                "tonWallet": read.candidate.ton_wallet_version,
                "address": read.candidate.address,
                "amount": read.balance.as_ref().map(|balance| balance.amount_display.clone()),
                "funded": read.funded,
                "used": read.used,
                "error": read.error.as_ref().map(ToString::to_string),
            });
            if read.funded {
                funded.push(row.clone());
            }
            reads.push(row);
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
        "endpoints": endpoints_json,
        "funded": funded,
        "reads": reads,
    }));
    Ok(())
}

/// Where a candidate sits: its path, or on TON its wallet version.
fn position(candidate: &spectra_core::derivation::funds_finder::FundsFinderCandidate) -> String {
    match candidate.ton_wallet_version {
        Some(version) => serde_json::to_value(version)
            .ok()
            .and_then(|name| name.as_str().map(str::to_string))
            .unwrap_or_default(),
        None => candidate.derivation_path.clone(),
    }
}

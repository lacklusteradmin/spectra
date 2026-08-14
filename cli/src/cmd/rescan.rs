//! Funds finder: derive every (chain, path) a seed could have used, then look
//! for balances on them.
//!
//! Core derives the candidate matrix — four Bitcoin script types across three
//! accounts, and the equivalent for every other chain — and says so in its own
//! doc: "the balance of this address is checked separately by Swift". Which is
//! the half that had no second implementation.

use clap::Args;
use colored::Colorize as _;
use spectra_core::derivation::funds_finder::{core_generate_funds_finder_candidates, FundsFinderRequest};

use super::chain::{service_for_chain, BALANCE, RPC};
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

    if !spectra_core::service::validate_mnemonic(seed_phrase.clone()) {
        return Err(CliError::rejected("not a valid BIP-39 English mnemonic"));
    }

    let mut candidates = core_generate_funds_finder_candidates(FundsFinderRequest {
        seed_phrase,
        passphrase: args.passphrase.clone(),
    })
    .map_err(CliError::from)?;

    if let Some(name) = &args.chain {
        let chain = resolve_chain(name)?;
        candidates.retain(|candidate| candidate.chain_id == chain.str_id());
        if candidates.is_empty() {
            return Err(CliError::rejected(format!(
                "no candidates for {}",
                chain.chain_display_name()
            )));
        }
    }

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
    for candidate in &candidates {
        let Some(chain) = spectra_core::registry::Chain::from_str_id(&candidate.chain_id) else {
            continue;
        };
        let Ok(service) = service_for_chain(chain, BALANCE | RPC) else {
            unreachable += 1;
            continue;
        };
        let summary = ctx.rt.block_on(service.fetch_native_balance_summary(
            candidate.chain_id.clone(),
            candidate.address.clone(),
        ));
        let Ok(summary) = summary else {
            unreachable += 1;
            continue;
        };
        let amount: f64 = summary.amount_display.parse().unwrap_or(0.0);
        if amount <= 0.0 {
            continue;
        }
        out.text(|| {
            println!(
                "  {}  {:<26} {} {}",
                out::tint("●", chain.chain_display_name()).bold(),
                candidate.path_label,
                summary.amount_display.bold(),
                out::tint(chain.coin_symbol(), chain.chain_display_name()),
            );
            println!("     {}", out::hint(&candidate.address));
        });
        funded.push(serde_json::json!({
            "chain": chain.chain_display_name(),
            "label": candidate.path_label,
            "address": candidate.address,
            "amount": summary.amount_display,
            "symbol": chain.coin_symbol(),
        }));
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

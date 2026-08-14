//! Diagnostics: core's own self-tests, and the exportable bundle.
//!
//! `self_tests.rs` is 605 lines of derivation and address checks that only the
//! iOS diagnostics screen has ever run. They need no network and no device, so
//! there was never a reason the CLI could not run them — only that nothing
//! asked.

use clap::{Args, Subcommand};
use colored::Colorize as _;

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum DiagnosticsCommand {
    /// Run core's self-tests for one chain, or all of them.
    SelfTest(SelfTestArgs),
    /// The diagnostics document core builds for a chain.
    Show(ShowArgs),
}

#[derive(Args)]
pub struct SelfTestArgs {
    /// Chain display name, registry id or symbol. Omit to run every chain.
    #[arg(long)]
    chain: Option<String>,
}

#[derive(Args)]
pub struct ShowArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
}

pub fn run(ctx: &Ctx, out: Out, command: DiagnosticsCommand) -> CliResult<()> {
    match command {
        DiagnosticsCommand::SelfTest(args) => self_test(out, args),
        DiagnosticsCommand::Show(args) => show(ctx, out, args),
    }
}

fn self_test(out: Out, args: SelfTestArgs) -> CliResult<()> {
    // Core keys self-tests by *display name*, not registry id — `chain_key` in
    // `ChainSpec` is "Bitcoin Cash", not "bitcoin-cash". Passing the id got
    // "no self-tests" for every chain, which reads as "this chain is not
    // covered" rather than "you asked with the wrong key".
    let by_chain = match &args.chain {
        Some(name) => {
            let chain = resolve_chain(name)?;
            let results =
                spectra_core::diagnostics::self_tests::self_tests_run_chain(
                    chain.chain_display_name().to_string(),
                );
            if results.is_empty() {
                return Err(CliError::rejected(format!(
                    "{} has no self-tests",
                    chain.chain_display_name()
                )));
            }
            std::collections::HashMap::from([(chain.chain_display_name().to_string(), results)])
        }
        None => spectra_core::diagnostics::self_tests::self_tests_run_all(),
    };

    let mut chains: Vec<_> = by_chain.into_iter().collect();
    chains.sort_by(|a, b| a.0.cmp(&b.0));

    let total: usize = chains.iter().map(|(_, results)| results.len()).sum();
    let failed: usize = chains
        .iter()
        .map(|(_, results)| results.iter().filter(|result| !result.passed).count())
        .sum();

    out.text(|| {
        println!();
        for (chain_id, results) in &chains {
            let chain_failed = results.iter().filter(|result| !result.passed).count();
            println!(
                "  {}  {:<22} {}",
                if chain_failed == 0 {
                    out::ok_mark()
                } else {
                    out::fail_mark()
                },
                chain_id.bold(),
                out::hint(&format!("{} checks", results.len())),
            );
            for result in results.iter().filter(|result| !result.passed) {
                println!("     {} {}", out::fail_mark(), result.name);
            }
        }
        println!();
        println!(
            "  {} {}",
            out::accent(&format!("{}/{}", total - failed, total)).bold(),
            out::hint("checks passed"),
        );
    });
    out.emit(serde_json::json!({
        "ok": failed == 0,
        "total": total,
        "failed": failed,
        "chains": chains
            .iter()
            .map(|(chain_id, results)| serde_json::json!({
                "chain": chain_id,
                "checks": results
                    .iter()
                    .map(|result| serde_json::json!({
                        "name": result.name,
                        "passed": result.passed,
                    }))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    }));

    if failed > 0 {
        return Err(CliError::reported(format!(
            "{failed} of {total} checks failed"
        )));
    }
    Ok(())
}

fn show(ctx: &Ctx, out: Out, args: ShowArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let _ = ctx;
    let json = spectra_core::diagnostics::core_diagnostics_json(
        chain.chain_display_name().to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
    )
    .ok_or_else(|| {
        CliError::failure(format!(
            "core built no diagnostics document for {}",
            chain.chain_display_name()
        ))
    })?;

    out.text(|| println!("{json}"));
    out.emit(serde_json::json!({ "ok": true, "chain": chain.chain_display_name(), "document": json }));
    Ok(())
}

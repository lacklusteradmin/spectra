//! Funds finder: derive the addresses a seed could have used — every
//! registry derivation profile at the first accounts, or TON's wallet
//! versions — then look for balances on them.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::api::endpoint::EndpointCapability;
use spectra_core::catalog::chains::DerivationProfile;
use spectra_core::derivation::funds_finder::{FundsFinderCandidate, FundsFinderRequest};
use spectra_core::derivation::ton::TonWalletVersion;
use spectra_core::registry::Chain;

use super::resolve_chain;
use crate::ctx::{Ctx, SecretSource};
use crate::error::CliResult;
use crate::out::{self, Out, Report};

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

/// Whom the candidate addresses are sent to.
#[derive(Serialize)]
struct EndpointRow {
    chain: Chain,
    endpoint: String,
    capabilities: Vec<EndpointCapability>,
}

/// Where a candidate sits: its network, profile, account and path, or on
/// TON its wallet version.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateRow {
    chain: Chain,
    profile: Option<DerivationProfile>,
    account: u32,
    path: String,
    ton_wallet: Option<TonWalletVersion>,
    address: String,
}

impl From<&FundsFinderCandidate> for CandidateRow {
    fn from(candidate: &FundsFinderCandidate) -> Self {
        Self {
            chain: candidate.chain_id,
            profile: candidate.profile,
            account: candidate.account,
            path: candidate.derivation_path.clone(),
            ton_wallet: candidate.ton_wallet_version,
            address: candidate.address.clone(),
        }
    }
}

impl CandidateRow {
    fn position(&self) -> String {
        match &self.ton_wallet {
            Some(version) => out::serde_name(version),
            None => self.path.clone(),
        }
    }
}

/// The addresses a scan would check, none of them checked.
#[derive(Serialize)]
struct CandidatesReport {
    checked: bool,
    endpoints: Vec<EndpointRow>,
    candidates: Vec<CandidateRow>,
}

impl Report for CandidatesReport {
    fn render(&self) {
        println!();
        for candidate in &self.candidates {
            println!(
                "  {}  {:<16} {:<22} {}",
                out::hint("·"),
                candidate.chain.str_id(),
                candidate.position(),
                out::hint(&candidate.address),
            );
        }
        println!();
        println!(
            "  {}, none checked",
            out::count(self.candidates.len(), "candidate")
        );
    }
}

/// One candidate address as the network answered for it.
#[derive(Serialize)]
struct ReadRow {
    #[serde(flatten)]
    candidate: CandidateRow,
    amount: Option<String>,
    funded: bool,
    used: bool,
    error: Option<String>,
}

/// Every candidate checked, in the scan's order: default profile first,
/// accounts ascending.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanReport {
    checked: bool,
    candidate_count: usize,
    unreachable: usize,
    endpoints: Vec<EndpointRow>,
    reads: Vec<ReadRow>,
}

impl Report for ScanReport {
    fn render(&self) {
        println!();
        let funded: Vec<&ReadRow> = self.reads.iter().filter(|read| read.funded).collect();
        if funded.is_empty() {
            println!("  {}", out::hint("no funded addresses found"));
        }
        for read in &funded {
            println!(
                "  {}  {:<16} {:<22} {}  {}",
                out::ok_mark(),
                read.candidate.chain.str_id(),
                read.candidate.position(),
                read.candidate.address,
                read.amount.as_deref().unwrap_or("").bold(),
            );
        }
        println!(
            "  {} of {} funded{}",
            out::accent(&funded.len().to_string()).bold(),
            self.candidate_count,
            if self.unreachable > 0 {
                format!(", {} unreachable", self.unreachable)
            } else {
                String::new()
            },
        );
    }
}

pub fn rescan(ctx: &Ctx, out: Out, args: RescanArgs) -> CliResult<()> {
    let seed_phrase = SecretSource {
        file: args.seed_file.clone(),
        env: args.seed_env.clone(),
    }
    .resolve("seed phrase", "seed-file")?;

    let chain = args.chain.as_deref().map(resolve_chain).transpose()?;
    // On one network core judges the phrase in that network's formats.
    if chain.is_none() {
        crate::cmd::reject_bad_seed_phrase(None, &seed_phrase)?;
    }

    let service = ctx.service()?;
    let scan = service.begin_funds_scan(
        FundsFinderRequest {
            seed_phrase,
            passphrase: args.passphrase.clone(),
        },
        chain,
    )?;
    let candidates = scan.candidates();
    // Named before anything is sent: whom the candidate addresses go to.
    let endpoints: Vec<EndpointRow> = ctx
        .rt
        .block_on(scan.endpoints())
        .into_iter()
        .map(|endpoint| EndpointRow {
            chain: endpoint.chain_id,
            endpoint: endpoint.endpoint,
            capabilities: endpoint.capabilities,
        })
        .collect();

    if args.dry_run {
        return out.report(&CandidatesReport {
            checked: false,
            endpoints,
            candidates: candidates.iter().map(CandidateRow::from).collect(),
        });
    }

    for endpoint in &endpoints {
        out::progress(&format!(
            "{} asks {}",
            endpoint.chain.str_id(),
            endpoint.endpoint
        ));
    }
    out::progress(&format!(
        "checking {} candidate addresses…",
        candidates.len()
    ));
    let mut reads = Vec::new();
    loop {
        let batch = ctx.rt.block_on(scan.next_batch());
        reads.extend(batch.reads.into_iter().map(|read| ReadRow {
            candidate: CandidateRow::from(&read.candidate),
            amount: read.balance.map(|balance| balance.amount_display),
            funded: read.funded,
            used: read.used,
            error: read.error.as_ref().map(ToString::to_string),
        }));
        if batch.complete {
            break;
        }
    }
    out.report(&ScanReport {
        checked: true,
        candidate_count: candidates.len(),
        unreachable: reads.iter().filter(|read| read.error.is_some()).count(),
        endpoints,
        reads,
    })
}

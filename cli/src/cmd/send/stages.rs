//! The staged flow: a transaction is built and stored, reviewed, signed to
//! its review digest, then broadcast to endpoints the caller names. Each
//! stage is its own process; the store carries the transaction between them.

use clap::Args;
use serde::Serialize;
use spectra_core::send::SendExecutionRequest;
use spectra_core::send::ethereum::{EvmCustomFeeConfiguration, EvmSendOverridesInput};
use spectra_core::service::OwnedReplacementDraft;

use super::{ArtifactReport, ArtifactsReport, BroadcastReport, MemoArgs};
use crate::cmd::resolve_chain;
use crate::ctx::{Ctx, PasswordArgs};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Args)]
pub struct BuildArgs {
    /// Wallet id, name or address.
    wallet: String,
    #[arg(long)]
    to: String,
    #[arg(long)]
    amount: String,
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long)]
    contract: Option<String>,
    #[arg(long)]
    decimals: Option<u32>,
    #[arg(long)]
    nonce: Option<i64>,
    #[arg(long)]
    gas_limit: Option<i64>,
    #[arg(long, requires = "priority_fee_gwei")]
    max_fee_gwei: Option<String>,
    #[arg(long, requires = "max_fee_gwei")]
    priority_fee_gwei: Option<String>,
    #[command(flatten)]
    memo: MemoArgs,
}

#[derive(Args)]
pub struct SignArgs {
    transaction_id: String,
    #[arg(long)]
    review_digest: String,
    #[arg(long)]
    endpoint: Option<String>,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct BroadcastArgs {
    transaction_id: String,
    #[arg(long, required = true)]
    endpoint: Vec<String>,
    #[arg(long)]
    yes: bool,
}

pub fn build(ctx: &Ctx, out: Out, args: BuildArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = wallet.chain_id;
    let service = ctx.service_with_endpoints(chain, args.endpoint.into_iter().collect())?;
    let overrides = (args.nonce.is_some()
        || args.gas_limit.is_some()
        || args.max_fee_gwei.is_some())
    .then_some(EvmSendOverridesInput {
        nonce: args.nonce,
        gas_limit: args.gas_limit,
        custom_fees: args.max_fee_gwei.zip(args.priority_fee_gwei).map(
            |(max_fee_per_gas_gwei, max_priority_fee_per_gas_gwei)| EvmCustomFeeConfiguration {
                max_fee_per_gas_gwei,
                max_priority_fee_per_gas_gwei,
            },
        ),
        ..Default::default()
    });
    let artifact = ctx.rt.block_on(service.build_send(SendExecutionRequest {
        token_standard: None,
        chain_id: chain,
        wallet_id: wallet.id,
        password: None,
        to_address: args.to,
        amount_str: args.amount,
        contract_address: args.contract,
        token_decimals: args.decimals,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: overrides,
        memo: args.memo.memo(),
    }))?;
    out.report(&ArtifactReport { artifact })
}

pub fn list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let artifacts = ctx.rt.block_on(ctx.service()?.list_sends())?;
    out.report(&ArtifactsReport { artifacts })
}

pub fn inspect(ctx: &Ctx, out: Out, transaction_id: String) -> CliResult<()> {
    let artifact = ctx
        .rt
        .block_on(ctx.service()?.inspect_send(transaction_id))?;
    out.report(&ArtifactReport { artifact })
}

pub fn sign(ctx: &Ctx, out: Out, args: SignArgs) -> CliResult<()> {
    let stored = ctx
        .rt
        .block_on(ctx.service()?.inspect_send(args.transaction_id.clone()))?;
    let password = args
        .password
        .for_wallet(&ctx.find_wallet(&stored.wallet_id)?)?;
    let service =
        ctx.service_with_endpoints(stored.chain_id, args.endpoint.into_iter().collect())?;
    let artifact =
        ctx.rt
            .block_on(service.sign_send(args.transaction_id, args.review_digest, password))?;
    out.report(&ArtifactReport { artifact })
}

pub fn broadcast(ctx: &Ctx, out: Out, args: BroadcastArgs) -> CliResult<()> {
    if !args.yes {
        return Err(CliError::usage("broadcast-signed requires --yes"));
    }
    let stored = ctx
        .rt
        .block_on(ctx.service()?.inspect_send(args.transaction_id.clone()))?;
    let service = ctx.service_with_endpoints(stored.chain_id, args.endpoint.clone())?;
    let artifact = ctx
        .rt
        .block_on(service.broadcast_send(args.transaction_id, args.endpoint))?;
    out.report(&ArtifactReport { artifact })
}

pub fn rebroadcast(ctx: &Ctx, out: Out, transaction_id: String, yes: bool) -> CliResult<()> {
    if !yes {
        return Err(CliError::usage("rebroadcast requires --yes"));
    }
    let transaction_hash = ctx
        .rt
        .block_on(ctx.service()?.rebroadcast_transaction(transaction_id))?;
    out.report(&BroadcastReport { transaction_hash })
}

/// The fields a replacement or cancellation of a pending EVM send would
/// carry: the same nonce, at fees core raised past the original's.
#[derive(Serialize)]
struct ReplacementReport {
    draft: OwnedReplacementDraft,
}

impl Report for ReplacementReport {
    fn render(&self) {
        let draft = &self.draft;
        println!();
        out::wide_field(12, "to", &draft.destination);
        out::wide_field(12, "amount", &draft.amount);
        out::wide_field(12, "nonce", &draft.nonce.to_string());
        out::wide_field(12, "max fee", &format!("{} gwei", draft.max_fee_gwei));
        out::wide_field(
            12,
            "priority fee",
            &format!("{} gwei", draft.priority_fee_gwei),
        );
    }
}

pub fn replacement(ctx: &Ctx, out: Out, transaction_id: String, cancel: bool) -> CliResult<()> {
    let draft = ctx
        .rt
        .block_on(ctx.service()?.replacement_draft(transaction_id, cancel))?;
    out.report(&ReplacementReport { draft })
}

/// The endpoints a send on a network goes to, in the order it tries them.
#[derive(Serialize)]
struct ConfiguredEndpointsReport {
    chain: &'static str,
    endpoints: Vec<String>,
}

impl Report for ConfiguredEndpointsReport {
    fn render(&self) {
        println!();
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            println!("  {}  {endpoint}", out::hint(&format!("{:>2}", index + 1)));
        }
    }
}

pub fn configured_endpoints(ctx: &Ctx, out: Out, chain: &str) -> CliResult<()> {
    let chain = resolve_chain(chain)?;
    let endpoints = ctx.rt.block_on(ctx.service()?.send_endpoints(chain))?;
    out.report(&ConfiguredEndpointsReport {
        chain: chain.str_id(),
        endpoints,
    })
}

//! Wallet-owned staking positions and durable preparation; send stages sign
//! and broadcast the reviewed artifact.

use clap::{Args, Subcommand, ValueEnum};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::api::endpoint::EndpointCapability;
use spectra_core::catalog::chains::StakingChainEntry;
use spectra_core::registry::Chain;
use spectra_core::staking::StakingPosition;

use super::resolve_chain;
use super::send::ArtifactReport;
use crate::ctx::{Ctx, PasswordArgs};
use crate::error::CliResult;
use crate::out::{self, Out, Report};

#[derive(Subcommand)]
pub enum StakingCommand {
    /// Staking network descriptions, minimum stake and unbonding periods. Offline.
    Chains,
    /// Available validators: name, identifier, commission and minimum delegation.
    Validators(ValidatorsArgs),
    /// Effective configured endpoints, without making network requests.
    Endpoints(ValidatorsArgs),
    /// Owned on-chain positions, including unlocking and withdrawable amounts.
    Positions(PositionsArgs),
    /// Prepare a staking operation; use send sign and broadcast-signed next.
    Build(BuildArgs),
    /// Read exact execution receipts. ICP authorizes fresh read_state envelopes.
    Recheck(RecheckArgs),
    /// Verify an interrupted ICP stake and prepare unfunded recovery for review.
    Repair(RecheckArgs),
    /// Broadcast endpoints that accept this network's staking protocol.
    BroadcastEndpoints(ValidatorsArgs),
}

#[derive(Args)]
pub struct WalletArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The network to stake on; defaults to the wallet's.
    #[arg(long)]
    chain: Option<String>,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct PositionsArgs {
    #[command(flatten)]
    wallet: WalletArgs,
    /// Additional known delegation pools; ownership is checked on chain.
    #[arg(long)]
    pool: Vec<String>,
}

#[derive(Args)]
pub struct BuildArgs {
    #[command(flatten)]
    wallet: WalletArgs,
    #[arg(long, value_enum)]
    action: ActionArg,
    #[arg(long)]
    validator: Option<String>,
    #[arg(long)]
    position: Option<String>,
    #[arg(long)]
    amount: Option<String>,
    /// Explicit ICP neuron dissolve delay, in seconds.
    #[arg(long)]
    lockup_seconds: Option<u64>,
}

/// The actions `staking build` prepares, one value each; clap refuses any
/// other spelling before a wallet is read.
#[derive(Clone, Copy, ValueEnum)]
enum ActionArg {
    Stake,
    Unstake,
    Withdraw,
    ClaimRewards,
}

impl From<ActionArg> for spectra_core::staking::StakingAction {
    fn from(action: ActionArg) -> Self {
        match action {
            ActionArg::Stake => Self::Stake,
            ActionArg::Unstake => Self::Unstake,
            ActionArg::Withdraw => Self::Withdraw,
            ActionArg::ClaimRewards => Self::ClaimRewards,
        }
    }
}

#[derive(Args)]
pub struct RecheckArgs {
    /// The staged staking transaction's id.
    #[arg(long)]
    id: String,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct ValidatorsArgs {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
    /// Most validators to show.
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

pub fn run(ctx: &Ctx, out: Out, command: StakingCommand) -> CliResult<()> {
    match command {
        StakingCommand::Chains => out.report(&ChainsReport {
            chains: spectra_core::catalog::chains::list_staking_chains(),
        }),
        StakingCommand::Validators(args) => validators(ctx, out, args),
        StakingCommand::Endpoints(args) => {
            let chain = resolve_chain(&args.chain)?;
            let config = ctx.rt.block_on(ctx.service()?.staking_endpoints(chain))?;
            out.report(&EndpointsReport {
                chain: config.chain_id,
                endpoints: config.endpoints,
                capabilities: config.capabilities,
            })
        }
        StakingCommand::BroadcastEndpoints(args) => {
            let chain = resolve_chain(&args.chain)?;
            let endpoints = ctx
                .rt
                .block_on(ctx.service()?.staking_broadcast_endpoints(chain))?;
            out.report(&EndpointsReport {
                chain,
                endpoints,
                capabilities: vec![EndpointCapability::Broadcast],
            })
        }
        StakingCommand::Positions(args) => positions(ctx, out, args),
        StakingCommand::Build(args) => build(ctx, out, args),
        StakingCommand::Recheck(args) => recheck(ctx, out, args, false),
        StakingCommand::Repair(args) => recheck(ctx, out, args, true),
    }
}

/// The networks the staking tab offers, and what staking on each asks.
#[derive(Serialize)]
struct ChainsReport {
    chains: Vec<StakingChainEntry>,
}

impl Report for ChainsReport {
    fn render(&self) {
        println!();
        for entry in &self.chains {
            println!(
                "  {}  {:<20} {}",
                out::tint("●", entry.chain).bold(),
                entry.chain.chain_display_name(),
                out::hint(&entry.short_mechanic),
            );
            println!(
                "     minimum {} · unbonding {}",
                entry.minimum_stake, entry.unbonding_period
            );
        }
    }
}

/// The endpoints staking on a network reads from or broadcasts to.
#[derive(Serialize)]
struct EndpointsReport {
    chain: Chain,
    endpoints: Vec<String>,
    #[serde(skip)]
    capabilities: Vec<EndpointCapability>,
}

impl Report for EndpointsReport {
    fn render(&self) {
        println!();
        let capabilities: Vec<&str> = self.capabilities.iter().map(|c| c.as_str()).collect();
        println!(
            "  {}  {}",
            out::chain(self.chain).bold(),
            out::hint(&capabilities.join(" · "))
        );
        for endpoint in &self.endpoints {
            println!("  {endpoint}");
        }
    }
}

/// The positions a wallet owns, with what each holds and allows.
#[derive(Serialize)]
struct PositionsReport {
    chain: Chain,
    positions: Vec<StakingPosition>,
}

impl Report for PositionsReport {
    fn render(&self) {
        let chain = self.chain;
        let amount = |units: &str| {
            spectra_core::amount::decimal::format_native_amount(chain, units.to_string())
                .unwrap_or_else(|| "—".into())
        };
        let symbol = chain.coin_symbol();
        println!();
        if self.positions.is_empty() {
            println!("  {}", out::hint("no positions"));
        }
        for position in &self.positions {
            println!(
                "  {}  {}  {}",
                out::serde_name(&position.status).bold(),
                position.validator_identifier,
                out::hint(&position.id)
            );
            println!(
                "     staked {} {symbol} · unlocking {} · withdrawable {}",
                amount(&position.staked_amount_smallest_unit),
                amount(&position.unbonding_amount_smallest_unit),
                amount(&position.withdrawable_amount_smallest_unit),
            );
            if !position.available_actions.is_empty() {
                let actions: Vec<String> = position
                    .available_actions
                    .iter()
                    .map(out::serde_name)
                    .collect();
                println!("     {}", out::hint(&actions.join(" · ")));
            }
        }
    }
}

fn positions(ctx: &Ctx, out: Out, args: PositionsArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet.wallet)?;
    let chain = args
        .wallet
        .chain
        .as_deref()
        .map(resolve_chain)
        .transpose()?
        .unwrap_or(wallet.chain_id);
    let password = if chain == Chain::Icp {
        args.wallet.password.for_wallet(&wallet)?
    } else {
        None
    };
    let positions = ctx.rt.block_on(
        ctx.service()?
            .fetch_staking_positions(wallet.id, chain, args.pool, password),
    )?;
    out.report(&PositionsReport { chain, positions })
}

fn build(ctx: &Ctx, out: Out, args: BuildArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet.wallet)?;
    let chain = args
        .wallet
        .chain
        .as_deref()
        .map(resolve_chain)
        .transpose()?
        .unwrap_or(wallet.chain_id);
    let password = args.wallet.password.for_wallet(&wallet)?;
    let request = spectra_core::staking::StakingRequest {
        wallet_id: wallet.id,
        chain_id: chain,
        action: args.action.into(),
        validator_id: args.validator,
        position_id: args.position,
        amount: args.amount,
        lockup_seconds: args.lockup_seconds,
    };
    let artifact = ctx
        .rt
        .block_on(ctx.service()?.build_staking(request, password))?;
    out.report(&ArtifactReport { artifact })
}

fn recheck(ctx: &Ctx, out: Out, args: RecheckArgs, repair: bool) -> CliResult<()> {
    let service = ctx.service()?;
    let stored = ctx.rt.block_on(service.inspect_send(args.id.clone()))?;
    let password = if stored.chain_id == Chain::Icp {
        args.password
            .for_wallet(&ctx.find_wallet(&stored.wallet_id)?)?
    } else {
        None
    };
    let artifact = if repair {
        ctx.rt.block_on(service.repair_staking(args.id, password))?
    } else {
        ctx.rt
            .block_on(service.recheck_staking(args.id, password))?
    };
    out.report(&ArtifactReport { artifact })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidatorRow {
    identifier: String,
    name: String,
    commission: Option<f64>,
    min_delegation_smallest_unit: Option<String>,
}

#[derive(Serialize)]
struct ValidatorsReport {
    chain: Chain,
    validators: Vec<ValidatorRow>,
    #[serde(skip)]
    available: usize,
}

impl Report for ValidatorsReport {
    fn render(&self) {
        println!();
        if self.validators.is_empty() {
            println!("  {}", out::hint("no validator data available"));
            return;
        }
        for validator in &self.validators {
            println!(
                "  {}  {}",
                out::tint("●", self.chain).bold(),
                validator.name
            );
            println!("     {}", out::hint(&validator.identifier));
        }
        println!();
        println!(
            "  {} of {}",
            out::accent(&self.validators.len().to_string()).bold(),
            out::count(self.available, "validator")
        );
    }
}

fn validators(ctx: &Ctx, out: Out, args: ValidatorsArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let validators = ctx
        .rt
        .block_on(ctx.service()?.fetch_staking_validators(chain))?;
    out.report(&ValidatorsReport {
        chain,
        available: validators.len(),
        validators: validators
            .into_iter()
            .take(args.limit)
            .map(|validator| ValidatorRow {
                identifier: validator.identifier,
                name: validator.display_name,
                commission: validator.commission,
                min_delegation_smallest_unit: validator.min_delegation_smallest_unit,
            })
            .collect(),
    })
}

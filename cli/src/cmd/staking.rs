//! Wallet-owned staking positions and durable preparation; send stages sign
//! and broadcast the reviewed artifact.

use clap::{Args, Subcommand};
use colored::Colorize as _;

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::CliResult;
use crate::out::{self, Out};

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
    #[arg(long)]
    from: String,
    #[arg(long)]
    chain: Option<String>,
    #[arg(long, conflicts_with = "password_env")]
    password_file: Option<String>,
    #[arg(long, conflicts_with = "password_file")]
    password_env: Option<String>,
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
    #[arg(long,value_parser=["stake","unstake","withdraw","claim-rewards"])]
    action: String,
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
#[derive(Args)]
pub struct RecheckArgs {
    #[arg(long)]
    id: String,
    #[arg(long, conflicts_with = "password_env")]
    password_file: Option<String>,
    #[arg(long, conflicts_with = "password_file")]
    password_env: Option<String>,
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
    let repair = matches!(&command, StakingCommand::Repair(_));
    match command {
        StakingCommand::Chains => {
            let chains = spectra_core::chains::list_staking_chains();
            out.text(|| {
                for entry in &chains {
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
            });
            out.emit(serde_json::json!({"ok": true, "chains": chains}));
            Ok(())
        }
        StakingCommand::Validators(args) => validators(ctx, out, args),
        StakingCommand::Endpoints(args) => {
            let chain = resolve_chain(&args.chain)?;
            let config = ctx.rt.block_on(ctx.service()?.staking_endpoints(chain))?;
            out.emit(
                serde_json::json!({"ok":true,"chain":config.chain_id,"endpoints":config.endpoints}),
            );
            Ok(())
        }
        StakingCommand::BroadcastEndpoints(args) => {
            let chain = resolve_chain(&args.chain)?;
            let endpoints = ctx
                .rt
                .block_on(ctx.service()?.staking_broadcast_endpoints(chain))?;
            out.emit(serde_json::json!({"ok":true,"chain":chain,"endpoints":endpoints}));
            Ok(())
        }
        StakingCommand::Positions(args) => {
            let wallet = ctx.find_wallet(&args.wallet.from)?;
            let chain = args
                .wallet
                .chain
                .as_deref()
                .map(resolve_chain)
                .transpose()?
                .unwrap_or(wallet.chain_id);
            let password = if chain == spectra_core::registry::Chain::Icp {
                super::tx::signing_password(
                    ctx,
                    &wallet.id,
                    args.wallet.password_file,
                    args.wallet.password_env,
                )?
            } else {
                None
            };
            let positions = ctx.rt.block_on(
                ctx.service()?
                    .fetch_staking_positions(wallet.id, chain, args.pool, password),
            )?;
            out.text(|| {
                for p in &positions {
                    println!(
                        "  {} {:?} stake {} · unlocking {} · withdrawable {} {}",
                        p.id,
                        p.status,
                        spectra_core::staking::format_staking_amount(
                            chain,
                            p.staked_amount_smallest_unit.clone()
                        )
                        .unwrap_or_else(|| "—".into()),
                        spectra_core::staking::format_staking_amount(
                            chain,
                            p.unbonding_amount_smallest_unit.clone()
                        )
                        .unwrap_or_else(|| "—".into()),
                        spectra_core::staking::format_staking_amount(
                            chain,
                            p.withdrawable_amount_smallest_unit.clone()
                        )
                        .unwrap_or_else(|| "—".into()),
                        chain.coin_symbol()
                    );
                }
            });
            out.emit(serde_json::json!({"ok":true,"chain":chain,"positions":positions}));
            Ok(())
        }
        StakingCommand::Build(args) => {
            let wallet = ctx.find_wallet(&args.wallet.from)?;
            let chain = args
                .wallet
                .chain
                .as_deref()
                .map(resolve_chain)
                .transpose()?
                .unwrap_or(wallet.chain_id);
            let password = super::tx::signing_password(
                ctx,
                &wallet.id,
                args.wallet.password_file,
                args.wallet.password_env,
            )?;
            let action = match args.action.as_str() {
                "stake" => spectra_core::staking::StakingAction::Stake,
                "unstake" => spectra_core::staking::StakingAction::Unstake,
                "withdraw" => spectra_core::staking::StakingAction::Withdraw,
                _ => spectra_core::staking::StakingAction::ClaimRewards,
            };
            let request = spectra_core::staking::StakingRequest {
                wallet_id: wallet.id,
                chain_id: chain,
                action,
                validator_id: args.validator,
                position_id: args.position,
                amount: args.amount,
                lockup_seconds: args.lockup_seconds,
            };
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.build_staking(request, password))?;
            out.text(|| {
                println!(
                    "  Prepared {:?} {} {} · artifact {}",
                    artifact.staking.as_ref().map(|r| r.action),
                    artifact.amount,
                    artifact.symbol,
                    artifact.id
                )
            });
            out.emit(serde_json::json!({"ok":true,"artifact":artifact}));
            Ok(())
        }
        StakingCommand::Recheck(args) | StakingCommand::Repair(args) => {
            let service = ctx.service()?;
            let artifact = ctx.rt.block_on(service.inspect_send(args.id.clone()))?;
            let password = if artifact.chain_id == spectra_core::registry::Chain::Icp {
                super::tx::signing_password(
                    ctx,
                    &artifact.wallet_id,
                    args.password_file,
                    args.password_env,
                )?
            } else {
                None
            };
            let artifact = if repair {
                ctx.rt.block_on(service.repair_staking(args.id, password))?
            } else {
                ctx.rt
                    .block_on(service.recheck_staking(args.id, password))?
            };
            out.emit(serde_json::json!({"ok":true,"artifact":artifact}));
            Ok(())
        }
    }
}

fn validators(ctx: &Ctx, out: Out, args: ValidatorsArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let service = ctx.service()?;
    let validators = ctx.rt.block_on(service.fetch_staking_validators(chain))?;

    out.text(|| {
        println!();
        if validators.is_empty() {
            println!("  {}", out::hint("no validator data available"));
            return;
        }
        for validator in validators.iter().take(args.limit) {
            println!(
                "  {}  {}",
                out::tint("●", chain).bold(),
                validator.display_name
            );
            println!("     {}", out::hint(&validator.identifier));
        }
        println!();
        println!(
            "  {} of {} {}",
            out::accent(&args.limit.min(validators.len()).to_string()).bold(),
            out::accent(&validators.len().to_string()).bold(),
            out::hint("validators")
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.str_id(),
        "validators": validators
            .iter()
            .take(args.limit)
            .map(|validator| serde_json::json!({
                "identifier": validator.identifier,
                "name": validator.display_name,
                "commission": validator.commission,
                "minDelegationSmallestUnit": validator.min_delegation_smallest_unit,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

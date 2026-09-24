//! Read-only. Core's build-and-sign paths exist and want `send`'s `--yes`
//! treatment before they are exposed here.

use clap::{Args, Subcommand};
use colored::Colorize as _;

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::CliResult;
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum StakingCommand {
    /// Validators a chain offers, with their APY.
    Validators(ValidatorsArgs),
    /// What a wallet currently has staked.
    Positions(PositionsArgs),
    /// Effective configured endpoints, without making network requests.
    Endpoints(ValidatorsArgs),
}

#[derive(Args)]
pub struct ValidatorsArgs {
    /// Chain display name, registry id or symbol.
    #[arg(long)]
    chain: String,
    /// Most validators to show.
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Args)]
pub struct PositionsArgs {
    /// Wallet id, name or address.
    wallet: String,
}

pub fn run(ctx: &Ctx, out: Out, command: StakingCommand) -> CliResult<()> {
    match command {
        StakingCommand::Validators(args) => validators(ctx, out, args),
        StakingCommand::Positions(args) => positions(ctx, out, args),
        StakingCommand::Endpoints(args) => {
            let chain = resolve_chain(&args.chain)?;
            let config = ctx
                .rt
                .block_on(ctx.service()?.staking_endpoints(chain.str_id().into()))?;
            out.emit(
                serde_json::json!({"ok":true,"chain":config.chain_id,"endpoints":config.endpoints}),
            );
            Ok(())
        }
    }
}

fn validators(ctx: &Ctx, out: Out, args: ValidatorsArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let service = ctx.service()?;
    let validators = ctx
        .rt
        .block_on(service.fetch_staking_validators(chain.str_id().to_string()))?;

    out.text(|| {
        println!();
        if validators.is_empty() {
            println!("  {}", out::hint("no validators reported"));
            return;
        }
        for validator in validators.iter().take(args.limit) {
            println!(
                "  {}  {:<34} {:>7}",
                out::tint("●", chain.str_id()).bold(),
                validator.display_name,
                format!("{:.2}%", validator.apy * 100.0).bold(),
            );
            println!("     {}", out::hint(&validator.identifier));
        }
        println!();
        println!(
            "  {} {}",
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
                "apy": validator.apy,
                "commission": validator.commission,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

fn positions(ctx: &Ctx, out: Out, args: PositionsArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = resolve_chain(&wallet.chain_id)?.mainnet_counterpart();
    let service = ctx.service()?;
    let positions = ctx
        .rt
        .block_on(service.fetch_staking_positions(wallet.id.clone()))?;

    out.text(|| {
        println!();
        if positions.is_empty() {
            println!("  {}", out::hint("nothing staked"));
            return;
        }
        for position in &positions {
            println!(
                "  {}  {:<30} {}",
                out::tint("●", &wallet.chain_id).bold(),
                position.validator_display_name,
                format!("{:?}", position.status).to_lowercase(),
            );
            out::field("staked", &position.staked_amount_smallest_unit);
            if position.claimable_rewards_smallest_unit != "0" {
                out::field("rewards", &position.claimable_rewards_smallest_unit);
            }
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "chain": chain.str_id(),
        "positions": positions
            .iter()
            .map(|position| serde_json::json!({
                "validator": position.validator_identifier,
                "name": position.validator_display_name,
                "status": format!("{:?}", position.status).to_lowercase(),
                "staked": position.staked_amount_smallest_unit,
                "unbonding": position.unbonding_amount_smallest_unit,
                "withdrawable": position.withdrawable_amount_smallest_unit,
                "claimableRewards": position.claimable_rewards_smallest_unit,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

//! Read-only. Core's build-and-sign paths exist and want `send`'s `--yes`
//! treatment before they are exposed here.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::registry::Chain;
use spectra_core::staking::service::StakingService;
use spectra_core::staking::StakingError;

use super::chain::{BALANCE, RPC};
use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum StakingCommand {
    /// Validators a chain offers, with their APY.
    Validators(ValidatorsArgs),
    /// What a wallet currently has staked.
    Positions(PositionsArgs),
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
    }
}

/// A staking service bound to one chain's endpoints.
///
/// The flag is checked before the endpoints are: a chain that does not stake
/// should say so rather than report "no endpoints registered for Bitcoin",
/// which is true and about the wrong thing. It is also the same flag the app's
/// staking picker is built from, so the two refuse the same set.
fn service_for(chain: Chain) -> CliResult<std::sync::Arc<StakingService>> {
    if !chain.supports_staking() {
        return Err(CliError::rejected(format!(
            "{} does not have protocol-native staking",
            chain.chain_display_name()
        )));
    }
    let name = chain.chain_display_name().to_string();
    let endpoints: Vec<String> =
        spectra_core::endpoint_records_for_chain_masked(name.clone(), BALANCE | RPC, false)
            .map_err(CliError::from)?
            .into_iter()
            .map(|record| record.endpoint)
            .collect();
    if endpoints.is_empty() {
        return Err(CliError::failure(format!(
            "no endpoints registered for {name}"
        )));
    }
    Ok(StakingService::new(vec![spectra_core::service::ChainEndpoints {
        chain_id: chain.str_id().to_string(),
        endpoints,
        api_key: None,
    }]))
}

/// "This chain does not stake" is core considering the request and saying no,
/// which a script has to be able to tell from the network being down.
fn staking_error(error: StakingError) -> CliError {
    match error {
        StakingError::NotYetImplemented => CliError::rejected(error.to_string()),
        other => CliError::failure(other.to_string()),
    }
}

fn validators(ctx: &Ctx, out: Out, args: ValidatorsArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let service = service_for(chain)?;
    let validators = ctx
        .rt
        .block_on(service.fetch_validators(chain.str_id().to_string()))
        .map_err(staking_error)?;

    out.text(|| {
        println!();
        if validators.is_empty() {
            println!("  {}", out::hint("no validators reported"));
            return;
        }
        for validator in validators.iter().take(args.limit) {
            println!(
                "  {}  {:<34} {:>7}",
                out::tint("●", chain.chain_display_name()).bold(),
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
        "chain": chain.chain_display_name(),
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
    let chain = resolve_chain(&wallet.chain_name)?;
    let service = service_for(chain)?;
    let positions = ctx
        .rt
        .block_on(service.fetch_positions(
            chain.str_id().to_string(),
            wallet_address(&wallet).to_string(),
        ))
        .map_err(staking_error)?;

    out.text(|| {
        println!();
        if positions.is_empty() {
            println!("  {}", out::hint("nothing staked"));
            return;
        }
        for position in &positions {
            println!(
                "  {}  {:<30} {}",
                out::tint("●", &wallet.chain_name).bold(),
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
        "chain": chain.chain_display_name(),
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

//! Transactions: what core has recorded, and putting a new one on a chain.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::send::ethereum::{
    parse_evm_custom_fees, parse_evm_nonce, prepare_evm_send_assembly, EvmSendAssemblyInput,
    EvmSendOverridesInput, EvmSupportedToken,
};
use spectra_core::send::{
    send_affordability, SendAffordability, SendAffordabilityInput, SendExecutionRequest,
};
use spectra_core::service::WalletService;
use spectra_core::store::wallet_domain::CoreTransactionKind;

use super::chain::{
    service_for_chain, ENDPOINT_CAPABILITY_BALANCE, ENDPOINT_CAPABILITY_BROADCAST,
    ENDPOINT_CAPABILITY_FEE, ENDPOINT_CAPABILITY_UTXO,
};
use super::resolve_chain;
use crate::ctx::{Ctx, SecretSource};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Args)]
pub struct TxsArgs {
    /// Read the recent/pending projection and indexed history aggregates.
    #[arg(long)]
    summary: bool,
    /// Read one transaction directly by stored ID.
    #[arg(long)]
    record: Option<String>,
    /// The two ends of one stored transaction, and which is the wallet's own.
    #[arg(long, value_name = "ID")]
    endpoints: Option<String>,

    /// Query a bounded, deduplicated page of stored history.
    #[arg(long)]
    page: bool,
    #[arg(long, default_value_t = 20)]
    limit: u32,
    /// Continue with the nextCursor returned by the preceding page.
    #[arg(long, requires = "page")]
    cursor: Option<String>,
    #[arg(long, default_value = "")]
    search: String,
    #[arg(long)]
    oldest_first: bool,
    #[arg(long, default_value = "all", value_parser = ["all", "send", "receive", "pending"])]
    filter: String,

    /// Explicit read endpoint for the rechecked transaction's stored network.
    #[arg(long, requires = "recheck")]
    endpoint: Option<String>,
    /// Recheck one stored ENDPOINT_CAPABILITY_UTXO transaction, including failed or confirmed records.
    #[arg(long, conflicts_with_all = ["refresh_pending", "maintenance", "poll_chain", "wallet", "replaceable"])]
    recheck: Option<String>,
    /// Poll all stored transaction networks and persist status changes.
    #[arg(long, conflicts_with_all = ["maintenance", "poll_chain", "wallet", "replaceable"])]
    refresh_pending: bool,
    /// Show chains whose stored transactions still need polling.
    #[arg(long)]
    maintenance: bool,
    /// Poll pending transactions for this chain and persist status changes.
    #[arg(long, conflicts_with_all = ["wallet", "replaceable"])]
    poll_chain: Option<String>,
    /// Only this wallet's transactions (id, name or address).
    #[arg(long)]
    wallet: Option<String>,
    /// Only the pending sends that can still be replaced on their chain.
    #[arg(long)]
    replaceable: bool,
}

/// Putting a transfer on a chain, and looking at one first.
///
/// `broadcast` is a subcommand rather than the bare verb on purpose: the
/// irreversible half of this tool should take a word that says so.
#[derive(Subcommand)]
pub enum SendCommand {
    /// Inspect durable local Monero scan progress without contacting a node.
    MoneroStatus {
        #[arg(long)]
        from: String,
    },
    /// Scan Monero locally. No view or spend key is sent to the daemon.
    SyncMonero {
        #[command(flatten)]
        identity: IdentityArgs,
        /// Only valid before the first scan; earlier transfers will not be discovered.
        #[arg(long)]
        restore_height: Option<u64>,
        /// Scan one durable batch instead of continuing to the chain tip.
        #[arg(long)]
        once: bool,
    },
    /// Build and persist the exact transaction for review, without signing.
    Build {
        #[arg(long)]
        from: String,
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
        max_fee_gwei: Option<f64>,
        #[arg(long, requires = "max_fee_gwei")]
        priority_fee_gwei: Option<f64>,
    },
    /// Build a tracked holding from user edits, persisting its risk review.
    BuildOwned {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        holding: String,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        destination: String,
    },
    /// List prepared and signed transactions that survive app restarts.
    List,
    /// Inspect a persisted prepared or signed transaction.
    Inspect { transaction_id: String },
    /// Sign exactly the reviewed transaction and persist it without broadcasting.
    Sign {
        transaction_id: String,
        #[arg(long)]
        review_digest: String,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        password_file: Option<String>,
        #[arg(long, default_value = "SPECTRA_PASSWORD")]
        password_env: Option<String>,
    },
    /// Broadcast the same saved signed payload to explicitly selected endpoints.
    BroadcastSigned {
        transaction_id: String,
        #[arg(long, required = true)]
        endpoint: Vec<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Show the actual configured endpoint table offline, in service order.
    ConfiguredEndpoints { chain: String },
    /// Review stored-asset routing and submit preflight offline; never signs.
    Review {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        holding: String,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        destination: String,
    },
    /// Quote a stored holding on its selected network; never signs.
    Preview {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        holding: String,
        #[arg(long)]
        amount: String,
        #[arg(long, default_value = "")]
        destination: String,
    },
    /// Resolve an owned send request and fees, without signing or broadcasting.
    Quote {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        holding: String,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        destination: String,
    },
    /// Whether a destination is one of the user's own addresses on the holding's network.
    SelfCheck {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        holding: String,
        #[arg(long)]
        destination: String,
    },
    /// Build a replacement or cancellation from a stored pending transaction; never signs.
    Replacement {
        transaction_id: String,
        #[arg(long)]
        cancel: bool,
    },
    /// Send a stored holding using core-owned routing, fees and balances.
    OwnedBroadcast {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        holding: String,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        destination: String,
        #[arg(long)]
        yes: bool,
        #[arg(long, value_name = "PATH")]
        password_file: Option<String>,
        #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
        password_env: Option<String>,
    },
    /// Resubmit the signed payload of a stored transaction.
    Rebroadcast {
        transaction_id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Resolve the stored sender and check its signing identity offline.
    Identity(IdentityArgs),
    /// Validate exact decimal input and show integer units, without keys or network.
    Amount(AmountArgs),
    /// Convert a fee-adjusted estimate to a conservative decimal shortcut offline.
    Shortcut(ShortcutArgs),
    /// Validate a fee or gas budget in native units, without keys or network.
    FeeUnits(FeeUnitsArgs),
    /// Sign and broadcast a transfer.
    Broadcast(SendArgs),
    /// Build the transaction an EVM send would sign — no key, no network.
    Assemble(AssembleArgs),
    /// Ask what a recipient address looks like before sending to it.
    Probe(ProbeArgs),
    /// Resolve what was typed into the address a send would go to.
    Destination(DestinationArgs),
    /// Read the address a scanned QR payload carries for a chain, offline.
    Scan(ScanArgs),
    /// Ask whether a send can land once the fee is counted.
    Affordability(AffordabilityArgs),
    /// Validate custom EVM gas fees in gwei, without keys or network.
    Fees(FeesArgs),
    /// Validate EVM nonce, gas, calldata and access-list overrides offline.
    Overrides(OverridesArgs),
}

pub fn run(ctx: &Ctx, out: Out, command: SendCommand) -> CliResult<()> {
    match command {
        SendCommand::Build {
            from,
            to,
            amount,
            endpoint,
            contract,
            decimals,
            nonce,
            gas_limit,
            max_fee_gwei,
            priority_fee_gwei,
        } => {
            let wallet = ctx.find_wallet(&from)?;
            let chain = wallet
                .chain()
                .ok_or_else(|| CliError::usage("Invalid wallet network"))?;
            let service = staged_service(ctx, chain.str_id(), endpoint.into_iter().collect())?;
            let artifact =
                ctx.rt.block_on(
                    service.build_send(SendExecutionRequest {
                        chain_id: chain.str_id().into(),
                        wallet_id: wallet.id,
                        password: None,
                        to_address: to,
                        amount_str: amount,
                        contract_address: contract,
                        token_decimals: decimals,
                        fee_rate_svb: None,
                        fee_sat: None,
                        gas_budget: None,
                        fee_amount: None,
                        evm_overrides: (nonce.is_some()
                            || gas_limit.is_some()
                            || max_fee_gwei.is_some())
                        .then_some(EvmSendOverridesInput {
                            nonce,
                            gas_limit,
                            custom_fees: max_fee_gwei.zip(priority_fee_gwei).map(
                                |(max_fee_per_gas_gwei, max_priority_fee_per_gas_gwei)| {
                                    spectra_core::send::ethereum::EvmCustomFeeConfiguration {
                                        max_fee_per_gas_gwei,
                                        max_priority_fee_per_gas_gwei,
                                    }
                                },
                            ),
                            ..Default::default()
                        }),
                        monero_priority: None,
                        sign_only: false,
                    }),
                )?;
            emit_artifact(out, &artifact);
            Ok(())
        }
        SendCommand::List => {
            let artifacts = ctx.rt.block_on(ctx.service()?.list_sends())?;
            out.text(|| {
                for artifact in &artifacts {
                    println!(
                        "{} {:?} {} {}",
                        artifact.id, artifact.stage, artifact.chain_id, artifact.amount
                    );
                }
            });
            out.emit(serde_json::json!({"artifacts":artifacts}));
            Ok(())
        }
        SendCommand::Inspect { transaction_id } => {
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.inspect_send(transaction_id))?;
            emit_artifact(out, &artifact);
            Ok(())
        }
        SendCommand::Sign {
            transaction_id,
            review_digest,
            endpoint,
            password_file,
            password_env,
        } => {
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.inspect_send(transaction_id.clone()))?;
            let password = signing_password(ctx, &artifact.wallet_id, password_file, password_env)?;
            let service = staged_service(ctx, &artifact.chain_id, endpoint.into_iter().collect())?;
            let artifact =
                ctx.rt
                    .block_on(service.sign_send(transaction_id, review_digest, password))?;
            emit_artifact(out, &artifact);
            Ok(())
        }
        SendCommand::BroadcastSigned {
            transaction_id,
            endpoint,
            yes,
        } => {
            if !yes {
                return Err(CliError::usage("broadcast-signed requires --yes"));
            }
            let artifact = ctx
                .rt
                .block_on(ctx.service()?.inspect_send(transaction_id.clone()))?;
            let service = staged_service(ctx, &artifact.chain_id, endpoint.clone())?;
            let artifact = ctx
                .rt
                .block_on(service.broadcast_send(transaction_id, endpoint))?;
            emit_artifact(out, &artifact);
            Ok(())
        }
        SendCommand::ConfiguredEndpoints { chain } => {
            let chain = resolve_chain(&chain)?;
            let endpoints = ctx
                .rt
                .block_on(ctx.service()?.send_endpoints(chain.str_id().into()))?;
            out.text(|| {
                for (index, endpoint) in endpoints.iter().enumerate() {
                    println!("{} {}", index + 1, endpoint);
                }
            });
            out.emit(serde_json::json!({"chain":chain.str_id(),"endpoints":endpoints}));
            Ok(())
        }

        SendCommand::Review {
            wallet,
            holding,
            amount,
            destination,
        } => {
            let wallet = ctx.find_wallet(&wallet)?;
            let service = ctx.service()?;
            let route = ctx
                .rt
                .block_on(service.send_asset_routing(wallet.id.clone(), holding.clone()));
            let preflight = ctx.rt.block_on(service.send_submit_preflight(
                wallet.id,
                holding,
                destination,
                amount,
            ))?;
            out.emit(serde_json::json!({"route":route,"preflight":preflight}));
            Ok(())
        }
        SendCommand::Preview {
            wallet,
            holding,
            amount,
            destination,
        } => {
            let wallet = ctx.find_wallet(&wallet)?;
            let preview = ctx.rt.block_on(ctx.service()?.preview_owned_send(
                wallet.id,
                holding,
                amount,
                destination,
                None,
                None,
            ))?;
            out.emit(serde_json::json!({"preview":preview}));
            Ok(())
        }
        SendCommand::Replacement {
            transaction_id,
            cancel,
        } => {
            let draft = ctx
                .rt
                .block_on(ctx.service()?.replacement_draft(transaction_id, cancel))?;
            out.emit(serde_json::json!({"draft": draft}));
            Ok(())
        }
        SendCommand::OwnedBroadcast {
            wallet,
            holding,
            amount,
            destination,
            yes,
            password_file,
            password_env,
        } => {
            if !yes {
                return Err(CliError::rejected("Broadcast requires --yes"));
            }
            let wallet = ctx.find_wallet(&wallet)?;
            let input = spectra_core::service::send_review::SendReviewInput {
                wallet_id: wallet.id,
                holding_key: holding,
                amount,
                destination,
                overrides: None,
            };
            let service = ctx.service()?;
            let password = signing_password(ctx, &input.wallet_id, password_file, password_env)?;
            let review = ctx.rt.block_on(service.review_owned_send(input.clone()))?;
            let result = ctx
                .rt
                .block_on(service.execute_owned_send(review.id, input, password))?;
            out.emit(serde_json::json!({"transactionHash": result.transaction_hash}));
            Ok(())
        }
        SendCommand::BuildOwned {
            wallet,
            holding,
            amount,
            destination,
        } => {
            let wallet = ctx.find_wallet(&wallet)?;
            let artifact = ctx.rt.block_on(ctx.service()?.build_owned_send(
                spectra_core::service::send_review::SendReviewInput {
                    wallet_id: wallet.id,
                    holding_key: holding,
                    amount,
                    destination,
                    overrides: None,
                },
            ))?;
            out.emit(serde_json::json!({"artifact":artifact}));
            Ok(())
        }
        SendCommand::Quote {
            wallet,
            holding,
            amount,
            destination,
        } => {
            let wallet = ctx.find_wallet(&wallet)?;
            let quote = ctx.rt.block_on(ctx.service()?.review_owned_send(
                spectra_core::service::send_review::SendReviewInput {
                    wallet_id: wallet.id,
                    holding_key: holding,
                    amount,
                    destination,
                    overrides: None,
                },
            ))?;
            out.emit(serde_json::json!({"quote":quote}));
            Ok(())
        }
        SendCommand::SelfCheck {
            wallet,
            holding,
            destination,
        } => {
            let wallet = ctx.find_wallet(&wallet)?;
            let own = ctx.rt.block_on(ctx.service()?.is_own_send_destination(
                wallet.id,
                holding,
                destination,
            ))?;
            out.text(|| {
                println!(
                    "  {}",
                    if own {
                        "own address"
                    } else {
                        "not an own address"
                    }
                )
            });
            out.emit(serde_json::json!({"ownAddress": own}));
            Ok(())
        }
        SendCommand::Rebroadcast {
            transaction_id,
            yes,
        } => {
            if !yes {
                return Err(CliError::usage("rebroadcast requires --yes"));
            }
            let hash = ctx
                .rt
                .block_on(ctx.service()?.rebroadcast_transaction(transaction_id))
                .map_err(CliError::from)?;
            out.emit(serde_json::json!({"ok": true, "transactionHash": hash}));
            Ok(())
        }
        SendCommand::MoneroStatus { from } => {
            let wallet = ctx.find_wallet(&from)?;
            let service = ctx.service()?;
            service.set_secret_store(ctx.secrets.clone());
            let status = ctx
                .rt
                .block_on(service.monero_sync_status(wallet.id.clone()))?;
            out.emit(serde_json::json!({"sync": status}));
            Ok(())
        }
        SendCommand::SyncMonero {
            identity: args,
            mut restore_height,
            once,
        } => {
            let wallet = ctx.find_wallet(&args.from)?;
            if let Some(chain) = args.chain.as_deref() {
                if resolve_chain(chain)?.str_id() != wallet.chain_id {
                    return Err(CliError::usage(
                        "Monero sync must use the wallet's selected network",
                    ));
                }
            }
            let password =
                signing_password(ctx, &wallet.id, args.password_file, args.password_env)?;
            let service = ctx.service()?;
            service.set_secret_store(ctx.secrets.clone());
            loop {
                let status = ctx.rt.block_on(service.sync_monero_wallet(
                    wallet.id.clone(),
                    password.clone(),
                    restore_height.take(),
                ))?;
                if once || status.complete {
                    out.emit(serde_json::json!({"sync":status}));
                    break;
                }
                eprintln!(
                    "Monero scan: {} / {}",
                    status.scanned_height, status.target_height
                );
            }
            Ok(())
        }
        SendCommand::Identity(args) => identity(ctx, out, args),
        SendCommand::Amount(args) => exact_amount(out, args),
        SendCommand::Shortcut(args) => shortcut(out, args),
        SendCommand::FeeUnits(args) => fee_units(out, args),
        SendCommand::Broadcast(args) => send(ctx, out, args),
        SendCommand::Assemble(args) => assemble(ctx, out, args),
        SendCommand::Probe(args) => probe(ctx, out, args),
        SendCommand::Destination(args) => destination(ctx, out, args),
        SendCommand::Scan(args) => scan(out, args),
        SendCommand::Affordability(args) => affordability(out, args),
        SendCommand::Fees(args) => fees(out, args),
        SendCommand::Overrides(args) => overrides(out, args),
    }
}

#[derive(Args)]
pub struct IdentityArgs {
    #[arg(long)]
    from: String,
    /// Defaults to the wallet's chain; EVM wallets may select another shared-address chain.
    #[arg(long)]
    chain: Option<String>,
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
}

fn signing_password(
    ctx: &Ctx,
    wallet_id: &str,
    file: Option<String>,
    env: Option<String>,
) -> CliResult<Option<String>> {
    let requires_password = ctx
        .state()?
        .wallets
        .iter()
        .find(|wallet| wallet.id == wallet_id)
        .is_some_and(|wallet| wallet.signing.requires_password());
    if !requires_password {
        return Ok(None);
    }
    let env = env.filter(|name| std::env::var_os(name).is_some());
    Ok(Some(SecretSource { file, env }.resolve("password")?))
}

fn identity(ctx: &Ctx, out: Out, args: IdentityArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.from)?;
    let chain = resolve_chain(args.chain.as_deref().unwrap_or(&wallet.chain_id))?;
    let password = signing_password(ctx, &wallet.id, args.password_file, args.password_env)?;
    let service = ctx.service()?;
    service.set_secret_store(ctx.secrets.clone());
    let address = ctx.rt.block_on(service.send_identity_address(
        wallet.id.clone(),
        chain.str_id().into(),
        password,
    ))?;
    out.text(|| println!("  {} sender: {address}", chain.chain_display_name()));
    out.emit(
        serde_json::json!({ "walletId": wallet.id, "chain": chain.str_id(), "address": address }),
    );
    Ok(())
}

#[derive(Args)]
pub struct AmountArgs {
    #[arg(long)]
    chain: String,
    /// Override precision for a token amount.
    #[arg(long)]
    decimals: Option<u32>,
    #[arg(long, allow_hyphen_values = true)]
    amount: String,
}

fn exact_amount(out: Out, args: AmountArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let decimals = args.decimals.unwrap_or(u32::from(chain.native_decimals()));
    let raw = spectra_core::send::amount_input::parse_raw_amount(&args.amount, decimals)?;
    out.text(|| println!("  {raw} integer units ({decimals} decimals)"));
    out.emit(serde_json::json!({ "chain": chain.str_id(), "decimals": decimals, "rawAmount": raw.to_string() }));
    Ok(())
}

#[derive(Args)]
pub struct ShortcutArgs {
    #[arg(long)]
    maximum: String,
    #[arg(long)]
    decimals: u32,
    #[arg(long, default_value_t = 100)]
    percentage: u32,
}
fn shortcut(out: Out, args: ShortcutArgs) -> CliResult<()> {
    let amount = spectra_core::send::amount_input::send_amount_shortcut(
        args.maximum,
        args.decimals,
        args.percentage,
    )
    .ok_or_else(|| {
        spectra_core::SpectraBridgeError::from("no positive amount within the quoted maximum")
    })?;
    out.text(|| println!("  {amount}"));
    out.emit(serde_json::json!({"amount": amount}));
    Ok(())
}

#[derive(Args)]
pub struct FeeUnitsArgs {
    #[arg(long)]
    chain: String,
    #[arg(long, allow_hyphen_values = true)]
    amount: f64,
}

fn fee_units(out: Out, args: FeeUnitsArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let raw =
        spectra_core::send::payload::fee_units(args.amount, u32::from(chain.native_decimals()))?;
    out.text(|| println!("  {raw} native integer units"));
    out.emit(serde_json::json!({"ok": true, "rawFee": raw.to_string()}));
    Ok(())
}

#[derive(Args)]
pub struct OverridesArgs {
    #[arg(long, default_value = "Ethereum")]
    chain: String,
    #[arg(long, allow_hyphen_values = true)]
    nonce: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    gas_limit: Option<i64>,
    /// Hex calldata, with or without 0x. Requires --gas-limit.
    #[arg(long)]
    calldata: Option<String>,
    /// JSON array of {address, storageKeys}; non-empty lists require --gas-limit.
    #[arg(long)]
    access_list: Option<String>,
    #[arg(long)]
    sign_only: bool,
}

fn overrides(out: Out, args: OverridesArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let resolved = EvmSendOverridesInput {
        nonce: args
            .nonce
            .map(parse_evm_nonce)
            .transpose()
            .map_err(|error| CliError::rejected(error.to_string()))?,
        gas_limit: args.gas_limit,
        calldata_hex: args.calldata,
        access_list_json: args.access_list,
        sign_only: Some(args.sign_only),
        ..Default::default()
    }
    .resolve(chain)?;
    out.text(|| {
        println!(
            "  {} valid EVM overrides (no signing or broadcast)",
            out::ok_mark()
        )
    });
    out.emit(serde_json::json!({
        "ok": true,
        "nonce": resolved.nonce,
        "gasLimit": resolved.gas_limit,
        "calldataBytes": resolved.calldata.as_ref().map(Vec::len),
        "accessListEntries": resolved.access_list.len(),
        "storageKeys": resolved.access_list.iter().map(|entry| entry.storage_keys.len()).sum::<usize>(),
        "signOnly": resolved.sign_only,
    }));
    Ok(())
}

#[derive(Args)]
pub struct FeesArgs {
    /// Maximum total fee per gas, in gwei.
    #[arg(long, allow_hyphen_values = true)]
    max_fee: String,
    /// Priority fee per gas, in gwei.
    #[arg(long, allow_hyphen_values = true)]
    priority_fee: String,
}

fn fees(out: Out, args: FeesArgs) -> CliResult<()> {
    let fees = parse_evm_custom_fees(args.max_fee, args.priority_fee)
        .map_err(|error| CliError::rejected(error.to_string()))?;
    out.text(|| {
        out::field("max fee (gwei)", &fees.max_fee_per_gas_gwei.to_string());
        out::field(
            "priority fee (gwei)",
            &fees.max_priority_fee_per_gas_gwei.to_string(),
        );
    });
    out.emit(serde_json::json!({ "ok": true, "fees": fees }));
    Ok(())
}

#[derive(Args)]
pub struct AffordabilityArgs {
    /// Deployment id from `token catalog`; omit for the native token.
    #[arg(long)]
    deployment: Option<String>,
    /// Chain the send is on.
    #[arg(long)]
    chain: String,
    /// Asset being sent.
    #[arg(long)]
    symbol: String,
    /// Amount, in whole units of that asset.
    #[arg(long)]
    amount: String,
    /// Network fee, in whole units of the chain's gas asset.
    #[arg(long)]
    fee: String,
    /// What the wallet holds of the asset being sent.
    #[arg(long)]
    balance: String,
    /// What it holds of the gas asset. Omit for a send of the chain's own asset.
    #[arg(long)]
    gas_balance: Option<String>,
}

/// The fee half of "can this send land", on the command line.
///
/// Whether the asset is the chain's own, what the gas asset is called and how
/// many decimals a fee is quoted to are all read from the registry — naming
/// the chain is the whole input.
fn affordability(out: Out, args: AffordabilityArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let deployment_id = args
        .deployment
        .unwrap_or_else(|| chain.entry().native_deployment_id.clone());
    let token = spectra_core::tokens::deployment(&deployment_id)
        .ok_or_else(|| CliError::usage("unknown deployment"))?;
    if token.chain_id != chain.str_id() || token.symbol != args.symbol {
        return Err(CliError::usage(
            "deployment does not match the selected network and symbol",
        ));
    }
    let verdict = send_affordability(SendAffordabilityInput {
        is_native: token.is_native(),
        chain_id: chain.str_id().to_string(),
        symbol: args.symbol,
        amount: args.amount,
        network_fee: args.fee,
        holding_balance: args.balance,
        gas_balance: args.gas_balance,
    });

    let body = match &verdict {
        SendAffordability::Unavailable => serde_json::json!({"verdict":"unavailable"}),
        SendAffordability::Affordable => serde_json::json!({ "verdict": "affordable" }),
        SendAffordability::AmountPlusFeeExceedsBalance { symbol, required } => serde_json::json!({
            "verdict": "amountPlusFeeExceedsBalance", "symbol": symbol, "required": required,
        }),
        SendAffordability::AmountExceedsBalance { symbol } => serde_json::json!({
            "verdict": "amountExceedsBalance", "symbol": symbol,
        }),
        SendAffordability::FeeExceedsGasBalance {
            gas_symbol,
            fee,
            chain_id,
        } => {
            serde_json::json!({
                "verdict": "feeExceedsGasBalance", "gasSymbol": gas_symbol,
                "fee": fee, "chainId": chain_id,
            })
        }
    };

    out.text(|| {
        println!();
        match &verdict {
            SendAffordability::Unavailable => println!("Unable to determine fee affordability"),
            SendAffordability::Affordable => println!("  {}  the send fits", "\u{2713}".green()),
            SendAffordability::AmountPlusFeeExceedsBalance { symbol, required } => {
                println!(
                    "  {}  needs ~{required} {symbol} for the amount plus the fee",
                    "\u{2717}".red()
                )
            }
            SendAffordability::AmountExceedsBalance { symbol } => {
                println!(
                    "  {}  more {symbol} than the wallet holds",
                    "\u{2717}".red()
                )
            }
            SendAffordability::FeeExceedsGasBalance {
                gas_symbol,
                fee,
                chain_id,
            } => {
                let chain_name = super::chain_name(chain_id);
                println!(
                    "  {}  not enough {gas_symbol} for the ~{fee} {chain_name} fee",
                    "\u{2717}".red()
                )
            }
        }
    });
    out.emit(body);
    Ok(())
}

#[derive(Args)]
pub struct ProbeArgs {
    /// Wallet the send would come from (id, name or address).
    #[arg(long)]
    wallet: String,
    /// Asset symbol being sent. Defaults to the wallet chain's own asset.
    #[arg(long)]
    asset: Option<String>,
    /// Narrows the asset to one chain, for a symbol the wallet holds on several.
    #[arg(long)]
    chain: Option<String>,
    /// Recipient, as the user would type it.
    #[arg(long)]
    to: String,
}

/// Run the composer's recipient check by wallet and asset.
/// Core resolves the contract and returns typed verdict flags;
/// the CLI supplies the wording.
fn probe(ctx: &Ctx, out: Out, args: ProbeArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let wallet_chain = resolve_chain(&wallet.chain_id)?.mainnet_counterpart();
    let symbol = args
        .asset
        .clone()
        .unwrap_or_else(|| wallet_chain.coin_symbol().to_string());
    let on_chain = args.chain.as_deref().map(resolve_chain).transpose()?;

    let candidates: Vec<&spectra_core::store::wallet_domain::AssetHolding> = wallet
        .holdings
        .iter()
        .filter(|h| h.symbol.eq_ignore_ascii_case(&symbol))
        .filter(|h| on_chain.is_none_or(|c| c.str_id() == h.chain_id))
        .collect();
    let holding = match candidates.as_slice() {
        [] => {
            return Err(CliError::rejected(format!(
                "wallet {} holds no {symbol}",
                wallet.id
            )))
        }
        [one] => *one,
        many => {
            let chains: Vec<String> = many
                .iter()
                .map(|h| super::chain_name(&h.chain_id))
                .collect();
            return Err(CliError::usage(format!(
                "{symbol} is held on {} — narrow it with --chain",
                chains.join(", ")
            )));
        }
    };
    let chain = resolve_chain(&holding.chain_id)?;

    // Both halves in one service: the holding and the token row come from the
    // opened state, the balance and history reads from the chain's endpoints.
    let service = ctx.service()?;

    let holding_key = holding.deployment_id();
    let risk = ctx
        .rt
        .block_on(service.send_destination_risk(wallet.id.clone(), holding_key, args.to.clone()))
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        out::field("destination", &args.to);
        out::field("asset", &holding.symbol);
        out::field("chain", chain.chain_display_name());
        out::field(
            "balance",
            if risk.balance_is_zero {
                "zero"
            } else {
                "non-zero"
            },
        );
        out::field("history", if risk.has_history { "yes" } else { "none" });
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "chain": chain.str_id(),
        "destination": args.to,
        "asset": holding.symbol,
        "activity": risk.activity,
        "balanceIsZero": risk.balance_is_zero,
        "hasHistory": risk.has_history,
    }));
    Ok(())
}

#[derive(Args)]
pub struct DestinationArgs {
    /// Chain the send is on.
    #[arg(long)]
    chain: String,
    /// What the user typed: an address, or a name on a chain that resolves one.
    #[arg(long)]
    to: String,
    /// Address shown in a previous review; refuse if the destination changed.
    #[arg(long)]
    expected: Option<String>,
}

/// What the composer does with the destination field, on the command line.
///
/// Whether a `.eth` name is looked up is the chain's, not the caller's, so
/// this needs no flag to say "try ENS": ask any other chain and the name is
/// refused without a request leaving the machine.
#[derive(Args)]
pub struct ScanArgs {
    /// Chain the send is on. Required: a payload is only an address once a
    /// chain has judged it.
    #[arg(long)]
    chain: String,
    /// The scanned text — a bare address or a payment URI.
    payload: String,
}

/// The composer's scanner, without a camera. Exit 3 when the payload carries no
/// address this chain accepts, so a script can assert the refusal.
fn scan(out: Out, args: ScanArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let Some(address) = spectra_core::send::flow::scanned_send_address(
        chain.str_id().to_string(),
        args.payload.clone(),
    ) else {
        return Err(CliError::rejected(format!(
            "no {} address in that payload",
            chain.chain_display_name()
        )));
    };
    out.text(|| {
        println!();
        out::field("scanned", &args.payload);
        out::field("address", &address);
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.str_id(),
        "payload": args.payload,
        "address": address,
    }));
    Ok(())
}

fn destination(ctx: &Ctx, out: Out, args: DestinationArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    // ENS needs the registry-selected EVM API; other chains validate locally.
    let service = if chain.resolves_ens_names() {
        service_for_chain(ctx, chain, 0)?
    } else {
        WalletService::new(Vec::new()).map_err(CliError::from)?
    };
    let resolved = ctx
        .rt
        .block_on(async {
            match args.expected {
                Some(expected) => {
                    service
                        .verify_send_destination(chain.str_id().into(), args.to.clone(), expected)
                        .await
                }
                None => {
                    service
                        .resolve_send_destination(chain.str_id().into(), args.to.clone())
                        .await
                }
            }
        })
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        out::field("typed", &args.to);
        out::field("address", &resolved.address);
        out::field("via", if resolved.used_ens { "ENS" } else { "typed" });
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.str_id(),
        "typed": args.to,
        "address": resolved.address,
        "usedEns": resolved.used_ens,
    }));
    Ok(())
}

#[derive(Args)]
pub struct AssembleArgs {
    /// Chain to assemble for.
    #[arg(long)]
    chain: String,
    /// Sender address.
    #[arg(long)]
    from: String,
    /// Recipient address.
    #[arg(long)]
    to: String,
    /// Amount, in whole units of the asset being sent.
    #[arg(long)]
    amount: String,
    /// Asset to send. Defaults to what the chain pays fees in.
    #[arg(long)]
    symbol: Option<String>,
    /// ERC-20 contract, when sending a token rather than the gas asset.
    #[arg(long)]
    contract: Option<String>,
    /// Token decimals. Required with --contract.
    #[arg(long)]
    decimals: Option<u32>,
}

#[derive(Args)]
pub struct SendArgs {
    /// Wallet to send from (id, name or address).
    #[arg(long)]
    from: String,
    /// Recipient address.
    #[arg(long)]
    to: String,
    /// Amount in the chain's native asset.
    #[arg(long)]
    amount: String,
    /// Broadcast without asking for confirmation.
    #[arg(long)]
    yes: bool,
    /// Sign the transaction and stop, printing the raw payload. Reads the live
    /// nonce or ENDPOINT_CAPABILITY_UTXO set, moves nothing, and needs no `--yes`.
    #[arg(long)]
    sign_only: bool,
    /// EVM gas limit. Given explicitly, the builder skips estimation — which
    /// is what lets an unfunded address sign, since a node refuses to estimate
    /// a transfer it cannot pay for.
    #[arg(long)]
    gas_limit: Option<i64>,
    /// EVM nonce. Omitted, the live one is read from the node.
    #[arg(long)]
    nonce: Option<i64>,
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: Option<String>,
}

/// Transactions core has recorded locally. Distinct from `history`, which asks
/// the chain.
pub fn txs(ctx: &Ctx, out: Out, args: TxsArgs) -> CliResult<()> {
    if args.summary {
        let summary = ctx.rt.block_on(ctx.service()?.transaction_snapshot())?;
        out.text(|| println!("{summary:?}"));
        out.emit(serde_json::json!({"ok":true,"summary":summary}));
        return Ok(());
    }
    if let Some(id) = args.endpoints {
        let endpoints = ctx
            .rt
            .block_on(ctx.service()?.transaction_endpoints(id))?
            .ok_or_else(|| CliError::rejected("no such transaction"))?;
        out.text(|| {
            for (label, end) in [("from", &endpoints.from), ("to", &endpoints.to)] {
                if let Some(end) = end {
                    let mine = if end.is_mine { "  (this wallet)" } else { "" };
                    println!("  {label:<4} {}{mine}", end.address);
                }
            }
        });
        out.emit(serde_json::json!({"ok":true,"endpoints":endpoints}));
        return Ok(());
    }
    if let Some(id) = args.record {
        let record = ctx.rt.block_on(ctx.service()?.transaction(id))?;
        out.text(|| println!("{record:?}"));
        out.emit(serde_json::json!({"ok":true,"actions":record.as_ref().map(|r| &r.actions),"record":record}));
        return Ok(());
    }
    if args.page {
        use spectra_core::service::{HistoryQuery, HistoryQueryFilter};
        let wallet_id = args
            .wallet
            .as_deref()
            .map(|needle| ctx.find_wallet(needle).map(|w| w.id))
            .transpose()?;
        let page = ctx.rt.block_on(ctx.service()?.history_page(HistoryQuery {
            wallet_id,
            filter: match args.filter.as_str() {
                "send" => HistoryQueryFilter::Send,
                "receive" => HistoryQueryFilter::Receive,
                "pending" => HistoryQueryFilter::Pending,
                _ => HistoryQueryFilter::All,
            },
            search: args.search,
            oldest_first: args.oldest_first,
            cursor: args.cursor,
            limit: args.limit,
        }))?;
        out.text(|| println!("{page:?}"));
        out.emit(serde_json::json!({"ok":true,"actions":page.records.iter().map(|r| (&r.id, &r.actions)).collect::<std::collections::BTreeMap<_, _>>(),"page":page}));
        return Ok(());
    }

    if let Some(id) = args.recheck {
        let service = ctx.service()?;
        if let Some(endpoint) = args.endpoint {
            let transaction = ctx
                .rt
                .block_on(service.transactions())?
                .into_iter()
                .find(|row| row.id.eq_ignore_ascii_case(&id))
                .ok_or_else(|| CliError::rejected("Transaction not found."))?;
            let chain = resolve_chain(&transaction.chain_id)?;
            ctx.rt.block_on(service.update_endpoints(
                vec![spectra_core::service::ChainEndpoints {
                    capabilities: vec![
                        "balance",
                        "history",
                        "utxo",
                        "fee",
                        "broadcast",
                        "verification",
                        "token-balance",
                        "token-discovery",
                        "token-history",
                        "staking",
                    ]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                    chain_id: chain.str_id().into(),
                    endpoints: vec![endpoint],
                }],
            ))?;
        }
        let change = ctx.rt.block_on(service.recheck_transaction_status(id))?;
        out.text(|| println!("{}: {}", change.id, change.new_status.as_raw()));
        out.emit(serde_json::json!({"ok": true, "change": change}));
        return Ok(());
    }
    if args.refresh_pending {
        let result = ctx
            .rt
            .block_on(ctx.service()?.refresh_pending_transactions())?;
        out.text(|| {
            println!(
                "{} networks, {} changes, {} failures",
                result.chains.len(),
                result.changes.len(),
                result.failures.len()
            )
        });
        out.emit(serde_json::json!({"ok": result.failures.is_empty(), "maintenance": result}));
        if !result.failures.is_empty() {
            return Err(CliError::rejected(
                "pending maintenance failed for one or more networks",
            ));
        }
        return Ok(());
    }
    if args.maintenance {
        let chains = ctx
            .rt
            .block_on(ctx.service()?.pending_maintenance_chains())?;
        out.text(|| println!("{}", chains.join(", ")));
        out.emit(serde_json::json!({"chains":chains}));
        return Ok(());
    }
    if let Some(name) = &args.poll_chain {
        let chain = resolve_chain(name)?;
        // A maintenance request names the stored transaction network.
        let network = chain;
        let service = WalletService::new(
            spectra_core::service::catalog_endpoints()?
                .into_iter()
                .filter(|row| row.chain_id.split(':').next() == Some(network.str_id()))
                .collect(),
        )
        .map_err(CliError::from)?;
        ctx.rt
            .block_on(service.open_state(ctx.db_path()))
            .map_err(CliError::from)?;
        ctx.prepare_transport(&service)?;
        let changes = ctx
            .rt
            .block_on(service.poll_pending_transactions(chain.str_id().into()))
            .map_err(CliError::from)?;
        out.text(|| println!("  {} transaction status changes", changes.len()));
        out.emit(serde_json::json!({"ok":true,"changes":changes}));
        return Ok(());
    }
    let service = ctx.service()?;
    if args.replaceable {
        return replaceable(ctx, out, args);
    }
    let records = match &args.wallet {
        Some(needle) => {
            let wallet = ctx.find_wallet(needle)?;
            ctx.rt
                .block_on(service.transactions_for_wallet(wallet.id))
                .map_err(CliError::from)?
        }
        None => ctx
            .rt
            .block_on(service.transactions())
            .map_err(CliError::from)?,
    };

    out.text(|| {
        println!();
        if records.is_empty() {
            println!("  {}", out::hint("nothing recorded"));
            return;
        }
        for record in &records {
            let incoming = matches!(record.kind, CoreTransactionKind::Receive);
            let mark = if incoming { "↓" } else { "↑" };
            let colored_mark = if incoming {
                mark.truecolor(120, 230, 160).bold()
            } else {
                mark.truecolor(255, 110, 130).bold()
            };
            println!(
                "  {}  {:>12}  {}  {}",
                colored_mark,
                format!("{:.6}", record.amount),
                out::tint(&record.symbol, &record.chain_id).bold(),
                out::hint(&record.address),
            );
            if let Some(hash) = &record.transaction_hash {
                println!("     {}", out::hint(&out::short_hash(hash)));
            }
        }
        println!();
        println!(
            "  {} {}",
            out::accent(&records.len().to_string()).bold(),
            out::hint(if records.len() == 1 {
                "transaction"
            } else {
                "transactions"
            })
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "count": records.len(),
        "transactions": records
            .iter()
            .map(|record| serde_json::json!({
                "hash": record.transaction_hash,
                "kind": match record.kind {
                    CoreTransactionKind::Send => "send",
                    CoreTransactionKind::Receive => "receive",
                },
                "amount": record.amount,
                "symbol": record.symbol,
                "chain": record.chain_id,
                "address": record.address,
                // What the detail sheet's "History Source" row shows, as the
                // app reads it: a proper noun, a chain's provider set, or
                // Spectra's own reader, which the row does not name.
                "historySource": record
                    .transaction_history_source
                    .clone()
                    .and_then(spectra_core::fetch::transactions::history_source)
                    .map(|source| match source {
                        spectra_core::fetch::transactions::HistorySource::Provider { name } =>
                            serde_json::json!({"provider": name}),
                        spectra_core::fetch::transactions::HistorySource::ChainProviders { chain_id } =>
                            serde_json::json!({"chainProviders": chain_id}),
                        spectra_core::fetch::transactions::HistorySource::Internal =>
                            serde_json::json!("internal"),
                    }),
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

/// The pending sends core says can still be replaced, and how.
///
/// The rule — an EVM chain, a send, still pending, with a hash to read its
/// nonce by — is `replaceable_sends`, derived where the records are. iOS asked
/// it of its own projection, and asked it of the chain *named* "Ethereum".
fn replaceable(ctx: &Ctx, out: Out, args: TxsArgs) -> CliResult<()> {
    let service = ctx.service()?;
    let wallet_id = match &args.wallet {
        Some(needle) => Some(ctx.find_wallet(needle)?.id),
        None => None,
    };
    let sends: Vec<_> = ctx
        .rt
        .block_on(service.replaceable_sends())?
        .into_iter()
        .filter(|send| {
            wallet_id
                .as_deref()
                .is_none_or(|id| send.wallet_id.eq_ignore_ascii_case(id))
        })
        .collect();

    out.text(|| {
        println!();
        if sends.is_empty() {
            println!("  {}", out::hint("nothing to replace"));
            return;
        }
        for send in &sends {
            println!(
                "  {:>12}  {}  {}",
                format!("{:.6}", send.amount),
                out::tint(&send.symbol, &send.chain_id).bold(),
                out::hint(&out::short_hash(&send.transaction_hash)),
            );
            println!(
                "     {}",
                out::hint(&match send.recorded_nonce {
                    Some(nonce) => format!(
                        "nonce {nonce} · {}",
                        if send.can_speed_up {
                            "speed up or cancel"
                        } else {
                            "cancel only"
                        }
                    ),
                    None => "cancel only".to_string(),
                })
            );
        }
        println!();
    });
    out.emit(serde_json::json!({
        "ok": true,
        "count": sends.len(),
        "replaceable": sends
            .iter()
            .map(|send| serde_json::json!({
                "transaction": send.transaction_id,
                "wallet": send.wallet_id,
                "chain": send.chain_id,
                "symbol": send.symbol,
                "amount": send.amount,
                "to": send.to_address,
                "hash": send.transaction_hash,
                "nonce": send.recorded_nonce,
                "canSpeedUp": send.can_speed_up,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

pub fn send(ctx: &Ctx, out: Out, args: SendArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.from)?;
    if wallet.is_watch_only() {
        return Err(CliError::rejected("a watch-only wallet cannot send"));
    }
    // The network this wallet is on, not its family's mainnet: it decides
    // which chain id is signed and which endpoints the send reads. Core
    // resolves it the same way, so the two agree on one rule
    // (`WalletState::chain`) rather than each having its own.
    let chain = wallet
        .chain()
        .unwrap_or(resolve_chain(&wallet.chain_id)?.mainnet_counterpart());

    let amount: f64 = args
        .amount
        .trim()
        .parse()
        .ok()
        .filter(|value: &f64| *value > 0.0)
        .ok_or_else(|| CliError::usage(format!("{:?} is not a positive amount", args.amount)))?;

    // Broadcasting is irreversible, so it takes an explicit --yes rather than
    // a prompt: a prompt cannot be answered by a script, and a script that
    // sends funds by accident is the failure worth designing against. Signing
    // without broadcasting moves nothing, so it does not ask.
    if !args.yes && !args.sign_only {
        return Err(CliError::usage(format!(
            "this broadcasts {} {} to {} — re-run with --yes",
            amount,
            chain.coin_symbol(),
            args.to
        )));
    }

    let password = signing_password(ctx, &wallet.id, args.password_file, args.password_env)?;
    let service = service_for_chain(
        ctx,
        chain,
        ENDPOINT_CAPABILITY_BALANCE
            | ENDPOINT_CAPABILITY_BROADCAST
            | ENDPOINT_CAPABILITY_FEE
            | ENDPOINT_CAPABILITY_UTXO,
    )?;
    service.set_secret_store(ctx.secrets.clone());
    ctx.rt.block_on(service.open_state(ctx.db_path()))?;
    let request = SendExecutionRequest {
        chain_id: chain.str_id().to_string(),
        wallet_id: wallet.id.clone(),
        password,
        to_address: args.to.clone(),
        amount_str: args.amount.trim().to_string(),
        contract_address: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: (args.gas_limit.is_some() || args.nonce.is_some()).then_some({
            EvmSendOverridesInput {
                nonce: args.nonce,
                custom_fees: None,
                gas_limit: args.gas_limit,
                calldata_hex: None,
                sign_only: None,
                access_list_json: None,
            }
        }),
        monero_priority: None,
        sign_only: args.sign_only,
    };

    out.text(|| {
        println!(
            "  {} {}…",
            out::hint("→"),
            if args.sign_only {
                "signing"
            } else {
                "signing and broadcasting"
            }
        )
    });
    let result = ctx
        .rt
        .block_on(service.execute_send(request))
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        if args.sign_only {
            println!("  {} signed, not broadcast", out::ok_mark());
        } else {
            println!("  {} broadcast", out::ok_mark());
        }
        if !result.transaction_hash.is_empty() {
            out::field("tx", &out::info(&result.transaction_hash).to_string());
        }
        if let Some(payload) = result.signed_payload.as_deref().filter(|p| !p.is_empty()) {
            out::field(
                "bytes",
                &(payload.trim_start_matches("0x").len() / 2).to_string(),
            );
            println!();
            println!("  {}", out::hint(payload));
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "signOnly": args.sign_only,
        "hash": result.transaction_hash,
        "signedPayload": result.signed_payload,
        "from": wallet.id,
        "to": args.to,
        "amount": amount,
        "symbol": chain.coin_symbol(),
    }));
    Ok(())
}

/// Build the transaction an EVM send would sign, and print it.
///
/// This exists because `prepare_evm_send_assembly` had only one caller — the
/// iOS send sheet — so no suite could see it. `is_supported_evm_chain` named
/// seven chains and `is_native_evm_asset` listed nine `(chain, symbol)` pairs,
/// two of which named a governance token: sixteen EVM mainnets could not
/// assemble at all, and ARB and OP assembled as the gas asset. Both were fixed
/// and neither was visible from a green suite until this command existed.
///
/// No key, no network and no store: this is a pure function over its
/// arguments, so it runs against an empty data directory.
pub fn assemble(_ctx: &Ctx, out: Out, args: AssembleArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    if !chain.is_evm() {
        return Err(CliError::rejected(format!(
            "{} is not an EVM chain; only EVM sends are assembled here",
            chain.chain_display_name()
        )));
    }

    // The typed decimal goes through untouched. Parsing it to an `f64` here
    // and letting the assembler shift that is what printed `--amount 1.1` as
    // 1100000000000000089 wei — 89 more than `spectra send broadcast` signs for
    // the same input, from the command whose whole job is showing what a send
    // would sign. The assembler validates it against the asset's precision.
    let amount = args.amount.trim().to_string();

    let symbol = args
        .symbol
        .clone()
        .unwrap_or_else(|| chain.coin_symbol().to_string());

    let token = match (&args.contract, args.decimals) {
        (Some(contract), Some(decimals)) => Some(EvmSupportedToken {
            symbol: symbol.clone(),
            contract_address: contract.clone(),
            decimals,
        }),
        (Some(_), None) => return Err(CliError::usage("--contract needs --decimals")),
        (None, Some(_)) => return Err(CliError::usage("--decimals needs --contract")),
        (None, None) => None,
    };

    let assembly = prepare_evm_send_assembly(EvmSendAssemblyInput {
        chain_id: chain.str_id().to_string(),
        symbol: symbol.clone(),
        from_address: args.from.clone(),
        resolved_destination: args.to.clone(),
        amount,
        token,
    })
    .map_err(|e| CliError::rejected(e.to_string()))?;

    out.text(|| {
        println!();
        out::field("chain", chain.chain_display_name());
        out::field("asset", &symbol);
        out::field(
            "kind",
            if assembly.is_native {
                "native value transfer"
            } else {
                "ERC-20 transfer"
            },
        );
        out::field("to", &assembly.to_address);
        out::field("value (wei)", &assembly.value_wei);
        out::field("data", &assembly.data_hex);
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.str_id(),
        "symbol": symbol,
        "isNative": assembly.is_native,
        "to": assembly.to_address,
        "valueWei": assembly.value_wei,
        "data": assembly.data_hex,
    }));
    Ok(())
}

fn staged_service(
    ctx: &Ctx,
    chain_id: &str,
    endpoints: Vec<String>,
) -> CliResult<std::sync::Arc<WalletService>> {
    if endpoints.is_empty() {
        return ctx.service();
    }
    let service = WalletService::new(vec![spectra_core::service::ChainEndpoints {
        capabilities: vec![
            "balance",
            "history",
            "utxo",
            "fee",
            "broadcast",
            "verification",
            "token-balance",
            "token-discovery",
            "token-history",
            "staking",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        chain_id: chain_id.into(),
        endpoints,
    }])?;
    service.set_secret_store(ctx.secrets.clone());
    ctx.rt.block_on(service.open_state(ctx.db_path()))?;
    Ok(service)
}
fn emit_artifact(out: Out, artifact: &spectra_core::send::stages::SendArtifact) {
    out.text(|| {
        println!(
            "{} {:?}\n{}\n{}",
            artifact.id, artifact.stage, artifact.review_digest, artifact.prepared_details
        );
    });
    out.emit(serde_json::json!({"artifact":artifact}));
}

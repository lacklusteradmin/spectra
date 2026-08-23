//! Transactions: what core has recorded, and putting a new one on a chain.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::send::ethereum::{
    prepare_evm_send_assembly, EvmSendAssemblyInput, EvmSupportedToken,
};
use spectra_core::send::SendExecutionRequest;
use spectra_core::store::wallet_domain::CoreTransactionKind;
use spectra_core::store::wallet_secrets;

use super::chain::{service_for_chain, BALANCE, BROADCAST, FEE, RPC, UTXO};
use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx, SecretSource};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Args)]
pub struct TxsArgs {
    /// Only this wallet's transactions (id, name or address).
    #[arg(long)]
    wallet: Option<String>,
}

/// Putting a transfer on a chain, and looking at one first.
///
/// `broadcast` is a subcommand rather than the bare verb on purpose: the
/// irreversible half of this tool should take a word that says so.
#[derive(Subcommand)]
pub enum SendCommand {
    /// Sign and broadcast a transfer.
    Broadcast(SendArgs),
    /// Build the transaction an EVM send would sign — no key, no network.
    Assemble(AssembleArgs),
}

pub fn run(ctx: &Ctx, out: Out, command: SendCommand) -> CliResult<()> {
    match command {
        SendCommand::Broadcast(args) => send(ctx, out, args),
        SendCommand::Assemble(args) => assemble(ctx, out, args),
    }
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
    let service = ctx.service()?;
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
                out::tint(&record.symbol, &record.chain_name).bold(),
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
                "chain": record.chain_name,
                "address": record.address,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

pub fn send(ctx: &Ctx, out: Out, args: SendArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.from)?;
    if wallet.is_watch_only {
        return Err(CliError::rejected("a watch-only wallet cannot send"));
    }
    let chain = resolve_chain(&wallet.chain_name)?;
    let derivation_path = wallet.derivation_path.clone().ok_or_else(|| {
        CliError::failure("this wallet has no derivation path stored, so it cannot be signed with")
    })?;

    let amount: f64 = args
        .amount
        .trim()
        .parse()
        .ok()
        .filter(|value: &f64| *value > 0.0)
        .ok_or_else(|| CliError::usage(format!("{:?} is not a positive amount", args.amount)))?;

    // Broadcasting is irreversible, so it takes an explicit --yes rather than
    // a prompt: a prompt cannot be answered by a script, and a script that
    // sends funds by accident is the failure worth designing against.
    if !args.yes {
        return Err(CliError::usage(format!(
            "this broadcasts {} {} to {} — re-run with --yes",
            amount,
            chain.coin_symbol(),
            args.to
        )));
    }

    let env = args
        .password_env
        .clone()
        .filter(|name| std::env::var_os(name).is_some());
    let password = SecretSource {
        file: args.password_file.clone(),
        env,
    }
    .resolve("password")?;
    let seed_phrase = wallet_secrets::unlock(ctx.secrets.as_ref(), &wallet.id, &password)?;

    let service = service_for_chain(chain, BALANCE | RPC | BROADCAST | FEE | UTXO)?;
    let request = SendExecutionRequest {
        chain_id: chain.str_id().to_string(),
        chain_name: wallet.chain_name.clone(),
        derivation_path,
        seed_phrase: Some(seed_phrase.to_string()),
        private_key_hex: None,
        from_address: wallet_address(&wallet).to_string(),
        to_address: args.to.clone(),
        amount,
        amount_str: Some(args.amount.trim().to_string()),
        contract_address: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        monero_priority: None,
        derivation_overrides: None,
    };

    out.text(|| println!("  {} signing and broadcasting…", out::hint("→")));
    let result = ctx
        .rt
        .block_on(service.execute_send(request))
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        println!("  {} broadcast", out::ok_mark());
        if !result.transaction_hash.is_empty() {
            out::field("tx", &out::info(&result.transaction_hash).to_string());
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "hash": result.transaction_hash,
        "from": wallet.id,
        "to": args.to,
        "amount": amount,
        "symbol": chain.coin_symbol(),
    }));
    Ok(())
}

/// Build the transaction an EVM send would sign, and print it.
///
/// This is the one funds-path rule the CLI could not reach. `is_supported_evm_chain`
/// named seven chains and `is_native_evm_asset` listed nine `(chain, symbol)`
/// pairs, two of which named a governance token — sixteen EVM mainnets could
/// not assemble at all, and ARB and OP assembled as the gas asset. Both are
/// fixed and both were invisible here, because the only caller of
/// `prepare_evm_send_assembly` is the iOS send sheet.
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

    let amount: f64 = args
        .amount
        .trim()
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| CliError::usage(format!("{:?} is not an amount", args.amount)))?;

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
        chain_name: chain.chain_display_name().to_string(),
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
        "chain": chain.chain_display_name(),
        "symbol": symbol,
        "isNative": assembly.is_native,
        "to": assembly.to_address,
        "valueWei": assembly.value_wei,
        "data": assembly.data_hex,
    }));
    Ok(())
}

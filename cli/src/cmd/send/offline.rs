//! The composer's arithmetic and parsing, with no key, store or network:
//! each is a pure function of its arguments, so it runs against an empty
//! data directory.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::send::ethereum::{
    EvmCustomFeeConfiguration, EvmSendAssemblyInput, EvmSendOverridesInput, EvmSupportedToken,
    parse_evm_custom_fees, parse_evm_nonce, prepare_evm_send_assembly,
};
use spectra_core::send::payment_memo::PaymentMemo;
use spectra_core::send::{SendAffordability, SendAffordabilityInput, send_affordability};

use super::MemoArgs;
use crate::cmd::resolve_chain;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AmountReport {
    chain: &'static str,
    decimals: u32,
    raw_amount: String,
}

impl Report for AmountReport {
    fn render(&self) {
        println!(
            "  {} integer units ({} decimals)",
            self.raw_amount, self.decimals
        );
    }
}

pub fn amount(out: Out, args: AmountArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let decimals = args.decimals.unwrap_or(u32::from(chain.native_decimals()));
    let raw = spectra_core::amount::input::parse_raw_amount(&args.amount, decimals)?;
    out.report(&AmountReport {
        chain: chain.str_id(),
        decimals,
        raw_amount: raw.to_string(),
    })
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

#[derive(Serialize)]
struct ShortcutReport {
    amount: String,
}

impl Report for ShortcutReport {
    fn render(&self) {
        println!("  {}", self.amount);
    }
}

pub fn shortcut(out: Out, args: ShortcutArgs) -> CliResult<()> {
    let amount = spectra_core::amount::input::send_amount_shortcut(
        args.maximum,
        args.decimals,
        args.percentage,
    )
    .ok_or_else(|| CliError::failure("no positive amount within the quoted maximum"))?;
    out.report(&ShortcutReport { amount })
}

#[derive(Args)]
pub struct FeeUnitsArgs {
    #[arg(long)]
    chain: String,
    #[arg(long, allow_hyphen_values = true)]
    amount: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FeeUnitsReport {
    raw_fee: String,
}

impl Report for FeeUnitsReport {
    fn render(&self) {
        println!("  {} native integer units", self.raw_fee);
    }
}

pub fn fee_units(out: Out, args: FeeUnitsArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let raw =
        spectra_core::send::payload::fee_units(&args.amount, u32::from(chain.native_decimals()))?;
    out.report(&FeeUnitsReport {
        raw_fee: raw.to_string(),
    })
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OverridesReport {
    nonce: Option<u64>,
    gas_limit: Option<u64>,
    calldata_bytes: Option<usize>,
    access_list_entries: usize,
    storage_keys: usize,
}

impl Report for OverridesReport {
    fn render(&self) {
        println!();
        let or_default = |value: Option<String>| value.unwrap_or_else(|| "network's".into());
        out::wide_field(12, "nonce", &or_default(self.nonce.map(|n| n.to_string())));
        out::wide_field(
            12,
            "gas limit",
            &or_default(self.gas_limit.map(|n| n.to_string())),
        );
        if let Some(bytes) = self.calldata_bytes {
            out::wide_field(12, "calldata", &format!("{bytes} bytes"));
        }
        out::wide_field(
            12,
            "access list",
            &format!(
                "{} entries, {} storage keys",
                self.access_list_entries, self.storage_keys
            ),
        );
    }
}

pub fn overrides(out: Out, args: OverridesArgs) -> CliResult<()> {
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
        ..Default::default()
    }
    .resolve(chain)?;
    out.report(&OverridesReport {
        nonce: resolved.nonce,
        gas_limit: resolved.gas_limit,
        calldata_bytes: resolved.calldata.as_ref().map(Vec::len),
        access_list_entries: resolved.access_list.len(),
        storage_keys: resolved
            .access_list
            .iter()
            .map(|entry| entry.storage_keys.len())
            .sum(),
    })
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

#[derive(Serialize)]
struct FeesReport {
    fees: EvmCustomFeeConfiguration,
}

impl Report for FeesReport {
    fn render(&self) {
        println!();
        out::wide_field(
            12,
            "max fee",
            &format!("{} gwei", self.fees.max_fee_per_gas_gwei),
        );
        out::wide_field(
            12,
            "priority fee",
            &format!("{} gwei", self.fees.max_priority_fee_per_gas_gwei),
        );
    }
}

pub fn fees(out: Out, args: FeesArgs) -> CliResult<()> {
    let fees = parse_evm_custom_fees(args.max_fee, args.priority_fee)
        .map_err(|error| CliError::rejected(error.to_string()))?;
    out.report(&FeesReport { fees })
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

/// Core's verdict on whether a send fits once its fee is counted.
#[derive(Serialize)]
#[serde(tag = "verdict", rename_all = "camelCase")]
enum AffordabilityReport {
    Unavailable,
    Affordable,
    #[serde(rename_all = "camelCase")]
    AmountPlusFeeExceedsBalance {
        symbol: String,
        required: String,
    },
    #[serde(rename_all = "camelCase")]
    AmountExceedsBalance {
        symbol: String,
    },
    #[serde(rename_all = "camelCase")]
    FeeExceedsGasBalance {
        gas_symbol: String,
        fee: String,
        chain: Chain,
    },
}

impl Report for AffordabilityReport {
    fn render(&self) {
        match self {
            Self::Unavailable => println!("  {}", out::hint("cannot tell whether the fee fits")),
            Self::Affordable => println!("  {} the send fits", out::ok_mark()),
            Self::AmountPlusFeeExceedsBalance { symbol, required } => println!(
                "  {} needs ~{required} {symbol} for the amount plus the fee",
                out::fail_mark()
            ),
            Self::AmountExceedsBalance { symbol } => {
                println!("  {} more {symbol} than the wallet holds", out::fail_mark())
            }
            Self::FeeExceedsGasBalance {
                gas_symbol,
                fee,
                chain,
            } => println!(
                "  {} not enough {gas_symbol} for the ~{fee} {} fee",
                out::fail_mark(),
                chain.chain_display_name()
            ),
        }
    }
}

/// Whether the asset is the chain's own, what the gas asset is called and how
/// many decimals a fee is quoted to are all read from the registry — naming
/// the chain is the whole input.
pub fn affordability(out: Out, args: AffordabilityArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let deployment_id = args
        .deployment
        .unwrap_or_else(|| chain.entry().native_deployment_id.clone());
    let token = spectra_core::catalog::tokens::deployment(&deployment_id)
        .ok_or_else(|| CliError::usage("unknown deployment"))?;
    if token.chain_id != chain || token.symbol != args.symbol {
        return Err(CliError::usage(
            "deployment does not match the selected network and symbol",
        ));
    }
    let verdict = send_affordability(SendAffordabilityInput {
        is_native: token.is_native(),
        chain_id: chain,
        symbol: args.symbol,
        amount: args.amount,
        network_fee: args.fee,
        holding_balance: args.balance,
        gas_balance: args.gas_balance,
    });
    out.report(&match verdict {
        SendAffordability::Unavailable => AffordabilityReport::Unavailable,
        SendAffordability::Affordable => AffordabilityReport::Affordable,
        SendAffordability::AmountPlusFeeExceedsBalance { symbol, required } => {
            AffordabilityReport::AmountPlusFeeExceedsBalance { symbol, required }
        }
        SendAffordability::AmountExceedsBalance { symbol } => {
            AffordabilityReport::AmountExceedsBalance { symbol }
        }
        SendAffordability::FeeExceedsGasBalance {
            gas_symbol,
            fee,
            chain_id,
        } => AffordabilityReport::FeeExceedsGasBalance {
            gas_symbol,
            fee,
            chain: chain_id,
        },
    })
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
    /// Ticker to print. Display only: the asset is `--contract`, or the gas
    /// asset without one.
    #[arg(long)]
    symbol: Option<String>,
    /// ERC-20 contract, when sending a token rather than the gas asset.
    #[arg(long)]
    contract: Option<String>,
    /// Token decimals. Required with --contract.
    #[arg(long)]
    decimals: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AssembleReport {
    chain: Chain,
    symbol: String,
    is_native: bool,
    to: String,
    value_wei: String,
    data: String,
}

impl Report for AssembleReport {
    fn render(&self) {
        println!();
        out::wide_field(11, "network", &out::chain(self.chain).to_string());
        out::wide_field(11, "asset", &self.symbol);
        out::wide_field(
            11,
            "kind",
            if self.is_native {
                "native value transfer"
            } else {
                "ERC-20 transfer"
            },
        );
        out::wide_field(11, "to", &self.to);
        out::wide_field(11, "value (wei)", &self.value_wei);
        out::wide_field(11, "data", &self.data);
    }
}

/// The transaction an EVM send would sign: the one place a suite can see
/// `prepare_evm_send_assembly`, which the iOS send sheet also calls.
pub fn assemble(out: Out, args: AssembleArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    if !chain.is_evm() {
        return Err(CliError::rejected(format!(
            "{} is not an EVM chain; only EVM sends are assembled here",
            chain.chain_display_name()
        )));
    }
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
    // The asset is its deployment: the contract's on this chain, or the gas
    // asset's when there is none. The ticker only labels the output.
    let deployment_id =
        spectra_core::catalog::tokens::deployment_id_for(chain, args.contract.as_deref())
            .ok_or_else(|| CliError::rejected("the contract is not valid on this chain"))?;
    // The typed decimal goes through untouched; the assembler checks it
    // against the asset's precision.
    let assembly = prepare_evm_send_assembly(EvmSendAssemblyInput {
        chain_id: chain,
        deployment_id,
        from_address: args.from,
        resolved_destination: args.to,
        amount: args.amount.trim().to_string(),
        token,
    })
    .map_err(|e| CliError::rejected(e.to_string()))?;
    out.report(&AssembleReport {
        chain,
        symbol,
        is_native: assembly.is_native,
        to: assembly.to_address,
        value_wei: assembly.value_wei,
        data: assembly.data_hex,
    })
}

/// The composer's scanner, without a camera.
#[derive(Args)]
pub struct ScanArgs {
    /// Chain the send is on. Required: a payload is only an address once a
    /// chain has judged it.
    #[arg(long)]
    chain: String,
    /// Contract of the token being sent; the network's coin when omitted. A
    /// code that asks for another asset is refused.
    #[arg(long)]
    token: Option<String>,
    /// The scanned text — a bare address or a payment URI.
    payload: String,
}

#[derive(Serialize)]
struct ScanReport {
    chain: &'static str,
    payload: String,
    address: String,
    amount: Option<String>,
    memo: Option<PaymentMemo>,
}

impl Report for ScanReport {
    fn render(&self) {
        println!();
        out::field("scanned", &self.payload);
        out::field("address", &self.address.bold().to_string());
        if let Some(amount) = &self.amount {
            out::field("amount", amount);
        }
        if let Some(memo) = &self.memo {
            out::field("memo", &memo.value);
        }
    }
}

/// The recipient, and the amount and memo the code asks for. Exit 3 when the
/// payload carries no address this chain accepts or asks for something else,
/// so a script can assert the refusal.
pub fn scan(out: Out, args: ScanArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let payment = spectra_core::send::scanned_payment::read_scanned_payment(
        chain,
        args.token,
        args.payload.clone(),
    )?;
    out.report(&ScanReport {
        chain: chain.str_id(),
        payload: args.payload,
        address: payment.address,
        amount: payment.amount,
        memo: payment.memo,
    })
}

/// The receive page's request, without a screen: what its QR code holds.
#[derive(Args)]
pub struct RequestArgs {
    /// Chain the address is on.
    #[arg(long)]
    chain: String,
    /// The receiving address.
    #[arg(long)]
    address: String,
    /// The amount of the network's coin to ask for.
    #[arg(long)]
    amount: Option<String>,
    #[command(flatten)]
    memo: MemoArgs,
}

#[derive(Serialize)]
struct RequestReport {
    chain: &'static str,
    uri: String,
}

impl Report for RequestReport {
    fn render(&self) {
        println!();
        println!("  {}", self.uri);
    }
}

/// Exit 3 when the network has no request format, or the address, amount or
/// memo is not one it takes.
pub fn request(out: Out, args: RequestArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let uri = spectra_core::send::scanned_payment::payment_request_uri(
        chain,
        args.address,
        args.amount,
        args.memo.memo(),
    )?;
    out.report(&RequestReport {
        chain: chain.str_id(),
        uri,
    })
}

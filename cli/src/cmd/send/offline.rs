//! The composer's arithmetic and parsing, with no key, store or network:
//! each is a pure function of its arguments, so it runs against an empty
//! data directory.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::send::ethereum::{
    EvmCustomFeeConfiguration, EvmSendOverridesInput, parse_evm_custom_fees, parse_evm_nonce,
};
use spectra_core::send::payment_memo::PaymentMemo;

use super::MemoArgs;
use crate::cmd::resolve_chain;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

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

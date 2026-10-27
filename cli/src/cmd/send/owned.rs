//! Sending a holding the store tracks: core picks the route, fees and
//! balances from the wallet, as the app's send page does.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::send::SendPreflight;
use spectra_core::service::send_review::{OwnedSendReview, SendReviewInput};
use spectra_core::service::{MoneroSyncStatus, OwnedSendPreview};

use super::{ArtifactReport, BroadcastReport, MemoArgs};
use crate::cmd::resolve_chain;
use crate::ctx::{Ctx, PasswordArgs, WalletArg};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Args)]
pub struct OwnedSendArgs {
    /// Wallet id, name or address.
    wallet: String,
    #[arg(long)]
    holding: String,
    #[arg(long)]
    amount: String,
    #[arg(long)]
    destination: String,
    #[command(flatten)]
    memo: MemoArgs,
}

impl OwnedSendArgs {
    fn input(self, ctx: &Ctx) -> CliResult<SendReviewInput> {
        Ok(SendReviewInput {
            wallet_id: ctx.find_wallet(&self.wallet)?.id,
            holding_key: self.holding,
            amount: self.amount,
            destination: self.destination,
            overrides: None,
            memo: self.memo.memo(),
        })
    }
}

#[derive(Args)]
pub struct OwnedBroadcastArgs {
    #[command(flatten)]
    send: OwnedSendArgs,
    #[arg(long)]
    yes: bool,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct ReviewArgs {
    /// Wallet id, name or address.
    wallet: String,
    #[arg(long)]
    holding: String,
    #[arg(long)]
    amount: String,
    #[arg(long)]
    destination: String,
}

#[derive(Args)]
pub struct PreviewArgs {
    /// Wallet id, name or address.
    wallet: String,
    #[arg(long)]
    holding: String,
    #[arg(long)]
    amount: String,
    #[arg(long, default_value = "")]
    destination: String,
}

#[derive(Args)]
pub struct IdentityArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Defaults to the wallet's chain; EVM wallets may select another shared-address chain.
    #[arg(long)]
    chain: Option<String>,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct SyncMoneroArgs {
    #[command(flatten)]
    identity: IdentityArgs,
    /// Scan one durable batch instead of continuing to the chain tip.
    #[arg(long)]
    once: bool,
}

/// What a send of a holding would be, checked without the network.
#[derive(Serialize)]
struct PreflightReport {
    preflight: SendPreflight,
}

impl Report for PreflightReport {
    fn render(&self) {
        let preflight = &self.preflight;
        println!();
        out::field("network", &out::chain(preflight.chain).to_string());
        out::field("to", &preflight.normalized_destination_address);
        out::field(
            "amount",
            &format!("{} {}", preflight.amount.bold(), preflight.symbol),
        );
        if let Some(contract) = &preflight.token_contract_address {
            out::field("contract", contract);
        }
    }
}

pub fn review(ctx: &Ctx, out: Out, args: ReviewArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let preflight = ctx.rt.block_on(ctx.service()?.send_submit_preflight(
        wallet.id,
        args.holding,
        args.destination,
        args.amount,
    ))?;
    out.report(&PreflightReport { preflight })
}

/// The composer's live quote: the fee, what can be sent, and why an amount
/// cannot be.
#[derive(Serialize)]
struct PreviewReport {
    preview: Option<OwnedSendPreview>,
}

impl Report for PreviewReport {
    fn render(&self) {
        println!();
        let Some(quote) = &self.preview else {
            println!("  {}", out::hint("no quote for this amount"));
            return;
        };
        let symbol = quote.chain_id.coin_symbol();
        let preview = &quote.preview;
        out::wide_field(12, "amount", &quote.amount.bold().to_string());
        out::wide_field(
            12,
            "fee",
            &format!("{} {symbol}", preview.estimated_network_fee),
        );
        if let Some(total) = &quote.total {
            out::wide_field(12, "total", total);
        }
        out::wide_field(12, "spendable", &preview.spendable_balance);
        out::wide_field(12, "max sendable", &preview.max_sendable);
        if let Some(rate) = &preview.fee_rate_description {
            out::wide_field(12, "fee rate", rate);
        }
        if let Some(check) = &quote.recipient
            && check.is_own_address
        {
            out::wide_field(12, "recipient", "one of your addresses");
        }
        if let Some(refusal) = &quote.amount_refusal {
            println!("  {} {}", out::fail_mark(), refusal);
        }
    }
}

pub fn preview(ctx: &Ctx, out: Out, args: PreviewArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let preview = ctx.rt.block_on(ctx.service()?.preview_owned_send(
        wallet.id,
        args.holding,
        args.amount,
        args.destination,
        None,
        None,
    ))?;
    out.report(&PreviewReport { preview })
}

/// A send resolved and quoted, with what must be confirmed before it goes.
#[derive(Serialize)]
struct QuoteReport {
    quote: OwnedSendReview,
}

impl Report for QuoteReport {
    fn render(&self) {
        let quote = &self.quote;
        let request = &quote.request;
        println!();
        out::field("network", &out::chain(request.chain_id).to_string());
        out::field("to", &request.to_address);
        out::field("amount", &request.amount_str.bold().to_string());
        if let Some(preview) = &quote.preview {
            out::field(
                "fee",
                &format!(
                    "{} {}",
                    preview.estimated_network_fee,
                    request.chain_id.coin_symbol()
                ),
            );
        }
        for warning in &quote.warnings {
            out::field("warning", &super::warning_text(warning));
        }
        for warning in &quote.recipient_warnings {
            out::field("warning", &super::recipient_warning_text(warning));
        }
        if quote.requires_self_send_confirmation {
            out::field("confirm", "the recipient is one of your own addresses");
        }
        out::field("review", &quote.id);
    }
}

pub fn quote(ctx: &Ctx, out: Out, args: OwnedSendArgs) -> CliResult<()> {
    let input = args.input(ctx)?;
    let quote = ctx.rt.block_on(ctx.service()?.review_owned_send(input))?;
    out.report(&QuoteReport { quote })
}

pub fn build(ctx: &Ctx, out: Out, args: OwnedSendArgs) -> CliResult<()> {
    let input = args.input(ctx)?;
    let artifact = ctx.rt.block_on(ctx.service()?.build_owned_send(input))?;
    out.report(&ArtifactReport { artifact })
}

pub fn broadcast(ctx: &Ctx, out: Out, args: OwnedBroadcastArgs) -> CliResult<()> {
    if !args.yes {
        return Err(CliError::usage("owned-broadcast requires --yes"));
    }
    let wallet = ctx.find_wallet(&args.send.wallet)?;
    let password = args.password.for_wallet(&wallet)?;
    let input = args.send.input(ctx)?;
    let service = ctx.service()?;
    let review = ctx.rt.block_on(service.review_owned_send(input.clone()))?;
    let result = ctx
        .rt
        .block_on(service.execute_owned_send(review.id, input, password))?;
    out.report(&BroadcastReport {
        transaction_hash: result.transaction_hash,
    })
}

/// The address a wallet signs as on a network, read from its own key.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentityReport {
    wallet: String,
    chain: Chain,
    address: String,
}

impl Report for IdentityReport {
    fn render(&self) {
        println!();
        out::field("network", &out::chain(self.chain).to_string());
        out::field("sender", &self.address);
    }
}

pub fn identity(ctx: &Ctx, out: Out, args: IdentityArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = match args.chain.as_deref() {
        Some(name) => resolve_chain(name)?,
        None => wallet.chain_id,
    };
    let password = args.password.for_wallet(&wallet)?;
    let address = ctx.rt.block_on(ctx.service()?.send_identity_address(
        wallet.id.clone(),
        chain,
        password,
    ))?;
    out.report(&IdentityReport {
        wallet: wallet.id,
        chain,
        address,
    })
}

/// Where a Monero wallet's local scan stands.
#[derive(Serialize)]
struct MoneroSyncReport {
    #[serde(skip)]
    chain: Chain,
    sync: Option<MoneroSyncStatus>,
}

impl Report for MoneroSyncReport {
    fn render(&self) {
        println!();
        let Some(sync) = &self.sync else {
            println!("  {}", out::hint("not scanned yet"));
            return;
        };
        out::wide_field(
            9,
            "scanned",
            &format!("{} / {}", sync.scanned_height, sync.target_height),
        );
        out::wide_field(
            9,
            "unlocked",
            &format!(
                "{} {}",
                spectra_core::amount::decimal::from_units(
                    u128::from(sync.unlocked_piconeros),
                    u32::from(self.chain.native_decimals())
                ),
                self.chain.coin_symbol()
            ),
        );
        out::wide_field(
            9,
            "state",
            if sync.complete {
                "complete"
            } else {
                "scanning"
            },
        );
    }
}

pub fn monero_status(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let sync = ctx
        .rt
        .block_on(ctx.service()?.monero_sync_status(wallet.id.clone()))?;
    out.report(&MoneroSyncReport {
        chain: wallet.chain_id,
        sync,
    })
}

pub fn sync_monero(ctx: &Ctx, out: Out, args: SyncMoneroArgs) -> CliResult<()> {
    let identity = args.identity;
    let wallet = ctx.find_wallet(&identity.wallet)?;
    if let Some(chain) = identity.chain.as_deref()
        && resolve_chain(chain)? != wallet.chain_id
    {
        return Err(CliError::usage(
            "Monero sync must use the wallet's selected network",
        ));
    }
    let password = identity.password.for_wallet(&wallet)?;
    let service = ctx.service()?;
    loop {
        let status = ctx
            .rt
            .block_on(service.sync_monero_wallet(wallet.id.clone(), password.clone()))?;
        if args.once || status.complete {
            return out.report(&MoneroSyncReport {
                chain: wallet.chain_id,
                sync: Some(status),
            });
        }
        out::progress(&format!(
            "Monero scan: {} / {}",
            status.scanned_height, status.target_height
        ));
    }
}

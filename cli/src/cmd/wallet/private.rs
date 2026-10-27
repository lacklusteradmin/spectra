//! Funds only this device can see: a Zcash wallet's shielded pools and a
//! Litecoin wallet's MWEB outputs, found by scanning blocks here, and the
//! transactions that move them.

use clap::Args;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::{LitecoinMwebStatus, ZcashShieldedStatus};

use crate::cmd::send::ArtifactReport;
use crate::ctx::{Ctx, PasswordArgs, WalletArg};
use crate::error::CliResult;
use crate::out::{self, Out, Report};

/// A wallet whose private funds this device scans for.
#[derive(Args)]
pub struct ScanSyncArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Scan one batch and stop.
    #[arg(long)]
    once: bool,
    #[command(flatten)]
    password: PasswordArgs,
}

#[derive(Args)]
pub struct SendShieldedArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The recipient: a unified, Sapling or transparent Zcash address.
    #[arg(long)]
    to: String,
    /// How much ZEC, as an exact decimal.
    #[arg(long)]
    amount: String,
    /// A memo for a shielded recipient, at most 512 bytes.
    #[arg(long)]
    memo: Option<String>,
}

#[derive(Args)]
pub struct MwebPeginArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// How much LTC arrives in MWEB, as an exact decimal.
    #[arg(long)]
    amount: String,
}

#[derive(Args)]
pub struct SendMwebArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The recipient: an MWEB address, or any other Litecoin address for a
    /// peg-out.
    #[arg(long)]
    to: String,
    /// How much LTC, as an exact decimal.
    #[arg(long)]
    amount: String,
}

#[derive(Serialize)]
struct ShieldedReport {
    #[serde(skip)]
    chain: Chain,
    shielded: ZcashShieldedStatus,
}

impl Report for ShieldedReport {
    fn render(&self) {
        let status = &self.shielded;
        let symbol = self.chain.coin_symbol();
        println!();
        out::wide_field(
            10,
            "scanned",
            &format!("{} / {}", status.scanned_height, status.chain_tip_height),
        );
        out::wide_field(10, "spendable", &format!("{} {symbol}", status.spendable));
        out::wide_field(10, "pending", &format!("{} {symbol}", status.pending));
        out::wide_field(10, "shieldable", &format!("{} {symbol}", status.shieldable));
        if let Some(address) = &status.address {
            out::wide_field(10, "address", address);
        }
    }
}

pub fn zcash_sync(ctx: &Ctx, out: Out, args: ScanSyncArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let password = args.password.for_wallet(&wallet)?;
    let service = ctx.service()?;
    // Each batch moves the scan on or finishes it.
    loop {
        let shielded = ctx
            .rt
            .block_on(service.sync_zcash_shielded(wallet.id.clone(), password.clone()))?;
        if args.once || shielded.complete {
            return out.report(&ShieldedReport {
                chain: wallet.chain_id,
                shielded,
            });
        }
        out::progress(&format!(
            "Zcash scan: {} / {} ({}‰), {} transactions to read",
            shielded.scanned_height,
            shielded.chain_tip_height,
            shielded.progress_permille,
            shielded.unread_transactions
        ));
    }
}

pub fn zcash_status(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let shielded = ctx
        .rt
        .block_on(ctx.service()?.zcash_shielded_status(wallet.id))?;
    out.report(&ShieldedReport {
        chain: wallet.chain_id,
        shielded,
    })
}

pub fn shield(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx
        .rt
        .block_on(ctx.service()?.build_zcash_shielding(wallet.id))?;
    out.report(&ArtifactReport { artifact })
}

pub fn send_shielded(ctx: &Ctx, out: Out, args: SendShieldedArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(ctx.service()?.build_zcash_shielded_send(
        wallet.id,
        args.to,
        args.amount,
        args.memo,
    ))?;
    out.report(&ArtifactReport { artifact })
}

#[derive(Serialize)]
struct MwebReport {
    #[serde(skip)]
    chain: Chain,
    mweb: LitecoinMwebStatus,
}

impl Report for MwebReport {
    fn render(&self) {
        let status = &self.mweb;
        let symbol = self.chain.coin_symbol();
        println!();
        out::wide_field(
            10,
            "scanned",
            &format!(
                "block {} ({}‰)",
                status.scanned_height, status.progress_permille
            ),
        );
        out::wide_field(10, "spendable", &format!("{} {symbol}", status.spendable));
        out::wide_field(10, "pending", &format!("{} {symbol}", status.pending));
        if let Some(address) = &status.address {
            out::wide_field(10, "address", address);
        }
    }
}

pub fn mweb_sync(ctx: &Ctx, out: Out, args: ScanSyncArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let password = args.password.for_wallet(&wallet)?;
    let service = ctx.service()?;
    // Each batch moves the scan on or finishes it.
    loop {
        let mweb = ctx
            .rt
            .block_on(service.sync_litecoin_mweb(wallet.id.clone(), password.clone()))?;
        if args.once || mweb.complete {
            return out.report(&MwebReport {
                chain: wallet.chain_id,
                mweb,
            });
        }
        out::progress(&format!(
            "MWEB scan: block {} ({}‰)",
            mweb.scanned_height, mweb.progress_permille
        ));
    }
}

pub fn mweb_status(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let mweb = ctx
        .rt
        .block_on(ctx.service()?.litecoin_mweb_status(wallet.id))?;
    out.report(&MwebReport {
        chain: wallet.chain_id,
        mweb,
    })
}

pub fn mweb_pegin(ctx: &Ctx, out: Out, args: MwebPeginArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(
        ctx.service()?
            .build_litecoin_mweb_pegin(wallet.id, args.amount),
    )?;
    out.report(&ArtifactReport { artifact })
}

pub fn send_mweb(ctx: &Ctx, out: Out, args: SendMwebArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(ctx.service()?.build_litecoin_mweb_send(
        wallet.id,
        args.to,
        args.amount,
    ))?;
    out.report(&ArtifactReport { artifact })
}

//! Transactions core has recorded locally, and keeping their status current.
//! Distinct from `history`, which asks the chain.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::{
    EndpointHolder, HistoryPage, HistoryQuery, HistoryQueryFilter, PendingMaintenanceResult,
    ReplaceableSend, TransactionEndpoints, TransactionSnapshot,
};
use spectra_core::store::TransactionStatusChange;
use spectra_core::store::persistence_models::HistorySource;
use spectra_core::store::persistence_models::TransactionRecord;
use spectra_core::store::wallet_domain::{
    TransactionDirection, TransactionKind, transaction_amount_direction,
};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Subcommand)]
pub enum TxsCommand {
    /// Every stored transaction, or one wallet's.
    List(WalletFilterArgs),
    /// The recent and pending projection and indexed history aggregates.
    Summary,
    /// One stored transaction.
    Show(TransactionIdArgs),
    /// The two ends of one stored transaction, and which is the wallet's own.
    Endpoints(TransactionIdArgs),
    /// A bounded, deduplicated page of stored history.
    Page(PageArgs),
    /// Recheck one stored UTXO transaction, failed or confirmed records
    /// included.
    Recheck(RecheckArgs),
    /// Poll every stored transaction network and persist status changes.
    RefreshPending,
    /// The chains whose stored transactions still need polling.
    Maintenance,
    /// Poll one chain's pending transactions and persist status changes.
    Poll(PollArgs),
    /// The pending sends that can still be replaced on their chain.
    Replaceable(WalletFilterArgs),
}

#[derive(Args)]
pub struct WalletFilterArgs {
    /// Only this wallet's transactions (id, name or address).
    #[arg(long)]
    wallet: Option<String>,
}

impl WalletFilterArgs {
    fn wallet_id(&self, ctx: &Ctx) -> CliResult<Option<String>> {
        self.wallet
            .as_deref()
            .map(|needle| ctx.find_wallet(needle).map(|wallet| wallet.id))
            .transpose()
    }
}

#[derive(Args)]
pub struct TransactionIdArgs {
    /// The stored transaction's id.
    id: String,
}

#[derive(Args)]
pub struct PageArgs {
    #[command(flatten)]
    wallet: WalletFilterArgs,
    #[arg(long, default_value_t = 20)]
    limit: u32,
    /// Continue with the nextCursor returned by the preceding page.
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long, default_value = "")]
    search: String,
    #[arg(long)]
    oldest_first: bool,
    #[arg(long, default_value = "all", value_parser = ["all", "send", "receive", "pending"])]
    filter: String,
    /// Leave out transfers below 0.00001 of their asset (zero-value and dust).
    #[arg(long)]
    hide_small_amounts: bool,
}

#[derive(Args)]
pub struct RecheckArgs {
    /// The stored transaction's id.
    id: String,
    /// Explicit read endpoint for the transaction's stored network.
    #[arg(long)]
    endpoint: Option<String>,
}

#[derive(Args)]
pub struct PollArgs {
    /// The stored transaction network to poll.
    chain: String,
}

fn stored(ctx: &Ctx, id: String) -> CliResult<TransactionRecord> {
    ctx.rt
        .block_on(ctx.service()?.transaction(id))?
        .ok_or_else(|| CliError::rejected("no such transaction"))
}

pub fn txs(ctx: &Ctx, out: Out, command: TxsCommand) -> CliResult<()> {
    match command {
        TxsCommand::List(args) => {
            let service = ctx.service()?;
            let records = match args.wallet_id(ctx)? {
                Some(wallet_id) => ctx
                    .rt
                    .block_on(service.transactions_for_wallet(wallet_id))?,
                None => ctx.rt.block_on(service.transactions())?,
            };
            out.report(&TransactionsReport {
                count: records.len(),
                transactions: records.iter().map(TransactionRow::from).collect(),
            })
        }
        TxsCommand::Summary => {
            let summary = ctx.rt.block_on(ctx.service()?.transaction_snapshot())?;
            out.report(&SummaryReport { summary })
        }
        TxsCommand::Show(args) => out.report(&RecordReport {
            record: stored(ctx, args.id)?,
        }),
        TxsCommand::Endpoints(args) => {
            let endpoints = ctx
                .rt
                .block_on(ctx.service()?.transaction_endpoints(args.id))?
                .ok_or_else(|| CliError::rejected("no such transaction"))?;
            out.report(&EndpointsReport { endpoints })
        }
        TxsCommand::Page(args) => {
            let page = ctx.rt.block_on(ctx.service()?.history_page(HistoryQuery {
                wallet_id: args.wallet.wallet_id(ctx)?,
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
                hide_small_amounts: args.hide_small_amounts,
            }))?;
            out.report(&PageReport { page })
        }
        TxsCommand::Recheck(args) => {
            let service = match args.endpoint {
                Some(endpoint) => {
                    let chain = stored(ctx, args.id.clone())?.chain_id;
                    ctx.service_with_endpoints(chain, vec![endpoint])?
                }
                None => ctx.service()?,
            };
            let change = ctx
                .rt
                .block_on(service.recheck_transaction_status(args.id))?;
            out.report(&ChangeReport { change })
        }
        TxsCommand::RefreshPending => {
            let maintenance = ctx
                .rt
                .block_on(ctx.service()?.refresh_pending_transactions())?;
            out.report(&MaintenanceReport { maintenance })
        }
        TxsCommand::Maintenance => {
            let chains = ctx
                .rt
                .block_on(ctx.service()?.pending_maintenance_chains())?;
            out.report(&PendingChainsReport { chains })
        }
        TxsCommand::Poll(args) => {
            // A maintenance request names the stored transaction network: the
            // poll targets only it, through its configured endpoints, the
            // user's saved ones included.
            let chain = resolve_chain(&args.chain)?;
            let changes = ctx
                .rt
                .block_on(ctx.service()?.poll_pending_transactions(chain))?;
            out.report(&ChangesReport { changes })
        }
        TxsCommand::Replaceable(args) => replaceable(ctx, out, args.wallet_id(ctx)?),
    }
}

// ─── Rendering a record ─────────────────────────────────────────────────────

/// One recorded transaction on a line: its direction, amount and the
/// counterparty, with the hash beneath.
fn render_line(
    kind: TransactionKind,
    amount: &str,
    symbol: &str,
    chain: Chain,
    address: &str,
    hash: Option<&str>,
) {
    let mark = match transaction_amount_direction(kind, amount) {
        TransactionDirection::Incoming => out::up("↓"),
        TransactionDirection::Outgoing => out::down("↑"),
        TransactionDirection::Neutral => out::hint("↔"),
    };
    println!(
        "  {}  {:>14} {}  {}",
        mark.bold(),
        amount,
        out::tint(symbol, chain).bold(),
        out::hint(address),
    );
    if let Some(hash) = hash {
        println!("     {}", out::hint(&out::short_hash(hash)));
    }
}

fn render_record_line(record: &TransactionRecord) {
    render_line(
        record.kind,
        &record.amount,
        &record.symbol,
        record.chain_id,
        &record.address,
        record.transaction_hash.as_deref(),
    );
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransactionRow {
    #[serde(skip)]
    direction: TransactionKind,
    hash: Option<String>,
    kind: &'static str,
    amount: String,
    symbol: String,
    chain: Chain,
    address: String,
    history_source: Option<HistorySource>,
}

impl From<&TransactionRecord> for TransactionRow {
    fn from(record: &TransactionRecord) -> Self {
        Self {
            direction: record.kind,
            hash: record.transaction_hash.clone(),
            kind: record.kind.as_raw(),
            amount: record.amount.clone(),
            symbol: record.symbol.clone(),
            chain: record.chain_id,
            address: record.address.clone(),
            history_source: record.history_source.clone(),
        }
    }
}

#[derive(Serialize)]
struct TransactionsReport {
    count: usize,
    transactions: Vec<TransactionRow>,
}

impl Report for TransactionsReport {
    fn render(&self) {
        println!();
        if self.transactions.is_empty() {
            println!("  {}", out::hint("nothing recorded"));
            return;
        }
        for row in &self.transactions {
            render_line(
                row.direction,
                &row.amount,
                &row.symbol,
                row.chain,
                &row.address,
                row.hash.as_deref(),
            );
        }
        println!();
        println!("  {}", out::count(self.count, "transaction"));
    }
}

// ─── Reports ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SummaryReport {
    summary: TransactionSnapshot,
}

impl Report for SummaryReport {
    fn render(&self) {
        let summary = &self.summary;
        println!();
        for record in &summary.recent_and_pending {
            render_record_line(record);
        }
        println!();
        println!(
            "  {}, {} pending, {} replaceable",
            out::count(summary.total_count as usize, "transaction"),
            summary.pending_count,
            summary.replaceable.len()
        );
    }
}

#[derive(Serialize)]
struct EndpointsReport {
    endpoints: TransactionEndpoints,
}

impl Report for EndpointsReport {
    fn render(&self) {
        println!();
        for (label, end) in [("from", &self.endpoints.from), ("to", &self.endpoints.to)] {
            let Some(end) = end else { continue };
            let mine = if end.is_mine { "  (this wallet)" } else { "" };
            let holder = match &end.holder {
                Some(EndpointHolder::Wallet { name }) => format!("  wallet {name}"),
                Some(EndpointHolder::Contact { name }) => format!("  contact {name}"),
                None => String::new(),
            };
            println!("  {label:<4} {}{mine}{holder}", end.address);
        }
    }
}

/// One stored transaction with what its detail sheet offers.
#[derive(Serialize)]
struct RecordReport {
    record: TransactionRecord,
}

impl Report for RecordReport {
    fn render(&self) {
        let record = &self.record;
        println!();
        render_record_line(record);
        println!();
        out::field("status", record.status.as_raw());
        out::field("network", &out::chain(record.chain_id).to_string());
        out::field("wallet", &record.wallet_name);
        if let Some(fee) = record
            .confirmed_network_fee
            .as_ref()
            .or(record.receipt_network_fee.as_ref())
        {
            out::field("fee", fee);
        }
        if let Some(confirmations) = record.confirmation_count {
            out::field("confirms", &confirmations.to_string());
        }
        if let Some(failure) = &record.failure_reason {
            out::field("failure", &out::serde_name(failure));
        }
        out::field("id", &out::hint(&record.id).to_string());
    }
}

#[derive(Serialize)]
struct PageReport {
    page: HistoryPage,
}

impl Report for PageReport {
    fn render(&self) {
        println!();
        if self.page.records.is_empty() {
            println!("  {}", out::hint("nothing recorded"));
        }
        for record in &self.page.records {
            render_record_line(record);
        }
        if let Some(cursor) = &self.page.next_cursor {
            println!();
            out::field("next", &format!("--cursor {cursor}"));
        }
    }
}

fn render_change(change: &TransactionStatusChange) {
    let status = if change.status_changed {
        format!(
            "{} → {}",
            change.old_status.as_raw(),
            change.new_status.as_raw().bold()
        )
    } else {
        change.new_status.as_raw().to_string()
    };
    println!(
        "  {}  {}  {}",
        out::chain(change.chain_id),
        status,
        out::hint(&out::short_hash(
            change.transaction_hash.as_deref().unwrap_or(&change.id)
        ))
    );
}

#[derive(Serialize)]
struct ChangeReport {
    change: TransactionStatusChange,
}

impl Report for ChangeReport {
    fn render(&self) {
        render_change(&self.change);
    }
}

#[derive(Serialize)]
struct ChangesReport {
    changes: Vec<TransactionStatusChange>,
}

impl Report for ChangesReport {
    fn render(&self) {
        println!();
        for change in &self.changes {
            render_change(change);
        }
        println!("  {}", out::count(self.changes.len(), "status change"));
    }
}

#[derive(Serialize)]
struct MaintenanceReport {
    maintenance: PendingMaintenanceResult,
}

impl Report for MaintenanceReport {
    fn render(&self) {
        let maintenance = &self.maintenance;
        println!();
        for change in &maintenance.changes {
            render_change(change);
        }
        for failure in &maintenance.failures {
            println!(
                "  {} {}  {}",
                out::fail_mark(),
                out::chain(failure.chain_id),
                out::hint(&failure.message)
            );
        }
        println!(
            "  {} polled, {}",
            out::count(maintenance.chains.len(), "network"),
            out::count(maintenance.changes.len(), "change")
        );
    }

    fn failure(&self) -> Option<String> {
        (!self.maintenance.failures.is_empty())
            .then(|| "pending maintenance failed for one or more networks".to_string())
    }
}

#[derive(Serialize)]
struct PendingChainsReport {
    chains: Vec<Chain>,
}

impl Report for PendingChainsReport {
    fn render(&self) {
        println!();
        if self.chains.is_empty() {
            println!("  {}", out::hint("nothing pending"));
        }
        for chain in &self.chains {
            println!("  {}", out::chain(*chain));
        }
    }
}

// ─── Replaceable sends ──────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceableRow {
    transaction: String,
    wallet: String,
    chain: Chain,
    symbol: String,
    amount: String,
    to: String,
    hash: String,
    nonce: Option<i64>,
    can_speed_up: bool,
}

impl From<ReplaceableSend> for ReplaceableRow {
    fn from(send: ReplaceableSend) -> Self {
        Self {
            transaction: send.transaction_id,
            wallet: send.wallet_id,
            chain: send.chain_id,
            symbol: send.symbol,
            amount: send.amount,
            to: send.to_address,
            hash: send.transaction_hash,
            nonce: send.recorded_nonce,
            can_speed_up: send.can_speed_up,
        }
    }
}

#[derive(Serialize)]
struct ReplaceableReport {
    count: usize,
    replaceable: Vec<ReplaceableRow>,
}

impl Report for ReplaceableReport {
    fn render(&self) {
        println!();
        if self.replaceable.is_empty() {
            println!("  {}", out::hint("nothing to replace"));
            return;
        }
        for send in &self.replaceable {
            render_line(
                TransactionKind::Send,
                &send.amount,
                &send.symbol,
                send.chain,
                &send.to,
                Some(&send.hash),
            );
            let how = if send.can_speed_up {
                "speed up or cancel"
            } else {
                "cancel only"
            };
            let line = match send.nonce {
                Some(nonce) => format!("nonce {nonce} · {how}"),
                None => how.to_string(),
            };
            println!("     {}", out::hint(&line));
        }
    }
}

/// The pending sends core says can still be replaced, and how: an EVM chain,
/// a send, still pending, with a hash to read its nonce by.
fn replaceable(ctx: &Ctx, out: Out, wallet_id: Option<String>) -> CliResult<()> {
    let replaceable: Vec<ReplaceableRow> = ctx
        .rt
        .block_on(ctx.service()?.replaceable_sends())?
        .into_iter()
        .filter(|send| wallet_id.as_deref().is_none_or(|id| send.wallet_id == id))
        .map(ReplaceableRow::from)
        .collect();
    out.report(&ReplaceableReport {
        count: replaceable.len(),
        replaceable,
    })
}

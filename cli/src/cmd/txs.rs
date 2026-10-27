//! Transactions core has recorded locally, and keeping their status current.
//! Distinct from `history`, which asks the chain.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::{
    EndpointHolder, HistoryPage, HistoryQuery, HistoryQueryFilter, PendingMaintenanceResult,
    ReplaceableSend, TransactionEndpoints, TransactionSnapshot,
};
use spectra_core::store::TransactionStatusChange;
use spectra_core::store::persistence_models::TransactionRecord;
use spectra_core::store::transaction_merge::{HistorySource, history_source};
use spectra_core::store::wallet_domain::{
    TransactionDirection, TransactionKind, transaction_amount_direction,
};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

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
    /// Leave out transfers below 0.00001 of their asset (zero-value and dust).
    #[arg(long, requires = "page")]
    hide_small_amounts: bool,

    /// Explicit read endpoint for the rechecked transaction's stored network.
    #[arg(long, requires = "recheck")]
    endpoint: Option<String>,
    /// Recheck one stored UTXO transaction, including failed or confirmed records.
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

pub fn txs(ctx: &Ctx, out: Out, args: TxsArgs) -> CliResult<()> {
    if args.summary {
        let summary = ctx.rt.block_on(ctx.service()?.transaction_snapshot())?;
        return out.report(&SummaryReport { summary });
    }
    if let Some(id) = args.endpoints {
        let endpoints = ctx
            .rt
            .block_on(ctx.service()?.transaction_endpoints(id))?
            .ok_or_else(|| CliError::rejected("no such transaction"))?;
        return out.report(&EndpointsReport { endpoints });
    }
    if let Some(id) = args.record {
        let record = ctx
            .rt
            .block_on(ctx.service()?.transaction(id))?
            .ok_or_else(|| CliError::rejected("no such transaction"))?;
        return out.report(&RecordReport { record });
    }
    if args.page {
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
            hide_small_amounts: args.hide_small_amounts,
        }))?;
        return out.report(&PageReport { page });
    }
    if let Some(id) = args.recheck {
        let service = ctx.service()?;
        if let Some(endpoint) = args.endpoint {
            let chain = ctx
                .rt
                .block_on(service.transaction(id.clone()))?
                .ok_or_else(|| CliError::rejected("no such transaction"))?
                .chain_id;
            ctx.rt.block_on(service.update_endpoints(vec![
                spectra_core::service::ChainEndpoints {
                    capabilities: spectra_core::api::endpoint::EndpointCapability::ALL.to_vec(),
                    chain_id: chain,
                    endpoints: vec![endpoint],
                },
            ]))?;
        }
        let change = ctx.rt.block_on(service.recheck_transaction_status(id))?;
        return out.report(&ChangeReport { change });
    }
    if args.refresh_pending {
        let maintenance = ctx
            .rt
            .block_on(ctx.service()?.refresh_pending_transactions())?;
        let failed = !maintenance.failures.is_empty();
        out.report(&MaintenanceReport { maintenance })?;
        if failed {
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
        return out.report(&PendingChainsReport { chains });
    }
    if let Some(name) = &args.poll_chain {
        // A maintenance request names the stored transaction network. The
        // catalog transport keeps persisted custom nodes available; the poll
        // itself still targets only this stored network.
        let chain = resolve_chain(name)?;
        let changes = ctx
            .rt
            .block_on(ctx.service()?.poll_pending_transactions(chain))?;
        return out.report(&ChangesReport { changes });
    }
    if args.replaceable {
        return replaceable(ctx, out, args.wallet.as_deref());
    }
    let service = ctx.service()?;
    let records = match &args.wallet {
        Some(needle) => {
            let wallet = ctx.find_wallet(needle)?;
            ctx.rt
                .block_on(service.transactions_for_wallet(wallet.id))?
        }
        None => ctx.rt.block_on(service.transactions())?,
    };
    out.report(&TransactionsReport {
        count: records.len(),
        transactions: records.iter().map(TransactionRow::from).collect(),
    })
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
    let mark = match transaction_amount_direction(kind, amount.to_string()) {
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

/// What the detail sheet's "History Source" row shows, as the app reads it:
/// a proper noun, a chain's provider set, or Spectra's own reader, which the
/// row does not name.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum SourceView {
    Provider(String),
    ChainProviders(Chain),
    Internal,
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
    history_source: Option<SourceView>,
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
            history_source: record
                .transaction_history_source
                .clone()
                .and_then(history_source)
                .map(|source| match source {
                    HistorySource::Provider { name } => SourceView::Provider(name),
                    HistorySource::ChainProviders { chain_id } => {
                        SourceView::ChainProviders(chain_id)
                    }
                    HistorySource::Internal => SourceView::Internal,
                }),
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

    fn ok(&self) -> bool {
        self.maintenance.failures.is_empty()
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
fn replaceable(ctx: &Ctx, out: Out, wallet: Option<&str>) -> CliResult<()> {
    let wallet_id = wallet
        .map(|needle| ctx.find_wallet(needle).map(|wallet| wallet.id))
        .transpose()?;
    let replaceable: Vec<ReplaceableRow> = ctx
        .rt
        .block_on(ctx.service()?.replaceable_sends())?
        .into_iter()
        .filter(|send| {
            wallet_id
                .as_deref()
                .is_none_or(|id| send.wallet_id.eq_ignore_ascii_case(id))
        })
        .map(ReplaceableRow::from)
        .collect();
    out.report(&ReplaceableReport {
        count: replaceable.len(),
        replaceable,
    })
}

//! Putting a transaction on a chain, and everything a send asks first.
//!
//! `stages` is the staged flow every wallet operation shares: build, review,
//! sign, broadcast. `owned` sends a stored holding through core's own
//! routing; `recipient` checks where a send would go; `offline` is the
//! composer's arithmetic and parsing, with no key, store or network.

mod offline;
mod owned;
mod recipient;
mod stages;

use clap::{Args, Subcommand};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::PaymentMemoKind;
use spectra_core::send::flow::{EvmRecipientPreflightWarning, HighRiskSendWarning};
use spectra_core::send::payment_memo::PaymentMemo;
use spectra_core::send::stages::{SendArtifact, SendStage, SubmissionOutcome, WalletOperation};

use crate::ctx::Ctx;
use crate::error::CliResult;
use crate::out::{self, Out, Report};

/// The XRP destination tag or Stellar memo a payment carries to a
/// recipient that shares its account. Core checks it against the network.
#[derive(Args, Clone)]
#[group(multiple = false)]
pub struct MemoArgs {
    /// XRP Ledger destination tag, a whole number.
    #[arg(long, value_name = "TAG")]
    destination_tag: Option<String>,
    /// Stellar text memo, up to 28 bytes.
    #[arg(long, value_name = "TEXT")]
    memo_text: Option<String>,
    /// Stellar ID memo, a whole number.
    #[arg(long, value_name = "ID")]
    memo_id: Option<String>,
}

impl MemoArgs {
    pub fn memo(self) -> Option<PaymentMemo> {
        let (kind, value) = match (self.destination_tag, self.memo_text, self.memo_id) {
            (Some(tag), ..) => (PaymentMemoKind::DestinationTag, tag),
            (_, Some(text), _) => (PaymentMemoKind::MemoText, text),
            (.., Some(id)) => (PaymentMemoKind::MemoId, id),
            _ => return None,
        };
        Some(PaymentMemo { kind, value })
    }
}

/// Putting a transfer on a chain, and looking at one first.
///
/// `broadcast-signed` names the irreversible step on purpose: the half of
/// this tool that cannot be undone takes a word that says so.
#[derive(Subcommand)]
pub enum SendCommand {
    /// Build and persist the exact transaction for review, without signing.
    Build(stages::BuildArgs),
    /// List prepared and signed transactions that survive app restarts.
    List,
    /// Inspect a persisted prepared or signed transaction.
    Inspect { transaction_id: String },
    /// Sign exactly the reviewed transaction and persist it without broadcasting.
    Sign(stages::SignArgs),
    /// Broadcast the same saved signed payload to explicitly selected endpoints.
    BroadcastSigned(stages::BroadcastArgs),
    /// Resubmit the signed payload of a stored transaction.
    Rebroadcast {
        transaction_id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Build a replacement or cancellation from a stored pending transaction; never signs.
    Replacement {
        transaction_id: String,
        #[arg(long)]
        cancel: bool,
    },
    /// Show the actual configured endpoint table offline, in service order.
    ConfiguredEndpoints { chain: String },
    /// Check a stored holding can be sent offline; never signs.
    Review(owned::ReviewArgs),
    /// Quote a stored holding on its selected network; never signs.
    Preview(owned::PreviewArgs),
    /// Resolve an owned send request and fees, without signing or broadcasting.
    Quote(owned::OwnedSendArgs),
    /// Build a transaction for a tracked holding, persisting it with its risk review.
    BuildOwned(owned::OwnedSendArgs),
    /// Send a stored holding using core-owned routing, fees and balances.
    OwnedBroadcast(owned::OwnedBroadcastArgs),
    /// Resolve the stored sender and check its signing identity offline.
    Identity(owned::IdentityArgs),
    /// Inspect durable local Monero scan progress without contacting a node.
    MoneroStatus(crate::ctx::WalletArg),
    /// Scan Monero locally, from the restore height the wallet was imported
    /// with. No view or spend key is sent to the daemon.
    SyncMonero(owned::SyncMoneroArgs),
    /// Whether a destination is one of the user's own addresses on the holding's network.
    SelfCheck(recipient::SelfCheckArgs),
    /// Ask what a recipient address looks like before sending to it.
    Probe(recipient::ProbeArgs),
    /// Resolve what was typed into the address a send would go to.
    Destination(recipient::DestinationArgs),
    /// Validate exact decimal input and show integer units, without keys or network.
    Amount(offline::AmountArgs),
    /// Convert a fee-adjusted estimate to a conservative decimal shortcut offline.
    Shortcut(offline::ShortcutArgs),
    /// Validate a fee or gas budget in native units, without keys or network.
    FeeUnits(offline::FeeUnitsArgs),
    /// Build the transaction an EVM send would sign — no key, no network.
    Assemble(offline::AssembleArgs),
    /// Read the address, amount and memo a scanned QR payload asks for, offline.
    Scan(offline::ScanArgs),
    /// The payment code a receiving address shows to ask for an amount, offline.
    Request(offline::RequestArgs),
    /// Ask whether a send can land once the fee is counted.
    Affordability(offline::AffordabilityArgs),
    /// Validate custom EVM gas fees in gwei, without keys or network.
    Fees(offline::FeesArgs),
    /// Validate EVM nonce, gas, calldata and access-list overrides offline.
    Overrides(offline::OverridesArgs),
}

pub fn run(ctx: &Ctx, out: Out, command: SendCommand) -> CliResult<()> {
    match command {
        SendCommand::Build(args) => stages::build(ctx, out, args),
        SendCommand::List => stages::list(ctx, out),
        SendCommand::Inspect { transaction_id } => stages::inspect(ctx, out, transaction_id),
        SendCommand::Sign(args) => stages::sign(ctx, out, args),
        SendCommand::BroadcastSigned(args) => stages::broadcast(ctx, out, args),
        SendCommand::Rebroadcast {
            transaction_id,
            yes,
        } => stages::rebroadcast(ctx, out, transaction_id, yes),
        SendCommand::Replacement {
            transaction_id,
            cancel,
        } => stages::replacement(ctx, out, transaction_id, cancel),
        SendCommand::ConfiguredEndpoints { chain } => {
            stages::configured_endpoints(ctx, out, &chain)
        }
        SendCommand::Review(args) => owned::review(ctx, out, args),
        SendCommand::Preview(args) => owned::preview(ctx, out, args),
        SendCommand::Quote(args) => owned::quote(ctx, out, args),
        SendCommand::BuildOwned(args) => owned::build(ctx, out, args),
        SendCommand::OwnedBroadcast(args) => owned::broadcast(ctx, out, args),
        SendCommand::Identity(args) => owned::identity(ctx, out, args),
        SendCommand::MoneroStatus(args) => owned::monero_status(ctx, out, args),
        SendCommand::SyncMonero(args) => owned::sync_monero(ctx, out, args),
        SendCommand::SelfCheck(args) => recipient::self_check(ctx, out, args),
        SendCommand::Probe(args) => recipient::probe(ctx, out, args),
        SendCommand::Destination(args) => recipient::destination(ctx, out, args),
        SendCommand::Amount(args) => offline::amount(out, args),
        SendCommand::Shortcut(args) => offline::shortcut(out, args),
        SendCommand::FeeUnits(args) => offline::fee_units(out, args),
        SendCommand::Assemble(args) => offline::assemble(out, args),
        SendCommand::Scan(args) => offline::scan(out, args),
        SendCommand::Request(args) => offline::request(out, args),
        SendCommand::Affordability(args) => offline::affordability(out, args),
        SendCommand::Fees(args) => offline::fees(out, args),
        SendCommand::Overrides(args) => offline::overrides(out, args),
    }
}

// ─── Reports ────────────────────────────────────────────────────────────────

/// A staged transaction: what `send build`, `sign` and `broadcast-signed`
/// answer, and every wallet operation and staking action that builds one.
#[derive(Serialize)]
pub struct ArtifactReport {
    pub artifact: SendArtifact,
}

impl Report for ArtifactReport {
    fn render(&self) {
        render_artifact(&self.artifact);
    }
}

/// The staged transactions that survive a restart.
#[derive(Serialize)]
pub struct ArtifactsReport {
    artifacts: Vec<SendArtifact>,
}

impl Report for ArtifactsReport {
    fn render(&self) {
        println!();
        if self.artifacts.is_empty() {
            println!("  {}", out::hint("nothing staged"));
            return;
        }
        for artifact in &self.artifacts {
            println!(
                "  {:<8}  {} {}  {}  {}",
                stage_name(&artifact.stage),
                artifact.amount.bold(),
                out::tint(&artifact.symbol, artifact.chain_id),
                out::chain(artifact.chain_id),
                out::hint(&artifact.id),
            );
        }
    }
}

fn stage_name(stage: &SendStage) -> &'static str {
    match stage {
        SendStage::Prepared => "prepared",
        SendStage::Signed => "signed",
    }
}

fn render_artifact(artifact: &SendArtifact) {
    let symbol = &artifact.symbol;
    println!();
    out::wide_field(9, "stage", stage_name(&artifact.stage));
    out::wide_field(9, "network", &out::chain(artifact.chain_id).to_string());
    out::wide_field(9, "from", &artifact.sender);
    out::wide_field(9, "to", &artifact.recipient);
    out::wide_field(9, "amount", &format!("{} {symbol}", artifact.amount.bold()));
    if let Some(memo) = &artifact.memo {
        out::wide_field(9, "memo", &memo.value);
    }
    if let Some(staking) = &artifact.staking {
        out::wide_field(9, "staking", &out::serde_name(&staking.action));
        if let Some(validator) = &staking.validator_id {
            out::wide_field(9, "validator", validator);
        }
    }
    if let Some(operation) = &artifact.operation {
        render_operation(artifact, operation);
    } else if let Some(fee) = &artifact.review.network_fee {
        out::wide_field(9, "fee", fee);
    }
    if let Some(total) = &artifact.review.total {
        out::wide_field(9, "total", &format!("{total} {symbol}"));
    }
    for warning in &artifact.review.warnings {
        out::wide_field(9, "warning", &warning_text(warning));
    }
    for warning in &artifact.review.recipient_warnings {
        out::wide_field(9, "warning", &recipient_warning_text(warning));
    }
    if let Some(hash) = &artifact.transaction_hash {
        out::wide_field(9, "hash", hash);
    }
    for attempt in &artifact.attempts {
        let outcome = match attempt.outcome {
            SubmissionOutcome::Accepted => out::up("accepted"),
            SubmissionOutcome::Rejected => out::down("rejected"),
            SubmissionOutcome::Uncertain => out::hint("uncertain"),
        };
        out::wide_field(9, "sent to", &format!("{}  {outcome}", attempt.endpoint));
    }
    out::wide_field(9, "id", &artifact.id);
    out::wide_field(9, "review", &artifact.review_digest.bold().to_string());
    if !artifact.prepared_details.is_empty() {
        println!();
        for line in artifact.prepared_details.lines() {
            println!("  {}", out::hint(line));
        }
    }
}

/// Core's warning about a send, worded for the terminal.
pub fn warning_text(warning: &HighRiskSendWarning) -> String {
    use HighRiskSendWarning as W;
    match warning {
        W::InvalidFormat { chain } => {
            format!("not a {} address", chain.chain_display_name())
        }
        W::NewAddress => "nothing has been sent to this address before".into(),
        W::EnsResolved { name, address } => format!("{name} resolved to {address}"),
        W::LargeSend { percent, symbol } => format!("{percent}% of the {symbol} held"),
        W::NonEvmOnEvm { chain } => format!(
            "shaped like another network's address, on {}",
            chain.chain_display_name()
        ),
        W::EnsOffEthereum { chain } => format!(
            "an ENS name, on {}, whose names ENS does not resolve",
            chain.chain_display_name()
        ),
        W::EthOnUtxo { chain } => format!("an Ethereum address, on {}", chain.chain_display_name()),
        W::ForeignAddressFormat { chain } => format!(
            "an address format {} does not use",
            chain.chain_display_name()
        ),
        W::ChainMismatch => "the address belongs to another network".into(),
    }
}

/// Core's warning about an EVM recipient, worded for the terminal.
pub fn recipient_warning_text(warning: &EvmRecipientPreflightWarning) -> String {
    use EvmRecipientPreflightWarning as W;
    match warning {
        W::RecipientIsContract { symbol, .. } => {
            format!("the recipient is a contract, which may not handle {symbol}")
        }
        W::RecipientCodeUnknown { .. } => "whether the recipient is a contract is unknown".into(),
        W::TokenContractMissing { token_symbol, .. } => {
            format!("no contract is deployed at the {token_symbol} address")
        }
        W::TokenCodeUnknown { token_symbol, .. } => {
            format!("whether the {token_symbol} contract exists is unknown")
        }
    }
}

/// What a wallet operation does, beside the transfer fields every artifact has.
fn render_operation(artifact: &SendArtifact, operation: &WalletOperation) {
    let symbol = artifact.chain_id.coin_symbol();
    let field = |label: &str, value: &str| out::wide_field(9, label, value);
    let fee = |fee: &str| field("fee", &format!("{fee} {symbol}"));
    match operation {
        WalletOperation::RevokeApproval {
            token,
            spender,
            network_fee,
        } => {
            field("revokes", token);
            field("spender", spender);
            fee(network_fee);
        }
        WalletOperation::CloseAccount {
            destination,
            reserve,
            removed_objects,
            network_fee,
        } => {
            println!(
                "  {}  closing deletes {} on the network; this cannot be undone",
                out::accent("!").bold(),
                artifact.sender
            );
            field("into", destination);
            field("reserve", &format!("{reserve} {symbol}"));
            field("objects", &removed_objects.to_string());
            fee(network_fee);
        }
        WalletOperation::CloseTokenAccounts {
            accounts,
            rent,
            network_fee,
        } => {
            for account in accounts {
                field("closes", account);
            }
            field("rent", &format!("{rent} {symbol}"));
            fee(network_fee);
        }
        WalletOperation::MergeCoins {
            coin_type,
            objects,
            network_fee,
        } => {
            field("merges", &format!("{objects} objects of {coin_type}"));
            fee(network_fee);
        }
        WalletOperation::TrustAsset {
            asset,
            reserve,
            network_fee,
        } => {
            field("trusts", asset);
            field("reserve", &format!("{reserve} {symbol}"));
            fee(network_fee);
        }
        WalletOperation::RemoveTrustLine {
            asset,
            reserve,
            network_fee,
        } => {
            field("untrusts", asset);
            field("frees", &format!("{reserve} {symbol}"));
            fee(network_fee);
        }
        WalletOperation::TransferNft {
            contract,
            standard,
            token_id,
            quantity,
            collection,
            network_fee,
        } => {
            field("token", &format!("{collection} #{token_id}"));
            field("standard", standard.label());
            field("contract", contract);
            field("quantity", quantity);
            fee(network_fee);
        }
        WalletOperation::ShieldTransparent {
            amount,
            network_fee,
        } => {
            field("shields", &format!("{amount} {symbol}"));
            fee(network_fee);
        }
        WalletOperation::ShieldedPayment { memo, network_fee } => {
            if let Some(memo) = memo {
                field("memo", memo);
            }
            fee(network_fee);
        }
        WalletOperation::DeleteAccessKey {
            public_key,
            receiver,
            network_fee,
        } => {
            field("deletes", public_key);
            field("calls", receiver);
            fee(network_fee);
        }
        WalletOperation::RefundTokenStorage {
            contract,
            refund,
            network_fee,
        } => {
            field("leaves", contract);
            field("refund", &format!("{refund} {symbol}"));
            fee(network_fee);
        }
    }
}

/// A transaction handed to the network, by its hash.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastReport {
    transaction_hash: String,
}

impl Report for BroadcastReport {
    fn render(&self) {
        println!("  {} broadcast {}", out::ok_mark(), self.transaction_hash);
    }
}

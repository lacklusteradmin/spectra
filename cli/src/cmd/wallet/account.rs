//! What a wallet's account on its network holds beyond its balance, and the
//! operations that change it. Each operation builds a staged transaction:
//! sign and broadcast it with `send sign` and `send broadcast-signed`.

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::service::{
    EmptyTokenAccounts, MoneroPaymentProof, NearAccessKeys, NearStorageDeposits, SuiCoinType,
    TokenApprovals, WalletNetworkAccount, WalletNfts, WalletTrustLines,
};

use crate::cmd::send::{ArtifactReport, MemoArgs};
use crate::ctx::{Ctx, WalletArg};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Args)]
pub struct RevokeArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The token contract.
    #[arg(long)]
    token: String,
    /// The spender whose allowance goes to zero.
    #[arg(long)]
    spender: String,
}

#[derive(Args)]
pub struct SendNftArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The collection's contract.
    #[arg(long)]
    contract: String,
    /// The token's id, a whole number in decimal.
    #[arg(long)]
    token_id: String,
    /// How many of an ERC-1155 token; an ERC-721 token is one.
    #[arg(long, default_value = "1")]
    quantity: String,
    /// The address that receives it.
    #[arg(long)]
    to: String,
}

#[derive(Args)]
pub struct CloseArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The existing account that receives everything.
    #[arg(long)]
    to: String,
    #[command(flatten)]
    memo: MemoArgs,
}

#[derive(Args)]
pub struct TrustArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The asset: `CODE.rIssuer` on the XRP Ledger, `CODE:ISSUER` on
    /// Stellar.
    #[arg(long)]
    asset: String,
}

#[derive(Args)]
pub struct DeleteKeyArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The function-call key, as `ed25519:…`.
    #[arg(long)]
    key: String,
}

#[derive(Args)]
pub struct RefundStorageArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The token contract, such as `usdt.tether-token.near`.
    #[arg(long)]
    contract: String,
}

#[derive(Args)]
pub struct MergeArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The coin type, such as `0x2::sui::SUI`.
    #[arg(long)]
    coin_type: String,
}

#[derive(Args)]
pub struct CloseTokenAccountsArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// A token account to close; repeat for several. None: every closable
    /// empty one.
    #[arg(long = "account")]
    accounts: Vec<String>,
}

#[derive(Args)]
pub struct ProvePaymentArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// The transaction's id.
    #[arg(long)]
    txid: String,
}

// ─── Reading the account ────────────────────────────────────────────────────

/// The account's network-specific state, each field as its network names it.
#[derive(Serialize)]
struct AccountReport {
    symbol: String,
    closable: bool,
    account: serde_json::Value,
}

impl Report for AccountReport {
    fn render(&self) {
        println!();
        out::wide_field(22, "symbol", &self.symbol);
        out::wide_field(22, "closable", &self.closable.to_string());
        if let Some(fields) = self.account.as_object() {
            for (name, field) in fields {
                let text = field
                    .as_str()
                    .map_or_else(|| field.to_string(), str::to_string);
                out::wide_field(22, name, &text);
            }
        }
    }
}

pub fn account(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let WalletNetworkAccount {
        symbol,
        account,
        closable,
        ..
    } = ctx
        .rt
        .block_on(ctx.service()?.wallet_network_account(wallet.id))?;
    out.report(&AccountReport {
        symbol,
        closable,
        account: serde_json::to_value(&account)
            .map_err(|error| CliError::failure(error.to_string()))?,
    })
}

#[derive(Serialize)]
struct EnsReport {
    name: Option<String>,
}

impl Report for EnsReport {
    fn render(&self) {
        match &self.name {
            Some(name) => println!("  {}", name.bold()),
            None => println!("  {}", out::hint("no primary name")),
        }
    }
}

pub fn ens(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let name = ctx.rt.block_on(ctx.service()?.wallet_ens_name(wallet.id))?;
    out.report(&EnsReport { name })
}

#[derive(Serialize)]
struct ApprovalsReport {
    approvals: TokenApprovals,
}

impl Report for ApprovalsReport {
    fn render(&self) {
        println!();
        if self.approvals.approvals.is_empty() {
            println!("  {}", out::hint("no allowance stands"));
        }
        for approval in &self.approvals.approvals {
            println!(
                "  {} {}  {}  {}",
                out::info(&approval.token),
                approval.symbol,
                out::hint(&approval.spender),
                if approval.unlimited {
                    "unlimited"
                } else {
                    &approval.allowance
                }
            );
        }
        if !self.approvals.complete {
            println!(
                "  {}",
                out::hint("the indexer scan stopped short; more may stand")
            );
        }
    }
}

pub fn approvals(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let approvals = ctx
        .rt
        .block_on(ctx.service()?.wallet_token_approvals(wallet.id))?;
    out.report(&ApprovalsReport { approvals })
}

#[derive(Serialize)]
struct NftsReport {
    nfts: WalletNfts,
}

impl Report for NftsReport {
    fn render(&self) {
        println!();
        if self.nfts.nfts.is_empty() {
            println!("  {}", out::hint("no NFTs held"));
        }
        for nft in &self.nfts.nfts {
            println!(
                "  {:<8} {}  #{}  ×{}  {}",
                nft.standard.label(),
                out::info(&nft.contract),
                nft.token_id,
                nft.quantity,
                nft.name.as_deref().unwrap_or(&nft.collection)
            );
        }
        if !self.nfts.complete {
            println!(
                "  {}",
                out::hint("the inventory was not read to its end; more may be held")
            );
        }
    }
}

pub fn nfts(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let nfts = ctx.rt.block_on(ctx.service()?.wallet_nfts(wallet.id))?;
    out.report(&NftsReport { nfts })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrustLinesReport {
    trust_lines: WalletTrustLines,
}

impl Report for TrustLinesReport {
    fn render(&self) {
        let lines = &self.trust_lines;
        println!();
        for line in &lines.lines {
            println!(
                "  {:<10} {}  {} of {}  {}",
                line.code,
                line.issuer,
                line.balance,
                line.limit,
                line.removal_blocked.as_deref().unwrap_or("removable")
            );
        }
        out::wide_field(16, "reserve per line", &lines.reserve_per_line);
    }
}

pub fn trust_lines(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let trust_lines = ctx
        .rt
        .block_on(ctx.service()?.wallet_trust_lines(wallet.id))?;
    out.report(&TrustLinesReport { trust_lines })
}

#[derive(Serialize)]
struct KeysReport {
    keys: NearAccessKeys,
}

impl Report for KeysReport {
    fn render(&self) {
        println!();
        for key in &self.keys.keys {
            let access = match &key.receiver {
                None => "full access".to_string(),
                Some(receiver) => format!(
                    "calls {receiver}{}, allowance {}",
                    if key.method_names.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", key.method_names.join(", "))
                    },
                    key.allowance.as_deref().unwrap_or("unlimited")
                ),
            };
            let mark = if key.signs {
                " (Spectra signs with it)"
            } else {
                ""
            };
            println!("  {}  {access}{mark}", key.public_key);
        }
    }
}

pub fn keys(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let keys = ctx
        .rt
        .block_on(ctx.service()?.wallet_access_keys(wallet.id))?;
    out.report(&KeysReport { keys })
}

#[derive(Serialize)]
struct StorageReport {
    #[serde(skip)]
    symbol: &'static str,
    storage: NearStorageDeposits,
}

impl Report for StorageReport {
    fn render(&self) {
        let symbol = self.symbol;
        println!();
        for deposit in &self.storage.deposits {
            println!(
                "  {}  {:<8} {} {symbol}",
                deposit.contract, deposit.symbol, deposit.refund
            );
        }
        out::wide_field(
            10,
            "refundable",
            &format!("{} {symbol}", self.storage.refundable),
        );
    }
}

pub fn token_storage(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let storage = ctx
        .rt
        .block_on(ctx.service()?.wallet_token_storage(wallet.id))?;
    out.report(&StorageReport {
        symbol: wallet.chain_id.coin_symbol(),
        storage,
    })
}

#[derive(Serialize)]
struct ObjectsReport {
    types: Vec<SuiCoinType>,
}

impl Report for ObjectsReport {
    fn render(&self) {
        println!();
        for entry in &self.types {
            println!(
                "  {:<8} {:>6} objects  {}  {}",
                entry.symbol, entry.objects, entry.balance, entry.coin_type
            );
        }
    }
}

pub fn objects(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let types = ctx
        .rt
        .block_on(ctx.service()?.wallet_coin_objects(wallet.id))?;
    out.report(&ObjectsReport { types })
}

#[derive(Serialize)]
struct TokenAccountsReport {
    #[serde(skip)]
    symbol: &'static str,
    empty: EmptyTokenAccounts,
}

impl Report for TokenAccountsReport {
    fn render(&self) {
        let symbol = self.symbol;
        println!();
        for account in &self.empty.accounts {
            println!(
                "  {}  {:<8} {} {symbol}  {}",
                account.address,
                account.symbol,
                account.rent,
                account.blocked.as_deref().unwrap_or("closable")
            );
        }
        out::wide_field(
            11,
            "reclaimable",
            &format!("{} {symbol}", self.empty.reclaimable),
        );
    }
}

pub fn token_accounts(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let empty = ctx
        .rt
        .block_on(ctx.service()?.wallet_empty_token_accounts(wallet.id))?;
    out.report(&TokenAccountsReport {
        symbol: wallet.chain_id.coin_symbol(),
        empty,
    })
}

#[derive(Serialize)]
struct ProofReport {
    #[serde(skip)]
    symbol: &'static str,
    proof: MoneroPaymentProof,
}

impl Report for ProofReport {
    fn render(&self) {
        let proof = &self.proof;
        println!();
        println!(
            "  check_tx_key {} {} {}",
            proof.txid, proof.tx_key, proof.address
        );
        println!(
            "  {}",
            out::hint(&format!("proves {} {} received", proof.amount, self.symbol))
        );
    }
}

pub fn prove_payment(ctx: &Ctx, out: Out, args: ProvePaymentArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let proof = ctx
        .rt
        .block_on(ctx.service()?.monero_payment_proof(wallet.id, args.txid))?
        .ok_or_else(|| CliError::usage("this device did not sign that Monero transaction"))?;
    out.report(&ProofReport {
        symbol: wallet.chain_id.coin_symbol(),
        proof,
    })
}

// ─── Operations ─────────────────────────────────────────────────────────────

pub fn revoke(ctx: &Ctx, out: Out, args: RevokeArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(ctx.service()?.build_approval_revocation(
        wallet.id,
        args.token,
        args.spender,
    ))?;
    out.report(&ArtifactReport { artifact })
}

pub fn send_nft(ctx: &Ctx, out: Out, args: SendNftArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(ctx.service()?.build_nft_transfer(
        wallet.id,
        args.contract,
        args.token_id,
        args.quantity,
        args.to,
    ))?;
    out.report(&ArtifactReport { artifact })
}

pub fn close(ctx: &Ctx, out: Out, args: CloseArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(ctx.service()?.build_account_closing(
        wallet.id,
        args.to,
        args.memo.memo(),
    ))?;
    out.report(&ArtifactReport { artifact })
}

/// Build the trust line change `wallet trust` or `wallet untrust` asks for.
pub fn trust(ctx: &Ctx, out: Out, args: TrustArgs, remove: bool) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let service = ctx.service()?;
    let artifact = ctx.rt.block_on(async {
        if remove {
            service.build_remove_trust_line(wallet.id, args.asset).await
        } else {
            service.build_trust_asset(wallet.id, args.asset).await
        }
    })?;
    out.report(&ArtifactReport { artifact })
}

pub fn delete_key(ctx: &Ctx, out: Out, args: DeleteKeyArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(
        ctx.service()?
            .build_access_key_deletion(wallet.id, args.key),
    )?;
    out.report(&ArtifactReport { artifact })
}

pub fn refund_storage(ctx: &Ctx, out: Out, args: RefundStorageArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(
        ctx.service()?
            .build_token_storage_refund(wallet.id, args.contract),
    )?;
    out.report(&ArtifactReport { artifact })
}

pub fn merge(ctx: &Ctx, out: Out, args: MergeArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx
        .rt
        .block_on(ctx.service()?.build_coin_merge(wallet.id, args.coin_type))?;
    out.report(&ArtifactReport { artifact })
}

pub fn close_token_accounts(ctx: &Ctx, out: Out, args: CloseTokenAccountsArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let artifact = ctx.rt.block_on(
        ctx.service()?
            .build_token_account_closure(wallet.id, args.accounts),
    )?;
    out.report(&ArtifactReport { artifact })
}

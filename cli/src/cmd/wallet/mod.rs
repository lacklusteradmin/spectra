//! Wallets enter through `WalletService::import_wallets` and change through
//! `StateCommand` — never by assembling a `WalletState` here, which would
//! skip every rule core applies on the way in, address validation included.
//!
//! `setup` answers what the setup page asks before a wallet exists;
//! `create` adds one; `show` reads one; `change` edits or deletes one;
//! `secrets` uses its keys; `account` reads and builds the network-specific
//! operations of its account; `private` scans and spends Zcash's shielded
//! and Litecoin's MWEB funds.

mod account;
mod change;
mod create;
mod private;
mod secrets;
mod setup;
mod show;

use std::collections::BTreeMap;

use clap::Subcommand;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::store::state::{WalletSigning, WalletState};

use crate::ctx::{Ctx, WalletArg, wallet_address};
use crate::error::CliResult;
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum WalletCommand {
    /// Validate password and confirmation from environment variables; never print either.
    CheckPassword(setup::CheckPasswordArgs),
    /// Judge a seed phrase from an environment variable as the import page
    /// does: its length and wordlist are inferred unless given. Prints the
    /// verdict, never the phrase.
    CheckSeed(setup::CheckSeedArgs),
    /// The ways a wallet can be added on a network, and what each accepts.
    Methods(setup::ChainArg),
    /// What a wallet on a network will do, its limits, and the endpoints its
    /// first refresh reads from. Contacts nothing.
    Capabilities(setup::ChainArg),
    /// The least a new account must receive to exist, read from the network
    /// (XRP and Stellar).
    Reserve(setup::ChainArg),
    /// Generate a new wallet and its seed phrase.
    New(create::NewArgs),
    /// Import a wallet from a seed phrase or raw private key.
    Import(create::ImportArgs),
    /// Track addresses, an account public key, or a Monero wallet's address
    /// with its view key, without the keys that spend.
    Watch(create::WatchArgs),
    /// The networks a wallet's key, or watched address, can be added to.
    CopyTargets(WalletArg),
    /// Add a wallet's key, or watched address, to another network as a
    /// wallet of its own, sealed under the same password.
    Copy(create::CopyArgs),
    /// List stored wallets.
    List,
    /// Show one wallet in detail.
    Show(WalletArg),
    /// What a wallet's page offers, with what each action does, and its
    /// network's capabilities and limits. Contacts nothing.
    Actions(WalletArg),
    /// Show a wallet's receive address.
    Receive(WalletArg),
    /// The addresses an account-discovery UTXO wallet has handed out and the
    /// coins they hold, read from the network.
    Coins(WalletArg),
    /// Core-derived portfolio and signing capabilities of every wallet.
    Derived,
    /// Rename a wallet.
    Rename(change::RenameArgs),
    /// Include or exclude a wallet from portfolio totals.
    Inclusion(change::InclusionArgs),
    /// Hide one of a wallet's holdings from its total and the portfolio; it
    /// stays sendable.
    Hide(change::HoldingArgs),
    /// Show a hidden holding again.
    Unhide(change::HoldingArgs),
    /// Delete a wallet, its history and its secrets.
    Delete(change::DeleteArgs),
    /// Decrypt and print a wallet's seed phrase or private key, or with
    /// `--key` one of its keys as other wallets import it.
    Export(secrets::ExportArgs),
    /// Sign a plain-text message with a wallet's key, in its network's
    /// scheme, to prove it holds its address.
    SignMessage(secrets::SignMessageArgs),
    /// What the wallet's account on its network holds and needs: Tron's
    /// resources, XRP's and Stellar's reserve, a TON contract's state, a
    /// Substrate balance's parts, NEAR's storage, a Cardano base address's
    /// stake address with its rewards and delegation.
    Account(WalletArg),
    /// An Ethereum wallet's ENS primary name, where it resolves back.
    Ens(WalletArg),
    /// The ERC-20 allowances an EVM wallet has granted that still stand,
    /// from its indexer's approval logs and a live read of each.
    Approvals(WalletArg),
    /// Build the transaction that sets an allowance back to zero; sign and
    /// broadcast it with `send sign` and `send broadcast-signed`.
    Revoke(account::RevokeArgs),
    /// The ERC-721 and ERC-1155 tokens an EVM wallet holds, from its
    /// network's Blockscout inventory.
    Nfts(WalletArg),
    /// Build the transaction that sends an NFT: one ERC-721 token, or a
    /// quantity of an ERC-1155 id. The standard and the wallet's ownership
    /// are read from the contract; sign and broadcast it with `send sign`
    /// and `send broadcast-signed`.
    SendNft(account::SendNftArgs),
    /// Build the transaction that closes an XRP or Stellar account into
    /// another existing account, recovering its reserve: everything it
    /// holds goes there and the account is deleted. Sign and broadcast it
    /// with `send sign` and `send broadcast-signed`.
    Close(account::CloseArgs),
    /// List an XRP Ledger or Stellar wallet's trust lines, each saying
    /// whether it can be removed.
    TrustLines(WalletArg),
    /// Build the transaction that opens a trust line to an issued asset, so
    /// the wallet can hold it; sign and broadcast it with `send`.
    Trust(account::TrustArgs),
    /// Build the transaction that removes an empty trust line, freeing its
    /// reserve; sign and broadcast it with `send`.
    Untrust(account::TrustArgs),
    /// A NEAR account's access keys, the one Spectra signs with marked.
    Keys(WalletArg),
    /// Build the transaction that deletes one of a NEAR account's
    /// function-call keys; sign and broadcast it with `send sign` and
    /// `send broadcast-signed`.
    DeleteKey(account::DeleteKeyArgs),
    /// The NEAR token contracts holding a storage deposit the account can
    /// have back: registered, with none of their token left in it.
    TokenStorage(WalletArg),
    /// Build the transaction that unregisters a NEAR account from a token
    /// contract it holds none of, returning the storage deposit; sign and
    /// broadcast it with `send sign` and `send broadcast-signed`.
    RefundStorage(account::RefundStorageArgs),
    /// A Sui wallet's coin types and how many objects hold each.
    Objects(WalletArg),
    /// Build the transaction that merges a Sui coin type's objects into
    /// one; sign and broadcast it with `send sign` and `send
    /// broadcast-signed`.
    Merge(account::MergeArgs),
    /// A Solana wallet's empty token accounts and the rent each returns.
    TokenAccounts(WalletArg),
    /// Build the transaction that closes empty Solana token accounts for
    /// their rent: the named ones, or every closable one (at most 20); sign
    /// and broadcast it with `send sign` and `send broadcast-signed`.
    CloseTokenAccounts(account::CloseTokenAccountsArgs),
    /// The proof of a Monero payment this wallet sent: the transaction key
    /// monero-wallet-cli's `check_tx_key` verifies.
    ProvePayment(account::ProvePaymentArgs),
    /// Scan a Zcash wallet's shielded pools from a lightwalletd server, a
    /// batch at a time until the scan is complete (one batch with `--once`).
    /// The first batch creates the shielded account from the seed.
    ZcashSync(private::ScanSyncArgs),
    /// Where a Zcash wallet's shielded scan stands, what it holds and the
    /// unified address it receives at.
    ZcashStatus(WalletArg),
    /// Build the transaction that moves a Zcash wallet's transparent funds
    /// into its shielded pool; sign and broadcast it with `send sign` and
    /// `send broadcast-signed` to a lightwalletd server.
    Shield(WalletArg),
    /// Build a payment from a Zcash wallet's shielded funds; sign and
    /// broadcast it with `send sign` and `send broadcast-signed` to a
    /// lightwalletd server.
    SendShielded(private::SendShieldedArgs),
    /// Scan for a Litecoin wallet's MWEB funds from a Litecoin node, a batch
    /// at a time until the scan is complete (one batch with `--once`). The
    /// first batch derives the MWEB keys from the seed.
    MwebSync(private::ScanSyncArgs),
    /// Where a Litecoin wallet's MWEB scan stands, what it holds and the
    /// stealth address it receives at.
    MwebStatus(WalletArg),
    /// Build the peg-in that moves a Litecoin wallet's transparent LTC into
    /// its MWEB funds; sign and broadcast it with `send sign` and
    /// `send broadcast-signed`.
    MwebPegin(private::MwebPeginArgs),
    /// Build a payment from a Litecoin wallet's MWEB funds, to an MWEB
    /// address or by a peg-out to any other; sign and broadcast it with
    /// `send sign` and `send broadcast-signed`.
    SendMweb(private::SendMwebArgs),
}

pub fn run(ctx: &Ctx, out: Out, command: WalletCommand) -> CliResult<()> {
    use WalletCommand as W;
    match command {
        W::CheckPassword(args) => setup::check_password(out, args),
        W::CheckSeed(args) => setup::check_seed(out, args),
        W::Methods(args) => setup::methods(out, args),
        W::Capabilities(args) => setup::capabilities(ctx, out, args),
        W::Reserve(args) => setup::reserve(ctx, out, args),
        W::New(args) => create::new(ctx, out, args),
        W::Import(args) => create::import(ctx, out, args),
        W::Watch(args) => create::watch(ctx, out, args),
        W::CopyTargets(args) => create::copy_targets(ctx, out, args),
        W::Copy(args) => create::copy(ctx, out, args),
        W::List => show::list(ctx, out),
        W::Show(args) => show::show(ctx, out, args),
        W::Actions(args) => show::actions(ctx, out, args),
        W::Receive(args) => show::receive(ctx, out, args),
        W::Coins(args) => show::coins(ctx, out, args),
        W::Derived => show::derived(ctx, out),
        W::Rename(args) => change::rename(ctx, out, args),
        W::Inclusion(args) => change::inclusion(ctx, out, args),
        W::Hide(args) => change::set_hidden(ctx, out, args, true),
        W::Unhide(args) => change::set_hidden(ctx, out, args, false),
        W::Delete(args) => change::delete(ctx, out, args),
        W::Export(args) => secrets::export(ctx, out, args),
        W::SignMessage(args) => secrets::sign_message(ctx, out, args),
        W::Account(args) => account::account(ctx, out, args),
        W::Ens(args) => account::ens(ctx, out, args),
        W::Approvals(args) => account::approvals(ctx, out, args),
        W::Revoke(args) => account::revoke(ctx, out, args),
        W::Nfts(args) => account::nfts(ctx, out, args),
        W::SendNft(args) => account::send_nft(ctx, out, args),
        W::Close(args) => account::close(ctx, out, args),
        W::TrustLines(args) => account::trust_lines(ctx, out, args),
        W::Trust(args) => account::trust(ctx, out, args, false),
        W::Untrust(args) => account::trust(ctx, out, args, true),
        W::Keys(args) => account::keys(ctx, out, args),
        W::DeleteKey(args) => account::delete_key(ctx, out, args),
        W::TokenStorage(args) => account::token_storage(ctx, out, args),
        W::RefundStorage(args) => account::refund_storage(ctx, out, args),
        W::Objects(args) => account::objects(ctx, out, args),
        W::Merge(args) => account::merge(ctx, out, args),
        W::TokenAccounts(args) => account::token_accounts(ctx, out, args),
        W::CloseTokenAccounts(args) => account::close_token_accounts(ctx, out, args),
        W::ProvePayment(args) => account::prove_payment(ctx, out, args),
        W::ZcashSync(args) => private::zcash_sync(ctx, out, args),
        W::ZcashStatus(args) => private::zcash_status(ctx, out, args),
        W::Shield(args) => private::shield(ctx, out, args),
        W::SendShielded(args) => private::send_shielded(ctx, out, args),
        W::MwebSync(args) => private::mweb_sync(ctx, out, args),
        W::MwebStatus(args) => private::mweb_status(ctx, out, args),
        W::MwebPegin(args) => private::mweb_pegin(ctx, out, args),
        W::SendMweb(args) => private::send_mweb(ctx, out, args),
    }
}

// ─── A wallet, as every report shows it ─────────────────────────────────────

/// A stored wallet as the reports that name one write it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletView {
    id: String,
    name: String,
    chain: Chain,
    address: String,
    /// Stored addresses for each network in the wallet's family.
    addresses: BTreeMap<&'static str, String>,
    derivation_path: Option<String>,
    restore_height: Option<u64>,
    hidden_holdings: Vec<String>,
    icp_principal: Option<String>,
    is_watch_only: bool,
    #[serde(skip)]
    holds: &'static str,
}

impl From<&WalletState> for WalletView {
    fn from(wallet: &WalletState) -> Self {
        Self {
            id: wallet.id.clone(),
            name: wallet.name.clone(),
            chain: wallet.chain_id,
            address: wallet_address(wallet).to_string(),
            addresses: wallet
                .addresses
                .iter()
                .map(|entry| (entry.chain_id.str_id(), entry.address.clone()))
                .collect(),
            derivation_path: wallet.derivation_path.clone(),
            restore_height: wallet.restore_height,
            hidden_holdings: wallet.hidden_holdings.clone(),
            icp_principal: wallet.icp_principal.clone(),
            is_watch_only: wallet.is_watch_only(),
            holds: match wallet.signing {
                _ if wallet.is_watch_only() => "watch-only",
                WalletSigning::PrivateKey { .. } => "private key",
                _ => "seed phrase",
            },
        }
    }
}

impl WalletView {
    fn render(&self) {
        out::field("name", &self.name.bold().to_string());
        out::field("chain", &out::chain(self.chain).to_string());
        out::field("type", self.holds);
        if let Some(path) = &self.derivation_path {
            out::field("path", &out::hint(path).to_string());
        }
        out::field("address", &out::info(&self.address).to_string());
        if let Some(principal) = &self.icp_principal {
            out::field("principal", &out::info(principal).to_string());
        }
    }
}

/// A recovery phrase, four numbered words to a line.
fn render_words(seed_phrase: &str) {
    let words: Vec<&str> = seed_phrase.split_whitespace().collect();
    for row in words.chunks(4).enumerate() {
        let (line, chunk) = row;
        let cells: Vec<String> = chunk
            .iter()
            .enumerate()
            .map(|(column, word)| format!("{:>2}. {word:<12}", line * 4 + column + 1))
            .collect();
        println!("  {}", cells.join("  "));
    }
}

/// The warning under anything secret a command prints.
fn render_secret_warning() {
    println!();
    println!(
        "  {} {}",
        out::accent("!").bold(),
        "store this securely and clear your terminal".bold()
    );
}

//! `validate` calls the same `validate_address` the import rule calls; the
//! address book reports core's `addressBookRejected` rather than deciding for
//! itself.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::registry::Chain;
use spectra_core::service::EndpointHolder;
use spectra_core::store::state::{AddressBookRejection, StateCommand, StateEvent, StateTransition};
use spectra_core::validation::address::validate_address;

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

#[derive(Subcommand)]
pub enum AddressCommand {
    /// Check an address the way an import does.
    Validate(ValidateArgs),
    /// Saved recipients.
    #[command(subcommand)]
    Book(BookCommand),
    /// Who holds an address on a wallet's network: that wallet, another of
    /// yours, or a saved contact.
    Holder(HolderArgs),
    /// Check that a message was signed by an address, in its network's
    /// scheme. Exits 3 when it was not.
    VerifyMessage(VerifyMessageArgs),
}

#[derive(Args)]
pub struct VerifyMessageArgs {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
    /// The address that signed.
    #[arg(long)]
    address: String,
    /// The message, as text.
    #[arg(long)]
    message: String,
    /// The signature, as the signing wallet wrote it.
    #[arg(long)]
    signature: String,
}

#[derive(Args)]
pub struct HolderArgs {
    /// The wallet the address is seen from; its network is the one asked about.
    wallet: String,
    /// The address to name.
    address: String,
}

#[derive(Args)]
pub struct ValidateArgs {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
    /// The address to check.
    address: String,
}

#[derive(Subcommand)]
pub enum BookCommand {
    /// List saved recipients.
    List,
    /// Save a recipient.
    Add(BookAddArgs),
    /// Remove a recipient by id or name.
    Remove(BookRemoveArgs),
}

#[derive(Args)]
pub struct BookAddArgs {
    /// Chain display name or registry id.
    #[arg(long)]
    chain: String,
    /// Contact name.
    #[arg(long)]
    name: String,
    /// Recipient address.
    #[arg(long)]
    address: String,
    /// Optional note.
    #[arg(long, default_value = "")]
    note: String,
}

#[derive(Args)]
pub struct BookRemoveArgs {
    /// Contact id or name.
    contact: String,
}

pub fn run(ctx: &Ctx, out: Out, command: AddressCommand) -> CliResult<()> {
    match command {
        AddressCommand::Validate(args) => validate(out, args),
        AddressCommand::VerifyMessage(args) => {
            let chain = resolve_chain(&args.chain)?;
            let valid = spectra_core::send::message::verify_message(
                chain,
                args.address,
                args.message,
                args.signature,
            );
            if !valid {
                return Err(CliError::rejected(
                    "the signature is not this address's over this message",
                ));
            }
            out.report(&SignedByReport { valid })
        }
        AddressCommand::Book(BookCommand::List) => book_list(ctx, out),
        AddressCommand::Book(BookCommand::Add(args)) => book_add(ctx, out, args),
        AddressCommand::Book(BookCommand::Remove(args)) => book_remove(ctx, out, args),
        AddressCommand::Holder(args) => holder(ctx, out, args),
    }
}

/// A signature checked against the address that claims it.
#[derive(Serialize)]
struct SignedByReport {
    valid: bool,
}

impl Report for SignedByReport {
    fn render(&self) {
        println!("  {} signed by this address", out::ok_mark());
    }
}

#[derive(Serialize)]
struct HolderReport {
    holder: Option<EndpointHolder>,
}

impl Report for HolderReport {
    fn render(&self) {
        match &self.holder {
            Some(EndpointHolder::Wallet { name }) => println!("  wallet {}", name.bold()),
            Some(EndpointHolder::Contact { name }) => println!("  contact {}", name.bold()),
            None => println!("  {}", out::hint("unknown")),
        }
    }
}

fn holder(ctx: &Ctx, out: Out, args: HolderArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let holder = ctx.rt.block_on(ctx.service()?.address_holder(
        wallet.id,
        wallet.chain_id,
        args.address.trim().to_string(),
    ))?;
    out.report(&HolderReport { holder })
}

/// An address the network takes, in the form it is stored.
#[derive(Serialize)]
struct ValidReport {
    valid: bool,
    chain: Chain,
    normalized: String,
    #[serde(skip)]
    typed: String,
}

impl Report for ValidReport {
    fn render(&self) {
        if self.normalized == self.typed {
            println!("  {} valid", out::ok_mark());
        } else {
            println!(
                "  {} valid, normalised to {}",
                out::ok_mark(),
                out::info(&self.normalized)
            );
        }
    }
}

/// Exit 3 on a refusal, so a script can assert it rather than parse a message.
fn validate(out: Out, args: ValidateArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let typed = args.address.trim().to_string();
    let result = validate_address(chain, &typed);
    if !result.is_valid {
        return Err(CliError::rejected(format!(
            "not a valid {} address",
            chain.chain_display_name()
        )));
    }
    out.report(&ValidReport {
        valid: true,
        chain,
        normalized: result.normalized_value.unwrap_or_else(|| typed.clone()),
        typed,
    })
}

#[derive(Serialize)]
struct ContactRow {
    id: String,
    name: String,
    chain: Chain,
    address: String,
    note: String,
}

#[derive(Serialize)]
struct ContactsReport {
    contacts: Vec<ContactRow>,
}

impl Report for ContactsReport {
    fn render(&self) {
        println!();
        if self.contacts.is_empty() {
            println!("  {}", out::hint("no saved recipients"));
            return;
        }
        for contact in &self.contacts {
            println!(
                "  {}  {}  {}",
                out::tint("●", contact.chain).bold(),
                contact.name.bold(),
                out::chain(contact.chain),
            );
            println!("     {}", out::info(&contact.address));
            if !contact.note.is_empty() {
                println!("     {}", out::hint(&contact.note));
            }
        }
    }
}

fn book_list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let contacts = ctx
        .state()?
        .address_book
        .into_iter()
        .map(|entry| ContactRow {
            id: entry.id,
            name: entry.name,
            chain: entry.chain_id,
            address: entry.address,
            note: entry.note,
        })
        .collect();
    out.report(&ContactsReport { contacts })
}

#[derive(Serialize)]
struct SavedReport {
    id: Option<String>,
    #[serde(skip)]
    name: String,
}

impl Report for SavedReport {
    fn render(&self) {
        println!("  {} saved {}", out::ok_mark(), self.name.bold());
    }
}

fn book_add(ctx: &Ctx, out: Out, args: BookAddArgs) -> CliResult<()> {
    // Resolved so `btc` and `Bitcoin` reach the same entry; core stores the
    // chain id and assigns the entry's id.
    let chain = resolve_chain(&args.chain)?;
    let transition = ctx.apply(StateCommand::AddAddressBookEntry {
        name: args.name.clone(),
        chain_id: chain,
        address: args.address.clone(),
        note: args.note.clone(),
    })?;
    if let Some(reason) = rejection(&transition) {
        return Err(CliError::rejected(rejection_text(reason)));
    }
    out.report(&SavedReport {
        id: transition.events.iter().find_map(|event| match event {
            StateEvent::AddressBookEntryAdded { id } => Some(id.clone()),
            _ => None,
        }),
        name: args.name,
    })
}

#[derive(Serialize)]
struct RemovedReport {
    removed: String,
    #[serde(skip)]
    name: String,
}

impl Report for RemovedReport {
    fn render(&self) {
        println!("  {} removed {}", out::ok_mark(), self.name.bold());
    }
}

fn book_remove(ctx: &Ctx, out: Out, args: BookRemoveArgs) -> CliResult<()> {
    let entry = ctx
        .state()?
        .address_book
        .into_iter()
        .find(|entry| {
            entry.id.eq_ignore_ascii_case(&args.contact)
                || entry.name.eq_ignore_ascii_case(&args.contact)
        })
        .ok_or_else(|| CliError::failure(format!("no contact matching {:?}", args.contact)))?;
    ctx.apply(StateCommand::RemoveAddressBookEntry {
        id: entry.id.clone(),
    })?;
    out.report(&RemovedReport {
        removed: entry.id,
        name: entry.name,
    })
}

/// Core decides; the front end only chooses the wording.
fn rejection(transition: &StateTransition) -> Option<AddressBookRejection> {
    transition.events.iter().find_map(|event| match event {
        StateEvent::AddressBookRejected { reason } => Some(*reason),
        _ => None,
    })
}

fn rejection_text(reason: AddressBookRejection) -> &'static str {
    match reason {
        AddressBookRejection::EmptyName => "a contact name cannot be empty",
        AddressBookRejection::InvalidAddress => "that address is not valid for this chain",
        AddressBookRejection::DuplicateAddress => "that address is already saved",
    }
}

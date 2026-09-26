//! `validate` calls the same `validate_address` the import rule calls; the
//! address book reports core's `addressBookRejected` rather than deciding for
//! itself.

use clap::{Args, Subcommand};
use colored::Colorize as _;
use spectra_core::store::state::{AddressBookRejection, StateCommand, StateEvent, StateTransition};
use spectra_core::validation::address::{AddressValidationRequest, validate_address};

use super::resolve_chain;
use crate::ctx::Ctx;
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

#[derive(Subcommand)]
pub enum AddressCommand {
    /// Check an address the way an import does.
    Validate(ValidateArgs),
    /// Saved recipients.
    #[command(subcommand)]
    Book(BookCommand),
}

#[derive(Args)]
pub struct ValidateArgs {
    /// Chain display name, registry id or symbol.
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
    /// Chain display name, registry id or symbol.
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
        AddressCommand::Book(BookCommand::List) => book_list(ctx, out),
        AddressCommand::Book(BookCommand::Add(args)) => book_add(ctx, out, args),
        AddressCommand::Book(BookCommand::Remove(args)) => book_remove(ctx, out, args),
    }
}

/// Exit 3 on a refusal, so a script can assert it rather than parse a message.
fn validate(out: Out, args: ValidateArgs) -> CliResult<()> {
    let chain = resolve_chain(&args.chain)?;
    let result = validate_address(AddressValidationRequest {
        kind: chain.address_validation_kind().to_string(),
        value: args.address.trim().to_string(),
    });

    if !result.is_valid {
        return Err(CliError::rejected(format!(
            "not a valid {} address",
            chain.chain_display_name()
        )));
    }

    let normalized = result
        .normalized_value
        .unwrap_or_else(|| args.address.clone());
    out.text(|| {
        if normalized == args.address.trim() {
            println!("  {} valid", out::ok_mark());
        } else {
            println!(
                "  {} valid, normalised to {}",
                out::ok_mark(),
                out::info(&normalized)
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "valid": true,
        "chain": chain.str_id(),
        "normalized": normalized,
    }));
    Ok(())
}

fn book_list(ctx: &Ctx, out: Out) -> CliResult<()> {
    let entries = ctx.state()?.address_book;
    out.text(|| {
        println!();
        if entries.is_empty() {
            println!("  {}", out::hint("no saved recipients"));
            return;
        }
        for entry in &entries {
            println!(
                "  {}  {}  {}",
                out::tint("●", &entry.chain_id).bold(),
                entry.name.bold(),
                out::tint(&super::chain_name(&entry.chain_id), &entry.chain_id),
            );
            println!("     {}", out::info(&entry.address));
            if !entry.note.is_empty() {
                println!("     {}", out::hint(&entry.note));
            }
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "contacts": entries
            .iter()
            .map(|entry| serde_json::json!({
                "id": entry.id,
                "name": entry.name,
                "chain": entry.chain_id,
                "address": entry.address,
                "note": entry.note,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

fn book_add(ctx: &Ctx, out: Out, args: BookAddArgs) -> CliResult<()> {
    // Resolved so `btc` and `Bitcoin` reach the same entry; core stores the
    // chain id and assigns the entry's id.
    let chain = resolve_chain(&args.chain)?;
    let transition = ctx.apply(StateCommand::AddAddressBookEntry {
        name: args.name.clone(),
        chain_id: chain.str_id().to_string(),
        address: args.address.clone(),
        note: args.note.clone(),
    })?;

    if let Some(reason) = rejection(&transition) {
        return Err(CliError::rejected(rejection_text(reason)));
    }

    let id = transition.events.iter().find_map(|event| match event {
        spectra_core::store::state::StateEvent::AddressBookEntryAdded { id } => Some(id.clone()),
        _ => None,
    });
    out.text(|| println!("  {} saved {}", out::ok_mark(), args.name.bold()));
    out.emit(serde_json::json!({ "ok": true, "id": id }));
    Ok(())
}

fn book_remove(ctx: &Ctx, out: Out, args: BookRemoveArgs) -> CliResult<()> {
    let entries = ctx.state()?.address_book;
    let entry = entries
        .iter()
        .find(|entry| {
            entry.id.eq_ignore_ascii_case(&args.contact)
                || entry.name.eq_ignore_ascii_case(&args.contact)
        })
        .ok_or_else(|| CliError::failure(format!("no contact matching {:?}", args.contact)))?;

    ctx.apply(StateCommand::RemoveAddressBookEntry {
        id: entry.id.clone(),
    })?;

    out.text(|| println!("  {} removed {}", out::ok_mark(), entry.name.bold()));
    out.emit(serde_json::json!({ "ok": true, "removed": entry.id }));
    Ok(())
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

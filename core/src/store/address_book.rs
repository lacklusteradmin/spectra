//! Saved recipients and the commands that change them.

use serde::{Deserialize, Serialize};

use super::state::{ResidentState, StateCommand, StateEvent};

/// A saved recipient.
///
/// `address` is stored already normalized for its chain, so two entries are
/// the same recipient exactly when their normalized addresses are equal. A
/// case-insensitive match would merge distinct base58 addresses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AddressBookEntry {
    pub id: String,
    pub name: String,
    pub chain_id: crate::registry::Chain,
    pub address: String,
    pub note: String,
}

/// Why an address-book entry was refused. Front ends map these to their own
/// wording; the decision itself is core's.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AddressBookRejection {
    EmptyName,
    InvalidAddress,
    DuplicateAddress,
}

/// Is this (chain, address) pair already saved? Addresses are stored
/// normalized, so this is a case-insensitive compare rather than a per-chain
/// rule. `excluding` skips one entry, for edit-in-place checks.
fn address_book_contains(
    state: &ResidentState,
    chain_id: crate::registry::Chain,
    normalized_address: &str,
    excluding: Option<&str>,
) -> bool {
    if normalized_address.is_empty() {
        return false;
    }
    state.address_book.iter().any(|entry| {
        Some(entry.id.as_str()) != excluding
            && entry.chain_id == chain_id
            && entry.address == normalized_address
    })
}

/// Apply a address book command; `reduce_state_in_place` routes only these here.
pub(super) fn reduce(
    state: &mut ResidentState,
    command: StateCommand,
    events: &mut Vec<StateEvent>,
) {
    match command {
        StateCommand::AddAddressBookEntry {
            name,
            chain_id,
            address,
            note,
        } => {
            let name = name.trim().to_string();
            let address = crate::send::flow::normalize_address(chain_id, &address);

            // Refusals are reported, not silently dropped: a front end that
            // ignored the result would otherwise show a saved contact that was
            // never saved.
            let rejection = if name.is_empty() {
                Some(AddressBookRejection::EmptyName)
            } else if !crate::send::flow::is_valid_send_address(chain_id, address.clone()) {
                Some(AddressBookRejection::InvalidAddress)
            } else if address_book_contains(state, chain_id, &address, None) {
                Some(AddressBookRejection::DuplicateAddress)
            } else {
                None
            };

            match rejection {
                Some(reason) => events.push(StateEvent::AddressBookRejected { reason }),
                None => {
                    // Newest first: the list is a recency-ordered shortlist,
                    // not an archive.
                    let id = super::new_id();
                    state.address_book.insert(
                        0,
                        AddressBookEntry {
                            id: id.clone(),
                            name,
                            chain_id,
                            address,
                            note: note.trim().to_string(),
                        },
                    );
                    events.push(StateEvent::AddressBookEntryAdded { id });
                }
            }
        }
        StateCommand::RenameAddressBookEntry { id, name } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                events.push(StateEvent::AddressBookRejected {
                    reason: AddressBookRejection::EmptyName,
                });
            } else if let Some(entry) = state.address_book.iter_mut().find(|e| e.id == id)
                && entry.name != name
            {
                entry.name = name;
                events.push(StateEvent::AddressBookEntryRenamed { id });
            }
        }
        StateCommand::RemoveAddressBookEntry { id } => {
            let before = state.address_book.len();
            state.address_book.retain(|entry| entry.id != id);
            if state.address_book.len() != before {
                events.push(StateEvent::AddressBookEntryRemoved { id });
            }
        }
        _ => unreachable!("routed by reduce_state_in_place"),
    }
}

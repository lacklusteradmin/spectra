//! Relational storage. Connection ownership and cross-table transactions stay
//! centralized; CRUD is grouped by the domain it stores.
use crate::store::state::{AddressBookEntry, ResidentState, WalletState};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// A stored record the wallet submitted: every kind but a receive
/// (`TransactionKind::is_submitted`). One spelling, so the pending-sender
/// index and the queries it serves cannot drift apart.
macro_rules! submitted_kind {
    () => {
        "json_extract(payload, '$.kind') <> 'receive'"
    };
}
pub(crate) use submitted_kind;

mod addresses;
mod connection;
pub mod error;
mod history;
mod history_pagination;
pub(crate) use history_pagination::*;
mod history_query;
pub(crate) use history_query::*;
mod keypool;
mod multisig;
mod state;
mod wallets;
pub use addresses::*;
pub use connection::WalletDatabase;
use connection::with_conn;
pub(crate) use connection::{delete_sqlite_files, now_secs, stored_wallet_ids};
pub use history::*;
pub use keypool::*;
pub(crate) use multisig::*;
pub use state::*;
pub use wallets::*;
#[cfg(test)]
mod tests;

mod sends;
pub(crate) use sends::*;

mod scan_cache;
pub(crate) use scan_cache::{scan_cache_load, scan_cache_save};
pub(crate) mod zcash;

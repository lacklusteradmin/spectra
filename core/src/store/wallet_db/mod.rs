//! Relational storage. Connection ownership and cross-table transactions stay
//! centralized; CRUD is grouped by the domain it stores.
use crate::store::state::{AddressBookEntry, CoreAppState, WalletSummary};
use rusqlite::params;
use serde::{Deserialize, Serialize};
mod addresses;
mod connection;
mod history;
mod keypool;
mod state;
mod teardown;
mod wallets;
pub use addresses::*;
pub(crate) use connection::now_secs;
use connection::with_conn;
pub use connection::WalletDatabase;
pub use history::*;
pub use keypool::*;
pub use state::*;
pub use teardown::*;
pub use wallets::*;
#[cfg(test)]
mod tests;

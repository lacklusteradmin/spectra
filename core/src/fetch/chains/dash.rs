//! Dash chain client.
//!
//! Dash never adopted SegWit, so its wire format is Bitcoin's legacy one. The
//! REST surface is Trezor Blockbook's (`https://dash1.trezor.io`), shared with
//! the rest of that family in [`super::blockbook`].

use super::blockbook::{
    BlockbookBalance, BlockbookClient, BlockbookHistoryEntry, BlockbookNetwork,
    BlockbookSendResult, BlockbookUtxoEntry,
};

pub struct Dash;
impl BlockbookNetwork for Dash {}

pub type DashClient = BlockbookClient<Dash>;
pub type DashBalance = BlockbookBalance;
pub type DashUtxo = BlockbookUtxoEntry;
pub type DashHistoryEntry = BlockbookHistoryEntry;
pub type DashSendResult = BlockbookSendResult;

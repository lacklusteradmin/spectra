//! Bitcoin Gold chain client.
//!
//! The REST surface is Trezor Blockbook's (`https://btg1.trezor.io`), shared
//! with the rest of that family in [`super::blockbook`].

use super::blockbook::{
    BlockbookBalance, BlockbookClient, BlockbookHistoryEntry, BlockbookNetwork,
    BlockbookSendResult, BlockbookUtxoEntry,
};

pub struct BitcoinGold;
impl BlockbookNetwork for BitcoinGold {}

pub type BitcoinGoldClient = BlockbookClient<BitcoinGold>;
pub type BtgBalance = BlockbookBalance;
pub type BtgUtxo = BlockbookUtxoEntry;
pub type BtgHistoryEntry = BlockbookHistoryEntry;
pub type BtgSendResult = BlockbookSendResult;

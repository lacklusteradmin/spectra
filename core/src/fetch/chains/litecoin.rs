//! Litecoin chain client.
//!
//! Litecoin supports both legacy P2PKH (L-addresses) and native SegWit P2WPKH
//! (ltc1q- addresses). Network version byte: 0x30 (P2PKH), 0x32 (P2SH),
//! bech32 HRP = "ltc".
//!
//! The REST surface is Trezor Blockbook's, shared with the rest of that family
//! in [`super::blockbook`]. Only the marker below is Litecoin's own.

use super::blockbook::{BlockbookClient, BlockbookNetwork, BlockbookSendResult};

pub struct Litecoin;
impl BlockbookNetwork for Litecoin {}

pub type LitecoinClient = BlockbookClient<Litecoin>;
pub type LtcSendResult = BlockbookSendResult;

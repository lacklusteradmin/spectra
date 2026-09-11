//! Bitcoin Cash chain client.
//!
//! BCH uses the CashAddr address format (prefix "bitcoincash:") but also
//! accepts legacy P2PKH addresses (version 0x00, same as BTC), which is why it
//! is the one member of the Blockbook family that normalizes an address before
//! asking about it. Everything else is the shared surface in
//! [`super::blockbook`].
//!
//! Signing is SIGHASH_ALL with replay protection — BCH uses its own
//! SIGHASH_FORKID = 0x40 rather than the BIP143 SegWit digest — and lives in
//! `crate::send::chains::bitcoin_cash`.

use super::blockbook::{BlockbookClient, BlockbookNetwork, BlockbookSendResult};

pub struct BitcoinCash;

impl BlockbookNetwork for BitcoinCash {
    fn normalize_address(address: &str) -> String {
        crate::derivation::chains::bitcoin_cash::normalize_bch_address(address)
    }
}

pub type BitcoinCashClient = BlockbookClient<BitcoinCash>;
pub type BchSendResult = BlockbookSendResult;

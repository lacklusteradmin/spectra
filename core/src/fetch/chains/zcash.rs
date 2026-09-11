//! Zcash transparent chain client.
//!
//! Shielded addresses (`zs...` / `u1...`) are out of scope; only `t1` / `t3`
//! transparent addresses are supported. The REST surface is Trezor
//! Blockbook's (`https://zec1.trezor.io`), shared with the rest of that family
//! in [`super::blockbook`] — including `fetch_chain_tip_height`, which the V5
//! transaction builder needs and which used to live only here.

use super::blockbook::{BlockbookClient, BlockbookNetwork, BlockbookSendResult};

pub struct Zcash;
impl BlockbookNetwork for Zcash {}

pub type ZcashClient = BlockbookClient<Zcash>;
pub type ZecSendResult = BlockbookSendResult;

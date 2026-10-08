//! Litecoin's MimbleWimble extension block (MWEB): device-local keys,
//! scanning and signing.
//!
//! Only the light-client requests of `api::litecoin_p2p` and the broadcast
//! of the signed transaction cross the transport; the scan key, the spend
//! key and what a scan finds stay on the device. The protocol is Litecoin
//! Core's libmw, as its blocks and ltcd (the Go implementation mwebd runs)
//! exercise it; `core/tests/fixtures/mweb-mainnet.json` holds mainnet blocks,
//! light-client answers and ltcd's vector that the tests check against.

pub(crate) mod keys;
pub(crate) mod output;
pub(crate) mod prepared;
pub(crate) mod primitives;
pub(crate) mod transaction;

#[cfg(test)]
#[path = "../tests/litecoin_mweb.rs"]
mod tests;

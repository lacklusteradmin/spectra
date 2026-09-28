//! Chain read clients, refresh policy and network helpers.

pub(crate) mod bitcoin_history;
pub mod history;
pub mod history_decode;
pub mod history_store;
pub mod http;
pub(crate) mod json_rpc;

pub mod price;
pub mod refresh_engine;
pub mod refresh_policy;
pub mod transactions;

// Per-chain read-path clients: client struct + shared types + balance /
// history / metadata / fee-estimate RPC methods.
pub mod aptos;
pub mod bitcoin;
pub mod bitcoin_sv;
pub mod bittensor;
pub mod blockbook;
pub mod cardano;
pub mod decred;
pub mod dogecoin;
pub mod evm;
pub mod icp;
pub mod kaspa;
pub mod near;
pub mod polkadot;
pub mod solana;
pub mod stellar;
pub mod sui;
pub mod ton;
pub mod tron;
pub(crate) mod tron_metadata_cache;
pub mod xrp;

/// One token holding an address turned out to have, as the chain itself
/// reports it.
///
/// Discovery asks the chain what an address holds instead of asking a
/// hand-kept list what to look for, so the contract address is the only field
/// the chain always supplies. `decimals` and `symbol` are `None` where the
/// chain reports a holding without them and the lookup that would resolve them
/// failed — callers fall back to the catalog, and a token nobody vouches for
/// is still reported, by its contract address, rather than hidden.
#[derive(Debug, Clone)]
pub struct HeldToken {
    pub contract: String,
    pub balance_raw: u128,
    pub decimals: Option<u8>,
    pub symbol: Option<String>,
}

/// Core amounts use u128 and decimal scaling up to 10^38.
pub(crate) fn checked_token_decimals(value: u128) -> Result<u8, String> {
    if value > 38 {
        return Err("token decimals exceed core precision limit (38)".into());
    }
    Ok(value as u8)
}

/// A history entry's time, in the provider's own unit: `None` only while the
/// chain has not given the transaction one — it is not yet in a block.
///
/// A confirmed transaction always has a time, so one that arrives without it
/// was read wrongly, and the fetch fails naming it rather than dating it 1970.
pub(crate) fn history_time(
    confirmed: bool,
    time: Option<u64>,
    txid: &str,
) -> Result<Option<u64>, String> {
    match time.filter(|t| *t > 0) {
        None if !confirmed => Ok(None),
        time => confirmed_history_time(time, txid).map(Some),
    }
}

/// The time of a transaction from a source that lists only confirmed ones.
pub(crate) fn confirmed_history_time(time: Option<u64>, txid: &str) -> Result<u64, String> {
    time.filter(|t| *t > 0)
        .ok_or_else(|| format!("history: confirmed transaction {txid} has no time"))
}

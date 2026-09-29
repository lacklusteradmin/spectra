//! Every request core sends to a chain service, and the parsing of its answer.
//!
//! One module per `EndpointApi`, named after its `as_str()`: each is that wire
//! contract's reads and submissions, whichever chain uses it. `utxo` is the one
//! module that spans several APIs — the UTXO family's client over them and the
//! answer types they share. `http` and `json_rpc` are the transport under all
//! of them, `time` the provider timestamps they read.
//!
//! `api` depends on nothing but the transport and `registry`: `fetch`, `send`,
//! `staking` and `service` call it, never the other way round. Building and
//! signing transactions is per chain and lives in `send`.

pub mod http;
pub(crate) mod json_rpc;
pub mod time;
pub(crate) mod tron_metadata_cache;
pub mod utxo;

pub mod aptos_rest;
pub mod bch_rest_v2;
pub mod blockbook;
pub mod blockcypher;
pub mod blockscout;
pub mod esplora;
pub mod evm_json_rpc;
pub mod horizon;
pub mod icp_rosetta;
pub mod insight;
pub mod kaspa_rest;
pub mod koios;
pub mod monero_daemon_rpc;
pub mod near_json_rpc;
pub mod nearblocks;
pub mod solana_json_rpc;
pub mod substrate_json_rpc;
pub mod sui_json_rpc;
pub mod toncenter_v2;
pub mod toncenter_v3;
pub mod tron_http;
pub mod trongrid_v1;
pub mod whatsonchain;
pub mod xrpl_json_rpc;

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

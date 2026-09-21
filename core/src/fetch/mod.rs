//! Chain read clients, refresh policy and network helpers.
//!
//! Protocol send results expose their native transaction identifiers and signed
//! payload encodings through [`SignedSubmission`]. Concrete result types stay
//! distinct so protocol-specific fields and FFI records remain explicit.

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
pub mod monero;
pub mod near;
pub mod polkadot;
pub mod solana;
pub mod stellar;
pub mod sui;
pub mod ton;
pub mod tron;
pub(crate) mod tron_metadata_cache;
pub mod xrp;

/// Encoding of the signed payload that a `*SendResult` carries. Lets
/// generic broadcast / rebroadcast code know how to hand the payload back
/// to the chain's submit RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedPayloadFormat {
    /// 0x-prefixed hex of raw signed bytes (Bitcoin family, Polkadot, EVM,
    /// XRP, Cardano CBOR, …).
    Hex,
    /// Base64-encoded signed bytes (Stellar XDR, NEAR, TON BOC, Solana).
    Base64,
    /// JSON-encoded signed transaction body (Tron, Aptos).
    Json,
    /// No portable payload — submission-style chains return only an
    /// identifier (ICP block index, Monero RPC echo).
    None,
}

/// Common shape of every `*SendResult`. Chain-specific result types
/// implement this so dispatch code at the service layer doesn't need to
/// match on the concrete type.
pub trait SignedSubmission {
    /// Canonical identifier the chain assigns the broadcast — txid, signature,
    /// digest, message hash, etc. Empty for chains that don't surface one
    /// before confirmation (none currently).
    fn submission_id(&self) -> &str;

    /// Signed bytes in the chain's native broadcast encoding (see
    /// [`SignedPayloadFormat`]). Empty when the chain doesn't expose a
    /// rebroadcastable payload.
    fn signed_payload(&self) -> &str;

    /// Encoding of [`Self::signed_payload`].
    fn signed_payload_format(&self) -> SignedPayloadFormat;
}

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

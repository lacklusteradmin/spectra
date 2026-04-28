//! Per-chain implementations. Each file in this folder owns one chain's code
//! for this axis (fetch / read-path).
//!
//! ## SendResult convention
//!
//! Every chain in `send::chains::*` returns a result type ending in
//! `SendResult` (e.g. `BitcoinSendResult`, `DotSendResult`, `EvmSendResult`).
//! Reader-confusion alarm: these aren't separate concepts, they're the same
//! shape with chain-specific field names that match the chain's native
//! encoding (hex, base64, cbor, xdr, json…). UniFFI requires distinct record
//! names per FFI surface, which is the only reason they aren't one type.
//!
//! Every `*SendResult` carries:
//!   1. A canonical transaction identifier (`txid` / `signature` / `digest` /
//!      `message_hash` / `block_index` — whatever the chain calls it).
//!   2. A signed payload encoded in the chain's native broadcast format,
//!      preserved so the wallet can rebroadcast without re-signing.
//!
//! Some chains add minor extras (EVM `nonce`, Aptos `version`, Monero
//! `fee_piconeros`/`amount_piconeros`) that are genuinely chain-specific
//! state — those are the only fields beyond the two above.
//!
//! See [`SignedSubmission`] for the trait that lets generic code treat all
//! `*SendResult` values uniformly.

pub mod aptos;
pub mod bitcoin;
pub mod bittensor;
pub mod bitcoin_cash;
pub mod bitcoin_gold;
pub mod bitcoin_sv;
pub mod cardano;
pub mod dash;
pub mod decred;
pub mod dogecoin;
pub mod evm;
pub mod icp;
pub mod kaspa;
pub mod litecoin;
pub mod monero;
pub mod near;
pub mod polkadot;
pub mod solana;
pub mod stellar;
pub mod sui;
pub mod ton;
pub mod tron;
pub mod xrp;
pub mod zcash;

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

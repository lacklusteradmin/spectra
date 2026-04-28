//! Substrate runtime call indexes.
//!
//! On Substrate-based chains the on-wire encoding of an extrinsic embeds two
//! 1-byte indexes — a pallet index and a call index within that pallet — that
//! identify the dispatch target. These indexes are *not* fixed by the chain's
//! protocol: a runtime upgrade can renumber pallets, and any chain that
//! adds/removes pallets in an upgrade will shift the indexes that surround
//! the change. Treat each `RuntimeCallIndex` value as a snapshot tied to a
//! specific runtime version, and re-verify against the chain's metadata
//! after every runtime upgrade.

#[derive(Debug, Clone, Copy)]
pub struct RuntimeCallIndex {
    pub pallet: u8,
    pub call: u8,
}

impl RuntimeCallIndex {
    pub const fn new(pallet: u8, call: u8) -> Self {
        Self { pallet, call }
    }
}

/// Polkadot mainnet (relay chain), `Balances.transfer_keep_alive`.
/// Verified against runtime spec_version 1_002_000 (April 2026).
pub const POLKADOT_BALANCES_TRANSFER_KEEP_ALIVE: RuntimeCallIndex = RuntimeCallIndex::new(0x05, 0x03);

/// Bittensor mainnet (subtensor), `Balances.transfer_keep_alive`.
/// Verified against subtensor runtime as of April 2026.
pub const BITTENSOR_BALANCES_TRANSFER_KEEP_ALIVE: RuntimeCallIndex = RuntimeCallIndex::new(0x06, 0x03);

/// Typed error for substrate-family signing paths. Replaces the
/// `Result<_, String>` returns that lost structure: callers can now
/// pattern-match on the failure mode (transient vs permanent vs caller
/// bug) instead of regex-matching the string. The `Display` impl
/// preserves the human-readable shape for log lines.
#[derive(Debug)]
pub enum SubstrateSignError {
    /// The 32-byte mini-secret didn't pass schnorrkel's validation
    /// (zero scalar, malformed encoding). Caller bug — the bytes
    /// shouldn't have made it past `derive_polkadot`.
    InvalidMiniSecret(String),
    /// Hex string failed to decode. Includes the field name so logs
    /// say which field — `genesis_hash`, `block_hash`, etc.
    HashDecode {
        field: &'static str,
        source: hex::FromHexError,
    },
    /// Hex string decoded but the byte count was wrong. Includes the
    /// expected and actual lengths plus the field name.
    WrongLength {
        field: &'static str,
        expected: usize,
        got: usize,
    },
}

impl std::fmt::Display for SubstrateSignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubstrateSignError::InvalidMiniSecret(detail) => {
                write!(f, "invalid sr25519 mini-secret: {detail}")
            }
            SubstrateSignError::HashDecode { field, source } => {
                write!(f, "{field} hex decode: {source}")
            }
            SubstrateSignError::WrongLength { field, expected, got } => {
                write!(f, "{field} wrong length: expected {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for SubstrateSignError {}

impl From<SubstrateSignError> for String {
    fn from(err: SubstrateSignError) -> String {
        err.to_string()
    }
}

// ── Typed byte-array wrappers ─────────────────────────────────────────
//
// `[u8; 32]` arguments to `build_signed_transfer` were positionally
// distinguishable but type-indistinguishable: a caller could pass
// (public_key, mini_secret) instead of (mini_secret, public_key) and
// produce a useless signature without a compile error. These wrappers
// tag each role so swaps fail to type-check.

/// 32-byte sr25519 mini-secret produced by `derive_polkadot` /
/// `derive_substrate_sr25519_material`. NOT an ed25519 secret — schnorrkel
/// expansion happens inside `build_signed_transfer`.
#[derive(Debug, Clone, Copy)]
pub struct Sr25519MiniSecret(pub [u8; 32]);

/// 32-byte sr25519 public key encoded in the SS58 address.
#[derive(Debug, Clone, Copy)]
pub struct Sr25519PublicKey(pub [u8; 32]);

/// 32-byte chain identifier (genesis hash, block hash). Tagged so swaps
/// at the call site fail to compile instead of producing a bad signing
/// payload.
#[derive(Debug, Clone, Copy)]
pub struct SubstrateChainHash(pub [u8; 32]);

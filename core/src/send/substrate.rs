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
pub const POLKADOT_BALANCES_TRANSFER_KEEP_ALIVE: RuntimeCallIndex =
    RuntimeCallIndex::new(0x05, 0x03);

/// Bittensor mainnet (subtensor), `Balances.transfer_keep_alive`.
/// Verified against subtensor runtime as of April 2026.
pub const BITTENSOR_BALANCES_TRANSFER_KEEP_ALIVE: RuntimeCallIndex =
    RuntimeCallIndex::new(0x06, 0x03);

pub(super) fn scale_compact_u32(n: u32) -> Vec<u8> {
    scale_compact_u128(n as u128)
}

pub(super) fn scale_compact_u128(n: u128) -> Vec<u8> {
    if n <= 63 {
        vec![(n << 2) as u8]
    } else if n <= 0x3fff {
        let v = ((n << 2) | 1) as u16;
        v.to_le_bytes().to_vec()
    } else if n <= 0x3fff_ffff {
        let v = ((n << 2) | 2) as u32;
        v.to_le_bytes().to_vec()
    } else {
        // Big-integer mode.
        let bytes = n.to_le_bytes();
        let sig_bytes = bytes.iter().rev().skip_while(|&&b| b == 0).count();
        let mut out = vec![((sig_bytes - 4) << 2 | 3) as u8];
        out.extend_from_slice(&bytes[..sig_bytes]);
        out
    }
}

pub(super) fn decode_hash_hex(hex_str: &str) -> Result<[u8; 32], String> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(s).map_err(|e| format!("hash decode: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| format!("hash wrong length: {}", hex_str))
}

pub(super) fn blake2b_256(data: &[u8]) -> [u8; 32] {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut h = Blake2b::<U32>::new();
    h.update(data);
    h.finalize().into()
}

//! Polkadot send: SCALE-encoded Balances.transfer_keep_alive builder,
//! sr25519 (schnorrkel) signer using the canonical substrate signing
//! context, and RPC submit (plus pre-signed rebroadcast).
//!
//! `private_key_bytes` is the 32-byte sr25519 mini-secret produced by
//! `derive_polkadot` (substrate-bip39 expansion). It is *not* a 64-byte
//! ed25519 secret — the historical signer that took 64 bytes never matched
//! the sr25519 public key encoded in the SS58 address, so the signature
//! never verified on-chain. This implementation expands the mini-secret to
//! a schnorrkel `Keypair`, signs with `signing_context(b"substrate")`, and
//! emits the result under `MultiSignature::Sr25519` (variant `0x01`).

use super::substrate::{blake2b_256, decode_hash_hex, scale_compact_u128, scale_compact_u32};
use serde_json::json;

use crate::derivation::polkadot::decode_ss58;
use crate::fetch::polkadot::{DotSendResult, PolkadotClient};
use crate::send::substrate::POLKADOT_BALANCES_TRANSFER_KEEP_ALIVE;

/// Substrate's transaction signing context — fixed across chains that use
/// the standard sr25519 multi-signature envelope.
const SR25519_SIGNING_CONTEXT: &[u8] = b"substrate";

impl PolkadotClient {
    /// Submit a pre-signed extrinsic hex (for rebroadcast).
    pub async fn submit_extrinsic_hex(&self, hex: &str) -> Result<DotSendResult, String> {
        let result = self
            .rpc_call("author_submitExtrinsic", json!([hex]))
            .await?;
        let txid = result.as_str().unwrap_or("").to_string();
        Ok(DotSendResult {
            txid,
            extrinsic_hex: hex.to_string(),
        })
    }
}

// ── SCALE-encoded Polkadot extrinsic builder

/// Build a signed Balances.transfer_keep_alive extrinsic.
#[allow(clippy::too_many_arguments)]
pub fn build_signed_transfer(
    to_address: &str,
    amount: u128,
    nonce: u32,
    spec_version: u32,
    tx_version: u32,
    genesis_hash: &str,
    block_hash: &str,
    private_key: &[u8; 32],
    public_key: &[u8; 32],
    era: Option<Vec<u8>>,
    tip: Option<u128>,
) -> Result<Vec<u8>, String> {
    let dest_pubkey = decode_ss58(to_address)?;

    let call = {
        let mut c = Vec::new();
        c.push(POLKADOT_BALANCES_TRANSFER_KEEP_ALIVE.pallet);
        c.push(POLKADOT_BALANCES_TRANSFER_KEEP_ALIVE.call);
        c.push(0x00); // dest: MultiAddress::Id
        c.extend_from_slice(&dest_pubkey);
        c.extend_from_slice(&scale_compact_u128(amount));
        c
    };

    let era = era.unwrap_or_else(|| vec![0x00u8]); // immortal
    let nonce_enc = scale_compact_u32(nonce);
    let tip = scale_compact_u128(tip.unwrap_or(0));

    let genesis_bytes = decode_hash_hex(genesis_hash)?;
    let block_bytes = decode_hash_hex(block_hash)?;

    let mut payload = Vec::new();
    payload.extend_from_slice(&call);
    payload.extend_from_slice(&era);
    payload.extend_from_slice(&nonce_enc);
    payload.extend_from_slice(&tip);
    payload.extend_from_slice(&spec_version.to_le_bytes());
    payload.extend_from_slice(&tx_version.to_le_bytes());
    payload.extend_from_slice(&genesis_bytes);
    payload.extend_from_slice(&block_bytes);

    // Substrate signs the Blake2-256 hash when the payload exceeds 256 bytes;
    // for short payloads it signs the bytes directly. Both branches produce
    // signatures the runtime accepts — the cutoff is purely a size optimization.
    let signing_input: Vec<u8> = if payload.len() > 256 {
        blake2b_256(&payload).to_vec()
    } else {
        payload.clone()
    };

    // ExpansionMode::Ed25519 — matches the public key encoded in the SS58 address.
    let mini = schnorrkel::MiniSecretKey::from_bytes(private_key)
        .map_err(|e| format!("invalid sr25519 mini-secret: {e}"))?;
    let keypair = mini.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    let signature = keypair.sign_simple(SR25519_SIGNING_CONTEXT, &signing_input);

    let mut extrinsic_body = Vec::new();
    extrinsic_body.push(0x84); // signed, version 4
    extrinsic_body.push(0x00); // signer: MultiAddress::Id
    extrinsic_body.extend_from_slice(public_key);
    extrinsic_body.push(0x01); // signature: MultiSignature::Sr25519
    extrinsic_body.extend_from_slice(&signature.to_bytes());
    extrinsic_body.extend_from_slice(&era);
    extrinsic_body.extend_from_slice(&nonce_enc);
    extrinsic_body.extend_from_slice(&tip);
    extrinsic_body.extend_from_slice(&call);

    let mut out = scale_compact_u32(extrinsic_body.len() as u32);
    out.extend_from_slice(&extrinsic_body);
    Ok(out)
}

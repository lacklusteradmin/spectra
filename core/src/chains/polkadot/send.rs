//! Polkadot send: SCALE-encoded Balances.transfer_keep_alive builder,
//! Ed25519 signer, and RPC submit (plus pre-signed rebroadcast).

use serde_json::json;

use super::derive::decode_ss58;
use super::{DotSendResult, PolkadotClient};

impl PolkadotClient {
    /// Sign and submit a Balances.transfer_keep_alive extrinsic.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        planck: u128,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<DotSendResult, String> {
        let _ = from_address;
        let nonce = self.fetch_nonce(from_address).await?;
        let (spec_version, tx_version) = self.fetch_runtime_version().await?;
        let genesis_hash = self.fetch_genesis_hash().await?;
        let block_hash = self.fetch_block_hash_latest().await?;

        let extrinsic = build_signed_transfer(
            to_address,
            planck,
            nonce,
            spec_version,
            tx_version,
            &genesis_hash,
            &block_hash,
            private_key_bytes,
            public_key_bytes,
        )?;

        let hex = format!("0x{}", hex::encode(&extrinsic));
        let result = self
            .rpc_call("author_submitExtrinsic", json!([hex]))
            .await?;
        let txid = result
            .as_str()
            .ok_or("author_submitExtrinsic: expected string")?
            .to_string();
        Ok(DotSendResult { txid, extrinsic_hex: hex })
    }

    /// Submit a pre-signed extrinsic hex (for rebroadcast).
    pub async fn submit_extrinsic_hex(&self, hex: &str) -> Result<DotSendResult, String> {
        let result = self
            .rpc_call("author_submitExtrinsic", json!([hex]))
            .await?;
        let txid = result.as_str().unwrap_or("").to_string();
        Ok(DotSendResult { txid, extrinsic_hex: hex.to_string() })
    }
}

// ----------------------------------------------------------------
// SCALE-encoded Polkadot extrinsic builder
// ----------------------------------------------------------------

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
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};

    // Decode the recipient's SS58 address to a 32-byte public key.
    let dest_pubkey = decode_ss58(to_address)?;

    // Balances.transfer_keep_alive: pallet 5, call 3 on Polkadot mainnet.
    let call = {
        let mut c = Vec::new();
        c.push(0x05); // pallet index (Balances)
        c.push(0x03); // call index (transfer_keep_alive)
        // dest: MultiAddress::Id(AccountId)
        c.push(0x00); // Id variant
        c.extend_from_slice(&dest_pubkey);
        // value: Compact<u128>
        c.extend_from_slice(&scale_compact_u128(amount));
        c
    };

    // Era: immortal (0x00).
    let era = vec![0x00u8];
    // Nonce: Compact<u32>.
    let nonce_enc = scale_compact_u32(nonce);
    // Tip: Compact<u128> = 0.
    let tip = scale_compact_u128(0);

    let genesis_bytes = decode_hash_hex(genesis_hash)?;
    let block_bytes = decode_hash_hex(block_hash)?;

    // Signing payload.
    let mut payload = Vec::new();
    payload.extend_from_slice(&call);
    payload.extend_from_slice(&era);
    payload.extend_from_slice(&nonce_enc);
    payload.extend_from_slice(&tip);
    payload.extend_from_slice(&spec_version.to_le_bytes());
    payload.extend_from_slice(&tx_version.to_le_bytes());
    payload.extend_from_slice(&genesis_bytes);
    payload.extend_from_slice(&block_bytes);

    // If payload > 256 bytes, sign its Blake2-256 hash instead.
    let signing_input: Vec<u8> = if payload.len() > 256 {
        blake2b_256(&payload).to_vec()
    } else {
        payload.clone()
    };

    let signing_key =
        SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&signing_input);

    // Build the extrinsic.
    // version = 0x84 (signed, version 4)
    let mut extrinsic_body = Vec::new();
    extrinsic_body.push(0x84); // version
    // signer: MultiAddress::Id(AccountId)
    extrinsic_body.push(0x00);
    extrinsic_body.extend_from_slice(public_key);
    // signature: MultiSignature::Ed25519(sig)
    extrinsic_body.push(0x00); // Ed25519 variant
    extrinsic_body.extend_from_slice(signature.to_bytes().as_ref());
    // extra (era, nonce, tip)
    extrinsic_body.extend_from_slice(&era);
    extrinsic_body.extend_from_slice(&nonce_enc);
    extrinsic_body.extend_from_slice(&tip);
    // call
    extrinsic_body.extend_from_slice(&call);

    // Prepend length (Compact<u32>).
    let mut out = scale_compact_u32(extrinsic_body.len() as u32);
    out.extend_from_slice(&extrinsic_body);
    Ok(out)
}

// ----------------------------------------------------------------
// SCALE codec helpers
// ----------------------------------------------------------------

fn scale_compact_u32(n: u32) -> Vec<u8> {
    scale_compact_u128(n as u128)
}

fn scale_compact_u128(n: u128) -> Vec<u8> {
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

fn decode_hash_hex(hex_str: &str) -> Result<[u8; 32], String> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(s).map_err(|e| format!("hash decode: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| format!("hash wrong length: {}", hex_str))
}

fn blake2b_256(data: &[u8]) -> [u8; 32] {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut h = Blake2b::<U32>::new();
    h.update(data);
    h.finalize().into()
}

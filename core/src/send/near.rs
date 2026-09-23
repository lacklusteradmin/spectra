//! NEAR send: BORSH-encoded Transfer + FunctionCall transaction builders,
//! Ed25519 signer, and `broadcast_tx_commit` RPC submit (plus rebroadcast).

use serde_json::json;

use crate::fetch::near::{NearClient, NearSendResult};

impl NearClient {
    /// Rebroadcast a pre-signed transaction (base64-encoded).
    pub async fn broadcast_signed_tx_b64(&self, tx_b64: &str) -> Result<NearSendResult, String> {
        let result = self.call("broadcast_tx_commit", json!([tx_b64])).await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(NearSendResult {
            txid,
            signed_tx_b64: tx_b64.to_string(),
        })
    }
}

// ── NEAR transaction builder (BORSH)

/// Build a signed NEAR Transfer transaction.
pub fn build_near_transfer_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let tx = borsh_encode_transfer(
        signer_id,
        public_key,
        nonce,
        receiver_id,
        yocto_amount,
        block_hash,
    );

    // Hash the transaction for signing.
    let tx_hash: [u8; 32] = Sha256::digest(&tx).into();

    let signing_key = SigningKey::from_bytes(private_key);
    if signing_key.verifying_key().as_bytes() != public_key {
        return Err("NEAR: public key does not match signer".into());
    }
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature
    // Signature in NEAR is: [key_type(1)] + [sig(64)]
    let mut signed = tx;
    signed.push(0); // key type = ED25519
    signed.extend_from_slice(signature.to_bytes().as_ref());

    Ok(signed)
}

/// BORSH-encode a NEAR Transfer transaction.
fn borsh_encode_transfer(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();

    // signer_id: string (u32 len + bytes)
    borsh_string(&mut out, signer_id);
    // public_key: key_type(u8) + bytes(32)
    out.push(0); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes());
    // Action::Transfer = variant 3
    out.push(3u8);
    // Transfer.deposit: u128
    out.extend_from_slice(&yocto_amount.to_le_bytes());

    out
}

/// Build a signed NEAR FunctionCall transaction (used for NEP-141 transfers).
#[allow(clippy::too_many_arguments)]
pub fn build_near_function_call_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    method_name: &str,
    args: &[u8],
    gas: u64,
    deposit: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let tx = borsh_encode_function_call(
        signer_id,
        public_key,
        nonce,
        receiver_id,
        method_name,
        args,
        gas,
        deposit,
        block_hash,
    );

    let tx_hash: [u8; 32] = Sha256::digest(&tx).into();
    let signing_key = SigningKey::from_bytes(private_key);
    if signing_key.verifying_key().as_bytes() != public_key {
        return Err("NEAR: public key does not match signer".into());
    }
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature (key_type(1) + sig(64))
    let mut signed = tx;
    signed.push(0); // ED25519
    signed.extend_from_slice(signature.to_bytes().as_ref());
    Ok(signed)
}

#[allow(clippy::too_many_arguments)]
fn borsh_encode_function_call(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    method_name: &str,
    args: &[u8],
    gas: u64,
    deposit: u128,
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();

    // signer_id: string
    borsh_string(&mut out, signer_id);
    // public_key: key_type(u8) + bytes(32)
    out.push(0); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes());
    // Action::FunctionCall = variant 2
    out.push(2u8);
    // method_name: string
    borsh_string(&mut out, method_name);
    // args: Vec<u8> (u32 len + bytes)
    out.extend_from_slice(&(args.len() as u32).to_le_bytes());
    out.extend_from_slice(args);
    // gas: u64
    out.extend_from_slice(&gas.to_le_bytes());
    // deposit: u128
    out.extend_from_slice(&deposit.to_le_bytes());

    out
}

fn borsh_string(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn near_transactions_match_official_sdk_vectors() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../testdata/protocol/transactions.json"))
                .unwrap();
        let public: [u8; 32] = hex::decode(fixtures["public_key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let transfer = build_near_transfer_tx(
            "alice.near",
            &public,
            42,
            "token.near",
            123456789,
            &[2; 32],
            &[1; 32],
        )
        .unwrap();
        let call = build_near_function_call_tx(
            "alice.near",
            &public,
            42,
            "token.near",
            "ft_transfer",
            br#"{"amount":"123456","receiver_id":"bob.near"}"#,
            30_000_000_000_000,
            1,
            &[2; 32],
            &[1; 32],
        )
        .unwrap();
        for (bytes, vector) in [transfer, call]
            .iter()
            .zip(fixtures["near"].as_array().unwrap())
        {
            assert_eq!(hex::encode(bytes), vector["signed_hex"].as_str().unwrap());
        }
        assert!(build_near_transfer_tx(
            "alice.near",
            &[9; 32],
            42,
            "token.near",
            1,
            &[2; 32],
            &[1; 32]
        )
        .is_err());
    }
}

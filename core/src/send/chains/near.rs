//! NEAR send: BORSH-encoded Transfer + FunctionCall transaction builders,
//! Ed25519 signer, and `broadcast_tx_commit` RPC submit (plus rebroadcast).

use serde_json::json;

use crate::fetch::chains::near::{NearClient, NearSendResult};

impl NearClient {
    /// Sign and broadcast a NEAR Transfer transaction.
    pub async fn sign_and_broadcast(
        &self,
        from_account_id: &str,
        to_account_id: &str,
        yocto_near: u128,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<NearSendResult, String> {
        let public_key_b58 = bs58::encode(public_key_bytes).into_string();
        let nonce = self
            .fetch_access_key_nonce(from_account_id, &public_key_b58)
            .await?
            + 1;
        let block_hash = self.fetch_latest_block_hash().await?;
        let block_hash_bytes = bs58::decode(&block_hash)
            .into_vec()
            .map_err(|e| format!("block hash decode: {e}"))?;
        if block_hash_bytes.len() != 32 {
            return Err("block hash wrong length".to_string());
        }
        let block_hash_arr: [u8; 32] = block_hash_bytes.try_into().unwrap();

        let tx_bytes = build_near_transfer_tx(
            from_account_id,
            public_key_bytes,
            nonce,
            to_account_id,
            yocto_near,
            &block_hash_arr,
            private_key_bytes,
        )?;

        use base64::Engine;
        let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

        let result = self.call("broadcast_tx_commit", json!([tx_b64])).await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .ok_or("broadcast: missing hash")?
            .to_string();
        Ok(NearSendResult { txid, signed_tx_b64: tx_b64 })
    }

    /// Rebroadcast a pre-signed transaction (base64-encoded).
    pub async fn broadcast_signed_tx_b64(&self, tx_b64: &str) -> Result<NearSendResult, String> {
        let result = self.call("broadcast_tx_commit", json!([tx_b64])).await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(NearSendResult { txid, signed_tx_b64: tx_b64.to_string() })
    }

    /// Sign and broadcast a NEP-141 `ft_transfer` call.
    ///
    /// Gas defaults to 30 TGas; `deposit` is the NEP-141-required exactly-1
    /// yoctoNEAR. The receiver must already have a storage deposit on the
    /// token contract — Spectra does not auto-register.
    pub async fn sign_and_broadcast_ft_transfer(
        &self,
        from_account_id: &str,
        token_contract: &str,
        to_account_id: &str,
        amount_raw: u128,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<NearSendResult, String> {
        let public_key_b58 = bs58::encode(public_key_bytes).into_string();
        let nonce = self
            .fetch_access_key_nonce(from_account_id, &public_key_b58)
            .await?
            + 1;
        let block_hash = self.fetch_latest_block_hash().await?;
        let block_hash_bytes = bs58::decode(&block_hash)
            .into_vec()
            .map_err(|e| format!("block hash decode: {e}"))?;
        if block_hash_bytes.len() != 32 {
            return Err("block hash wrong length".to_string());
        }
        let block_hash_arr: [u8; 32] = block_hash_bytes.try_into().unwrap();

        let args = json!({
            "receiver_id": to_account_id,
            "amount": amount_raw.to_string(),
        });
        let args_bytes = serde_json::to_vec(&args).map_err(|e| format!("args serialize: {e}"))?;

        let tx_bytes = build_near_function_call_tx(
            from_account_id,
            public_key_bytes,
            nonce,
            token_contract,
            "ft_transfer",
            &args_bytes,
            30_000_000_000_000, // 30 TGas
            1u128,              // exactly-1 yoctoNEAR (NEP-141 requirement)
            &block_hash_arr,
            private_key_bytes,
        )?;

        use base64::Engine;
        let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);
        let result = self.call("broadcast_tx_commit", json!([tx_b64])).await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .ok_or("broadcast: missing hash")?
            .to_string();
        Ok(NearSendResult { txid, signed_tx_b64: tx_b64 })
    }
}

// ----------------------------------------------------------------
// NEAR transaction builder (BORSH)
// ----------------------------------------------------------------

/// Build a signed NEAR Transfer transaction.
pub fn build_near_transfer_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    // BORSH-encode the transaction.
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

    let signing_key =
        SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature
    // Signature in NEAR is: [key_type(4)] + [sig(64)]
    let mut signed = tx;
    signed.extend_from_slice(&0u32.to_le_bytes()); // key type = ED25519
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
    // public_key: key_type(u32) + bytes(32)
    out.extend_from_slice(&0u32.to_le_bytes()); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes()); // 1 action
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
    private_key: &[u8; 64],
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
    let signing_key =
        SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature (key_type(4) + sig(64))
    let mut signed = tx;
    signed.extend_from_slice(&0u32.to_le_bytes()); // ED25519
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
    // public_key: key_type(u32) + bytes(32)
    out.extend_from_slice(&0u32.to_le_bytes()); // ED25519
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

//! Stellar send: XDR Payment builder (native + issued assets), Ed25519 signer,
//! and Horizon POST /transactions.

use crate::http::{with_fallback, RetryProfile};

use crate::derivation::chains::stellar::decode_stellar_address;
use crate::fetch::chains::stellar::{StellarClient, StellarSendResult};

impl StellarClient {
    /// Sign and submit a native XLM Payment transaction.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        stroops: i64,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
        network_passphrase: Option<Vec<u8>>,
    ) -> Result<StellarSendResult, String> {
        self.sign_and_submit_with_asset(
            from_address,
            to_address,
            stroops,
            StellarAsset::Native,
            private_key_bytes,
            public_key_bytes,
            network_passphrase,
        )
        .await
    }

    /// Sign and submit a custom-asset (credit_alphanum4 / credit_alphanum12)
    /// Payment transaction.
    pub async fn sign_and_submit_asset(
        &self,
        from_address: &str,
        to_address: &str,
        stroops: i64,
        asset_code: &str,
        asset_issuer: &str,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
        network_passphrase: Option<Vec<u8>>,
    ) -> Result<StellarSendResult, String> {
        let issuer_key = decode_stellar_address(asset_issuer)?;
        let code_len = asset_code.len();
        if code_len == 0 || code_len > 12 {
            return Err(format!("invalid asset code length: {code_len}"));
        }
        let asset = StellarAsset::Credit {
            code: asset_code.to_string(),
            issuer: issuer_key,
        };
        self.sign_and_submit_with_asset(
            from_address,
            to_address,
            stroops,
            asset,
            private_key_bytes,
            public_key_bytes,
            network_passphrase,
        )
        .await
    }

    async fn sign_and_submit_with_asset(
        &self,
        from_address: &str,
        to_address: &str,
        stroops: i64,
        asset: StellarAsset,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
        network_passphrase: Option<Vec<u8>>,
    ) -> Result<StellarSendResult, String> {
        let sequence = self.fetch_sequence(from_address).await? + 1;
        let base_fee = self.fetch_base_fee().await?;

        let default_passphrase = b"Public Global Stellar Network ; September 2015";
        let passphrase_bytes: &[u8] = network_passphrase.as_deref().unwrap_or(default_passphrase);
        let tx_xdr = build_signed_payment_xdr_with_asset(
            from_address,
            to_address,
            stroops,
            &asset,
            base_fee,
            sequence,
            passphrase_bytes,
            private_key_bytes,
            public_key_bytes,
        )?;

        // Submit via Horizon POST /transactions
        use base64::Engine;
        let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_xdr);

        let result = with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let tx_b64 = tx_b64.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: serde_json::Value = client
                    .post_json(
                        &url,
                        &serde_json::json!({"tx": tx_b64}),
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                let hash = resp
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .ok_or("submit: missing hash")?
                    .to_string();
                Ok(StellarSendResult { txid: hash, signed_xdr_b64: tx_b64.clone() })
            }
        })
        .await?;

        Ok(result)
    }

    /// Submit a pre-signed XDR envelope (for rebroadcast).
    pub async fn submit_envelope_b64(&self, tx_b64: &str) -> Result<StellarSendResult, String> {
        let tx_b64 = tx_b64.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let tx_b64 = tx_b64.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: serde_json::Value = client
                    .post_json(
                        &url,
                        &serde_json::json!({"tx": tx_b64}),
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                let hash = resp
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(StellarSendResult { txid: hash, signed_xdr_b64: tx_b64.clone() })
            }
        })
        .await
    }
}

// ----------------------------------------------------------------
// XDR transaction builder
// ----------------------------------------------------------------

/// Stellar Asset variants for Payment operations.
#[derive(Debug, Clone)]
pub enum StellarAsset {
    Native,
    /// `code` is 1-12 alphanumeric ASCII, `issuer` is the 32-byte ed25519 key.
    Credit { code: String, issuer: [u8; 32] },
}

/// Build a signed Stellar Payment transaction in XDR binary (native XLM).
#[allow(clippy::too_many_arguments)]
pub fn build_signed_payment_xdr(
    from: &str,
    to: &str,
    stroops: i64,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    build_signed_payment_xdr_with_asset(
        from,
        to,
        stroops,
        &StellarAsset::Native,
        base_fee,
        sequence,
        network_passphrase,
        private_key,
        public_key,
    )
}

/// Build a signed Stellar Payment transaction with an arbitrary asset.
#[allow(clippy::too_many_arguments)]
pub fn build_signed_payment_xdr_with_asset(
    from: &str,
    to: &str,
    stroops: i64,
    asset: &StellarAsset,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let _from_bytes = decode_stellar_address(from)?;
    let to_bytes = decode_stellar_address(to)?;

    // Network hash prefix for transaction signing.
    let network_hash: [u8; 32] = Sha256::digest(network_passphrase).into();

    // TransactionV0/Transaction XDR encoding (manual).
    let tx_xdr = encode_payment_tx(&to_bytes, stroops, asset, base_fee, sequence, public_key)?;

    // Signing payload: sha256(network_hash || ENVELOPE_TYPE_TX(2) || tx_xdr)
    let mut payload = Vec::new();
    payload.extend_from_slice(&network_hash);
    payload.extend_from_slice(&2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    payload.extend_from_slice(&tx_xdr);
    let sig_payload: [u8; 32] = Sha256::digest(&payload).into();

    let signing_key =
        SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&sig_payload);

    // TransactionEnvelope: type=ENVELOPE_TYPE_TX(2), tx, signatures
    let mut envelope = Vec::new();
    envelope.extend_from_slice(&2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    envelope.extend_from_slice(&tx_xdr);
    // DecoratedSignature array (1 item)
    envelope.extend_from_slice(&1u32.to_be_bytes()); // array length
    // hint = last 4 bytes of public key
    envelope.extend_from_slice(&public_key[28..32]);
    // signature (VarOpaque, max 64)
    xdr_write_bytes(&mut envelope, signature.to_bytes().as_ref());

    Ok(envelope)
}

fn encode_payment_tx(
    to: &[u8; 32],
    stroops: i64,
    asset: &StellarAsset,
    base_fee: u64,
    sequence: u64,
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    let mut tx = Vec::new();
    // sourceAccount: PUBLIC_KEY_TYPE_ED25519(0) + key
    tx.extend_from_slice(&0u32.to_be_bytes());
    tx.extend_from_slice(public_key);
    // fee (Uint32)
    tx.extend_from_slice(&(base_fee as u32).to_be_bytes());
    // seqNum (SequenceNumber = Int64)
    tx.extend_from_slice(&(sequence as i64).to_be_bytes());
    // timeBounds: optional=0 (none)
    tx.extend_from_slice(&0u32.to_be_bytes());
    // memo: MEMO_NONE=0
    tx.extend_from_slice(&0u32.to_be_bytes());
    // operations: array of 1
    tx.extend_from_slice(&1u32.to_be_bytes());
    // Operation: sourceAccount optional=0, type=PAYMENT(1)
    tx.extend_from_slice(&0u32.to_be_bytes()); // no source account override
    tx.extend_from_slice(&1u32.to_be_bytes()); // PAYMENT op type
    // PaymentOp: destination (PUBLIC_KEY_TYPE_ED25519 + key)
    tx.extend_from_slice(&0u32.to_be_bytes());
    tx.extend_from_slice(to);
    // asset
    encode_asset(&mut tx, asset)?;
    // amount: Int64
    tx.extend_from_slice(&stroops.to_be_bytes());
    // ext: 0
    tx.extend_from_slice(&0u32.to_be_bytes());
    Ok(tx)
}

/// Encode a Stellar Asset into XDR.
/// ASSET_TYPE_NATIVE=0, CREDIT_ALPHANUM4=1, CREDIT_ALPHANUM12=2.
fn encode_asset(tx: &mut Vec<u8>, asset: &StellarAsset) -> Result<(), String> {
    match asset {
        StellarAsset::Native => {
            tx.extend_from_slice(&0u32.to_be_bytes());
        }
        StellarAsset::Credit { code, issuer } => {
            let bytes = code.as_bytes();
            let len = bytes.len();
            if len == 0 || len > 12 {
                return Err(format!("asset code length {len} out of range"));
            }
            if !bytes.iter().all(|b| b.is_ascii_alphanumeric()) {
                return Err(format!("asset code contains non-alphanumeric: {code}"));
            }
            if len <= 4 {
                // ASSET_TYPE_CREDIT_ALPHANUM4 = 1
                tx.extend_from_slice(&1u32.to_be_bytes());
                // assetCode4: opaque[4] (fixed, right-padded with zeros)
                let mut code4 = [0u8; 4];
                code4[..len].copy_from_slice(bytes);
                tx.extend_from_slice(&code4);
            } else {
                // ASSET_TYPE_CREDIT_ALPHANUM12 = 2
                tx.extend_from_slice(&2u32.to_be_bytes());
                // assetCode12: opaque[12] (fixed, right-padded with zeros)
                let mut code12 = [0u8; 12];
                code12[..len].copy_from_slice(bytes);
                tx.extend_from_slice(&code12);
            }
            // issuer: AccountID (PUBLIC_KEY_TYPE_ED25519 + 32-byte key)
            tx.extend_from_slice(&0u32.to_be_bytes());
            tx.extend_from_slice(issuer);
        }
    }
    Ok(())
}

fn xdr_write_bytes(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    // XDR pads to 4-byte boundary
    let pad = (4 - (len % 4)) % 4;
    for _ in 0..pad {
        out.push(0);
    }
}

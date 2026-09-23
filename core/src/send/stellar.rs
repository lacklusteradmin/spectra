//! Stellar send: XDR Payment builder (native XLM), Ed25519 signer,
//! and Horizon POST /transactions.

use crate::fetch::http::{with_fallback, RetryProfile};

use crate::derivation::stellar::decode_stellar_address;
use crate::fetch::stellar::{StellarClient, StellarSendResult};

impl StellarClient {
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
                Ok(StellarSendResult {
                    txid: hash,
                    signed_xdr_b64: tx_b64.clone(),
                })
            }
        })
        .await
    }
}

// ── XDR transaction builder

/// Build a signed Stellar Payment transaction for native XLM.
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
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let _from_bytes = decode_stellar_address(from)?;
    let to_bytes = decode_stellar_address(to)?;

    // Network hash prefix for transaction signing.
    let network_hash: [u8; 32] = Sha256::digest(network_passphrase).into();

    // TransactionV0/Transaction XDR encoding (manual).
    let tx_xdr = encode_payment_tx(&to_bytes, stroops, base_fee, sequence, public_key);

    // Signing payload: sha256(network_hash || ENVELOPE_TYPE_TX(2) || tx_xdr)
    let mut payload = Vec::new();
    payload.extend_from_slice(&network_hash);
    payload.extend_from_slice(&2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    payload.extend_from_slice(&tx_xdr);
    let sig_payload: [u8; 32] = Sha256::digest(&payload).into();

    let signing_key = SigningKey::from_bytes(
        &private_key[..32]
            .try_into()
            .map_err(|_| "privkey too short")?,
    );
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
    base_fee: u64,
    sequence: u64,
    public_key: &[u8; 32],
) -> Vec<u8> {
    let mut tx = Vec::new();
    // sourceAccount: PUBLIC_KEY_TYPE_ED25519(0) + key
    tx.extend_from_slice(&0u32.to_be_bytes());
    tx.extend_from_slice(public_key);
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
    // ASSET_TYPE_NATIVE = 0
    tx.extend_from_slice(&0u32.to_be_bytes());
    // amount: Int64
    tx.extend_from_slice(&stroops.to_be_bytes());
    // ext: 0
    tx.extend_from_slice(&0u32.to_be_bytes());
    tx
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

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, SigningKey};
    use sha2::{Digest, Sha256};

    #[test]
    fn native_payment_envelope_encodes_amount_and_verifiable_signature() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let public = key.verifying_key().to_bytes();
        let mut strkey = vec![0x30];
        strkey.extend_from_slice(&public);
        let checksum = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM).checksum(&strkey);
        strkey.extend_from_slice(&checksum.to_le_bytes());
        let address = data_encoding::BASE32_NOPAD.encode(&strkey);
        let network = b"Test SDF Network ; September 2015";
        let envelope = build_signed_payment_xdr(
            &address,
            &address,
            12_345_678,
            100,
            42,
            network,
            &key.to_keypair_bytes(),
            &public,
        )
        .unwrap();
        // One native Payment and one decorated Ed25519 signature.
        assert_eq!(envelope.len(), 200);
        assert_eq!(&envelope[4..8], &0u32.to_be_bytes());
        assert_eq!(&envelope[8..40], &public);
        assert_eq!(&envelope[40..44], &100u32.to_be_bytes());
        assert_eq!(&envelope[44..52], &42u64.to_be_bytes());
        assert_eq!(&envelope[68..72], &1u32.to_be_bytes());
        assert_eq!(&envelope[76..108], &public);
        assert_eq!(&envelope[108..112], &0u32.to_be_bytes());
        assert_eq!(&envelope[112..120], &12_345_678i64.to_be_bytes());
        assert_eq!(&envelope[124..128], &1u32.to_be_bytes());
        let mut payload = Sha256::digest(network).to_vec();
        payload.extend_from_slice(&envelope[..124]);
        let signature = Signature::from_slice(&envelope[136..]).unwrap();
        key.verifying_key()
            .verify_strict(&Sha256::digest(payload), &signature)
            .unwrap();
    }
}

//! XRP send: build + sign Payment transactions (binary codec), submit,
//! and rebroadcast pre-signed blobs.

use serde_json::json;

use super::derive::decode_xrp_address;
use super::{XrpClient, XrpSendResult};

impl XrpClient {
    /// Sign and submit an XRP Payment transaction.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        drops: u64,
        private_key_bytes: &[u8],
        public_key_hex: &str,
    ) -> Result<XrpSendResult, String> {
        let sequence = self.fetch_sequence(from_address).await?;
        let fee = self.fetch_fee().await?;

        let tx_blob = build_signed_payment(
            from_address,
            to_address,
            drops,
            fee,
            sequence,
            private_key_bytes,
            public_key_hex,
        )?;

        let result = self.call("submit", json!({"tx_blob": tx_blob})).await?;
        let txid = result
            .get("tx_json")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .ok_or("submit: missing hash")?
            .to_string();
        Ok(XrpSendResult { txid, tx_blob_hex: tx_blob })
    }

    /// Submit a pre-signed transaction blob (for rebroadcast).
    pub async fn submit_signed_blob(&self, tx_blob_hex: &str) -> Result<XrpSendResult, String> {
        let result = self.call("submit", json!({"tx_blob": tx_blob_hex})).await?;
        let txid = result
            .get("tx_json")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(XrpSendResult { txid, tx_blob_hex: tx_blob_hex.to_string() })
    }
}

// ----------------------------------------------------------------
// XRP binary codec (minimal — Payment only)
// ----------------------------------------------------------------

/// Build and sign an XRP Payment transaction.
/// Returns the signed tx blob as an uppercase hex string.
pub fn build_signed_payment(
    from: &str,
    to: &str,
    amount_drops: u64,
    fee_drops: u64,
    sequence: u32,
    private_key_bytes: &[u8],
    public_key_hex: &str,
) -> Result<String, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    // Build signing payload (canonical field order per XRPL spec).
    let signing_prefix = hex::decode("53545800").unwrap(); // "STX\x00"

    let unsigned_fields = encode_payment_fields(from, to, amount_drops, fee_drops, sequence, public_key_hex)?;

    let mut signing_payload = signing_prefix.clone();
    signing_payload.extend_from_slice(&unsigned_fields);

    let msg_hash = sha512_half(&signing_payload);
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg = Message::from_digest_slice(&msg_hash).map_err(|e| format!("msg: {e}"))?;
    let sig = secp.sign_ecdsa(&msg, &secret_key);
    let der_sig = sig.serialize_der();
    let sig_hex = hex::encode_upper(der_sig.as_ref());

    // Rebuild with TxnSignature field inserted (field 16, type 7 = VL).
    let signed_fields = encode_payment_fields_signed(
        from, to, amount_drops, fee_drops, sequence, public_key_hex, &sig_hex,
    )?;

    Ok(hex::encode_upper(&signed_fields))
}

/// Encode the canonical Payment STObject fields (without TxnSignature).
fn encode_payment_fields(
    from: &str,
    to: &str,
    amount_drops: u64,
    fee_drops: u64,
    sequence: u32,
    public_key_hex: &str,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    // TransactionType = 0 (Payment), field 2, type 1 (UInt16)
    out.extend_from_slice(&[0x12, 0x00, 0x00]);
    // Flags, field 2, type 2 (UInt32) = 0
    out.extend_from_slice(&[0x22, 0x00, 0x00, 0x00, 0x00]);
    // Sequence, field 4, type 2
    out.push(0x24);
    out.extend_from_slice(&sequence.to_be_bytes());
    // Amount, field 1, type 6 (Amount)
    out.push(0x61);
    // XRP amount: 0x4000000000000000 | drops
    let amount_encoded: u64 = 0x4000_0000_0000_0000 | amount_drops;
    out.extend_from_slice(&amount_encoded.to_be_bytes());
    // Fee, field 8, type 6
    out.push(0x68);
    let fee_encoded: u64 = 0x4000_0000_0000_0000 | fee_drops;
    out.extend_from_slice(&fee_encoded.to_be_bytes());
    // SigningPubKey, field 3, type 7 (VL)
    out.push(0x73);
    let pk_bytes = hex::decode(public_key_hex).map_err(|e| format!("pubkey hex: {e}"))?;
    push_vl(&mut out, &pk_bytes);
    // Account (from), field 1, type 8 (AccountID)
    out.push(0x81);
    let from_bytes = decode_xrp_address(from)?;
    push_vl(&mut out, &from_bytes);
    // Destination (to), field 3, type 8
    out.push(0x83);
    let to_bytes = decode_xrp_address(to)?;
    push_vl(&mut out, &to_bytes);
    Ok(out)
}

fn encode_payment_fields_signed(
    from: &str,
    to: &str,
    amount_drops: u64,
    fee_drops: u64,
    sequence: u32,
    public_key_hex: &str,
    sig_hex: &str,
) -> Result<Vec<u8>, String> {
    let mut out = encode_payment_fields(from, to, amount_drops, fee_drops, sequence, public_key_hex)?;
    // TxnSignature, field 4, type 7
    out.push(0x74);
    let sig_bytes = hex::decode(sig_hex).map_err(|e| format!("sig hex: {e}"))?;
    push_vl(&mut out, &sig_bytes);
    Ok(out)
}

fn push_vl(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    if len < 193 {
        out.push(len as u8);
    } else {
        // Extended VL (simplified: only handles up to 12480 bytes)
        let adjusted = len - 193;
        out.push(193 + (adjusted / 256) as u8);
        out.push((adjusted % 256) as u8);
    }
    out.extend_from_slice(data);
}

fn sha512_half(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha512};
    let hash = Sha512::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hash[..32]);
    out
}

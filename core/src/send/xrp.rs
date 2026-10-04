//! XRP send: build + sign Payment transactions (binary codec).

use crate::send::error::SendError;

use crate::derivation::xrp::decode_xrp_address;

/// XRP's native amount encoding reserves its high bits for asset/sign flags.
/// The protocol also caps XRP amounts to the original supply, 10^17 drops.
pub(crate) fn validate_drops(drops: u128) -> Result<(), SendError> {
    if !(1..=100_000_000_000_000_000).contains(&drops) {
        return Err(SendError::Invalid(
            "XRP amount and fee must be positive and at most 100000000000 XRP".into(),
        ));
    }
    Ok(())
}

// ── XRP binary codec (minimal — Payment only)

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
) -> Result<String, SendError> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    // Build signing payload (canonical field order per XRPL spec).
    let signing_prefix = hex::decode("53545800").unwrap(); // "STX\x00"

    let unsigned_fields = encode_payment_fields(
        from,
        to,
        amount_drops,
        fee_drops,
        sequence,
        public_key_hex,
        None,
    )?;

    let mut signing_payload = signing_prefix.clone();
    signing_payload.extend_from_slice(&unsigned_fields);

    let msg_hash = sha512_half(&signing_payload);
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| SendError::Invalid(format!("invalid key: {e}").into()))?;
    let msg = Message::from_digest_slice(&msg_hash)
        .map_err(|e| SendError::Internal(format!("msg: {e}")))?;
    let sig = secp.sign_ecdsa(&msg, &secret_key);
    let der_sig = sig.serialize_der();
    // TxnSignature is a Blob (type 7, field 4), before AccountID fields.
    let signed_fields = encode_payment_fields(
        from,
        to,
        amount_drops,
        fee_drops,
        sequence,
        public_key_hex,
        Some(der_sig.as_ref()),
    )?;

    Ok(hex::encode_upper(&signed_fields))
}

/// Encode the canonical Payment STObject, optionally including its signature.
fn encode_payment_fields(
    from: &str,
    to: &str,
    amount_drops: u64,
    fee_drops: u64,
    sequence: u32,
    public_key_hex: &str,
    signature: Option<&[u8]>,
) -> Result<Vec<u8>, SendError> {
    validate_drops(u128::from(amount_drops))?;
    validate_drops(u128::from(fee_drops))?;
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
    let pk_bytes = hex::decode(public_key_hex)
        .map_err(|e| SendError::Invalid(format!("pubkey hex: {e}").into()))?;
    push_vl(&mut out, &pk_bytes);
    if let Some(signature) = signature {
        // TxnSignature, field 4, type 7 (VL)
        out.push(0x74);
        push_vl(&mut out, signature);
    }
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

#[cfg(test)]
#[path = "tests/xrp.rs"]
mod tests;

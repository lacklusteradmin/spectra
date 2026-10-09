//! XRP send: build + sign Payment (XRP or an issued currency), TrustSet
//! and AccountDelete transactions (binary codec).

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

// ── XRP binary codec (Payment, TrustSet and AccountDelete)

/// An issued-currency amount: a value of one issuer's currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssuedAmount {
    pub issue: crate::api::xrpl_amount::XrplIssue,
    pub value: crate::api::xrpl_amount::IouValue,
}

impl IssuedAmount {
    /// The 48 bytes of an `Amount` field: value, currency, issuer.
    fn encode(&self, out: &mut Vec<u8>) -> Result<(), SendError> {
        out.extend_from_slice(&self.value.to_bytes());
        out.extend_from_slice(&self.issue.currency.0);
        out.extend_from_slice(&decode_xrp_address(&self.issue.issuer)?);
        Ok(())
    }
}

/// What a Payment delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaymentAmount {
    Drops(u64),
    Issued(IssuedAmount),
}

/// The transaction types Spectra signs, by their `TransactionType` code.
#[derive(Clone)]
enum XrpTransaction<'a> {
    /// Delivers `amount` to `Destination`, spending at most `send_max` of
    /// the sender's issued currency when an issuer's transfer rate takes a
    /// share on the way.
    Payment {
        amount: &'a PaymentAmount,
        send_max: Option<&'a IssuedAmount>,
    },
    /// Deletes the account and sends everything it holds, less the fee, to
    /// `Destination`.
    AccountDelete,
    /// Sets the account's trust line to `limit`'s issuer and currency, with
    /// rippling through the account switched off. A limit of zero on an
    /// empty line removes it.
    TrustSet { limit: &'a IssuedAmount },
}

/// `tfSetNoRipple`: an account that is not an issuer does not let others'
/// payments pass through its trust lines.
const TF_SET_NO_RIPPLE: u32 = 0x0002_0000;

/// Build and sign an XRP Payment transaction.
/// Returns the signed tx blob as an uppercase hex string.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_signed_payment(
    from: &str,
    to: &str,
    destination_tag: Option<u32>,
    amount: &PaymentAmount,
    send_max: Option<&IssuedAmount>,
    fee_drops: u64,
    sequence: u32,
    private_key_bytes: &[u8],
    public_key_hex: &str,
) -> Result<String, SendError> {
    if let (PaymentAmount::Issued(amount), Some(send_max)) = (amount, send_max)
        && (amount.issue != send_max.issue || send_max.value < amount.value)
    {
        return Err(SendError::invalid(
            "XRP: the most a payment spends must cover what it delivers",
        ));
    }
    build_signed(
        XrpTransaction::Payment { amount, send_max },
        from,
        Some((to, destination_tag)),
        fee_drops,
        sequence,
        private_key_bytes,
        public_key_hex,
    )
}

/// Build and sign an XRP TrustSet: the account `from` trusts `limit`'s
/// issuer for up to its value of that currency.
pub(crate) fn build_signed_trust_set(
    from: &str,
    limit: &IssuedAmount,
    fee_drops: u64,
    sequence: u32,
    private_key_bytes: &[u8],
    public_key_hex: &str,
) -> Result<String, SendError> {
    if limit.value.negative || limit.issue.issuer == from {
        return Err(SendError::invalid(
            "XRP: a trust line is to another account, for a limit of zero or more",
        ));
    }
    build_signed(
        XrpTransaction::TrustSet { limit },
        from,
        None,
        fee_drops,
        sequence,
        private_key_bytes,
        public_key_hex,
    )
}

/// Build and sign an XRP AccountDelete transaction: the account `from` is
/// removed and its balance, less the fee, goes to `to`, with
/// `destination_tag` when the destination asks for one. The network charges
/// at least the owner reserve as its fee.
pub fn build_signed_account_delete(
    from: &str,
    to: &str,
    destination_tag: Option<u32>,
    fee_drops: u64,
    sequence: u32,
    private_key_bytes: &[u8],
    public_key_hex: &str,
) -> Result<String, SendError> {
    if from == to {
        return Err(SendError::Invalid(
            "XRP: an account cannot be deleted into itself".into(),
        ));
    }
    build_signed(
        XrpTransaction::AccountDelete,
        from,
        Some((to, destination_tag)),
        fee_drops,
        sequence,
        private_key_bytes,
        public_key_hex,
    )
}

/// `to` is the destination and its tag, when the transaction has one.
fn build_signed(
    transaction: XrpTransaction<'_>,
    from: &str,
    to: Option<(&str, Option<u32>)>,
    fee_drops: u64,
    sequence: u32,
    private_key_bytes: &[u8],
    public_key_hex: &str,
) -> Result<String, SendError> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let fields = |signature: Option<&[u8]>| {
        encode_fields(
            transaction.clone(),
            from,
            to,
            fee_drops,
            sequence,
            public_key_hex,
            signature,
        )
    };
    // Signing payload: "STX\0" and the canonical unsigned fields.
    let mut signing_payload = b"STX\0".to_vec();
    signing_payload.extend_from_slice(&fields(None)?);

    let msg_hash = sha512_half(&signing_payload);
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| SendError::Invalid(format!("invalid key: {e}").into()))?;
    let msg = Message::from_digest_slice(&msg_hash)
        .map_err(|e| SendError::Internal(format!("msg: {e}")))?;
    let sig = secp.sign_ecdsa(&msg, &secret_key);
    let der_sig = sig.serialize_der();
    Ok(hex::encode_upper(fields(Some(der_sig.as_ref()))?))
}

/// Encode the canonical STObject, optionally including its signature.
/// Fields go in type-code then field-code order.
fn encode_fields(
    transaction: XrpTransaction<'_>,
    from: &str,
    to: Option<(&str, Option<u32>)>,
    fee_drops: u64,
    sequence: u32,
    public_key_hex: &str,
    signature: Option<&[u8]>,
) -> Result<Vec<u8>, SendError> {
    validate_drops(u128::from(fee_drops))?;
    let mut out = Vec::new();
    // TransactionType, field 2, type 1 (UInt16): Payment 0, TrustSet 20,
    // AccountDelete 21.
    let (code, flags): (u16, u32) = match transaction {
        XrpTransaction::Payment { .. } => (0, 0),
        XrpTransaction::TrustSet { .. } => (20, TF_SET_NO_RIPPLE),
        XrpTransaction::AccountDelete => (21, 0),
    };
    out.push(0x12);
    out.extend_from_slice(&code.to_be_bytes());
    // Flags, field 2, type 2 (UInt32)
    out.push(0x22);
    out.extend_from_slice(&flags.to_be_bytes());
    // Sequence, field 4, type 2
    out.push(0x24);
    out.extend_from_slice(&sequence.to_be_bytes());
    if let Some((_, Some(tag))) = to {
        // DestinationTag, field 14, type 2
        out.push(0x2e);
        out.extend_from_slice(&tag.to_be_bytes());
    }
    match &transaction {
        XrpTransaction::Payment { amount, .. } => {
            // Amount, field 1, type 6 (Amount); XRP is 0x4000000000000000 | drops.
            out.push(0x61);
            match amount {
                PaymentAmount::Drops(drops) => {
                    validate_drops(u128::from(*drops))?;
                    out.extend_from_slice(&(0x4000_0000_0000_0000 | drops).to_be_bytes());
                }
                PaymentAmount::Issued(issued) => {
                    if issued.value.is_zero() || issued.value.negative {
                        return Err(SendError::invalid(
                            "XRP: a payment delivers a positive amount",
                        ));
                    }
                    issued.encode(&mut out)?;
                }
            }
        }
        XrpTransaction::TrustSet { limit } => {
            // LimitAmount, field 3, type 6
            out.push(0x63);
            limit.encode(&mut out)?;
        }
        XrpTransaction::AccountDelete => {}
    }
    // Fee, field 8, type 6
    out.push(0x68);
    let fee_encoded: u64 = 0x4000_0000_0000_0000 | fee_drops;
    out.extend_from_slice(&fee_encoded.to_be_bytes());
    if let XrpTransaction::Payment {
        send_max: Some(send_max),
        ..
    } = &transaction
    {
        // SendMax, field 9, type 6
        out.push(0x69);
        send_max.encode(&mut out)?;
    }
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
    if let Some((to, _)) = to {
        // Destination (to), field 3, type 8
        out.push(0x83);
        let to_bytes = decode_xrp_address(to)?;
        push_vl(&mut out, &to_bytes);
    }
    Ok(out)
}

pub(super) fn push_vl(out: &mut Vec<u8>, data: &[u8]) {
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

pub(super) fn sha512_half(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha512};
    let hash = Sha512::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hash[..32]);
    out
}

#[cfg(test)]
#[path = "tests/xrp.rs"]
mod tests;

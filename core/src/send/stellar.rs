//! Stellar send: XDR Payment (native XLM or a credit asset), ChangeTrust
//! and AccountMerge builders and Ed25519 signer.

use crate::api::stellar_asset::StellarAsset;
use crate::send::error::SendError;
use crate::send::payment_memo::StellarMemo;

use crate::derivation::stellar::decode_stellar_address;

// ── XDR transaction builder

/// The one operation a Spectra transaction carries.
enum StellarOperation<'a> {
    /// PAYMENT of native XLM, or of `asset` when there is one.
    Payment {
        to: &'a [u8; 32],
        asset: Option<&'a StellarAsset>,
        stroops: i64,
    },
    /// ACCOUNT_MERGE: the source account is removed and its balance, less
    /// the fee, goes to `to`.
    AccountMerge { to: &'a [u8; 32] },
    /// CHANGE_TRUST: hold up to `limit` of `asset`; a limit of zero on an
    /// empty trustline removes it.
    ChangeTrust { asset: &'a StellarAsset, limit: i64 },
}

/// The largest trustline limit, which the SDK sets when none is given.
pub(crate) const MAX_TRUST_LIMIT: i64 = i64::MAX;

/// Build a signed Stellar Payment transaction: native XLM, or `asset`, with
/// `memo` for the recipient.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_signed_payment_xdr(
    from: &str,
    to: &str,
    memo: Option<StellarMemo<'_>>,
    asset: Option<&StellarAsset>,
    stroops: i64,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, SendError> {
    let _from_bytes = decode_stellar_address(from)?;
    let to = decode_stellar_address(to)?;
    if stroops <= 0 {
        return Err(SendError::invalid(
            "Stellar: a payment moves a positive amount",
        ));
    }
    build_signed(
        StellarOperation::Payment {
            to: &to,
            asset,
            stroops,
        },
        memo,
        base_fee,
        sequence,
        network_passphrase,
        private_key,
        public_key,
    )
}

/// Build a signed Stellar ChangeTrust: the account `from` holds up to
/// `limit` stroops of `asset`.
pub(crate) fn build_signed_change_trust_xdr(
    from: &str,
    asset: &StellarAsset,
    limit: i64,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, SendError> {
    if limit < 0 || asset.issuer == from {
        return Err(SendError::invalid(
            "Stellar: a trustline is to another account, for a limit of zero or more",
        ));
    }
    build_signed(
        StellarOperation::ChangeTrust { asset, limit },
        None,
        base_fee,
        sequence,
        network_passphrase,
        private_key,
        public_key,
    )
}

/// Build a signed Stellar AccountMerge transaction: the account `from` is
/// removed and everything it holds, less the fee, goes to `to`, with `memo`
/// when the destination asks for one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_signed_account_merge_xdr(
    from: &str,
    to: &str,
    memo: Option<StellarMemo<'_>>,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, SendError> {
    let from = decode_stellar_address(from)?;
    let to = decode_stellar_address(to)?;
    if from == to {
        return Err(SendError::Invalid(
            "Stellar: an account cannot be merged into itself".into(),
        ));
    }
    build_signed(
        StellarOperation::AccountMerge { to: &to },
        memo,
        base_fee,
        sequence,
        network_passphrase,
        private_key,
        public_key,
    )
}

fn build_signed(
    operation: StellarOperation<'_>,
    memo: Option<StellarMemo<'_>>,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, SendError> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    // Network hash prefix for transaction signing.
    let network_hash: [u8; 32] = Sha256::digest(network_passphrase).into();

    // TransactionV0/Transaction XDR encoding (manual).
    let tx_xdr = encode_tx(&operation, memo, base_fee, sequence, public_key)?;

    // Signing payload: sha256(network_hash || ENVELOPE_TYPE_TX(2) || tx_xdr)
    let mut payload = Vec::new();
    payload.extend_from_slice(&network_hash);
    payload.extend_from_slice(&2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    payload.extend_from_slice(&tx_xdr);
    let sig_payload: [u8; 32] = Sha256::digest(&payload).into();

    let signing_key = SigningKey::from_bytes(
        &private_key[..32]
            .try_into()
            .map_err(|_| SendError::Invalid("privkey too short".into()))?,
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

/// An `Asset`: native, or `credit_alphanum4`/`12` with the code padded
/// with zeros and the issuer's key.
fn encode_asset(tx: &mut Vec<u8>, asset: Option<&StellarAsset>) -> Result<(), SendError> {
    let Some(asset) = asset else {
        tx.extend_from_slice(&0u32.to_be_bytes()); // ASSET_TYPE_NATIVE
        return Ok(());
    };
    let width = if asset.code.len() <= 4 { 4 } else { 12 };
    tx.extend_from_slice(&(if width == 4 { 1u32 } else { 2u32 }).to_be_bytes());
    let mut code = vec![0u8; width];
    code[..asset.code.len()].copy_from_slice(asset.code.as_bytes());
    tx.extend_from_slice(&code);
    tx.extend_from_slice(&0u32.to_be_bytes()); // PUBLIC_KEY_TYPE_ED25519
    tx.extend_from_slice(&decode_stellar_address(&asset.issuer)?);
    Ok(())
}

fn encode_tx(
    operation: &StellarOperation<'_>,
    memo: Option<StellarMemo<'_>>,
    base_fee: u64,
    sequence: u64,
    public_key: &[u8; 32],
) -> Result<Vec<u8>, SendError> {
    let mut tx = Vec::new();
    // sourceAccount: PUBLIC_KEY_TYPE_ED25519(0) + key
    tx.extend_from_slice(&0u32.to_be_bytes());
    tx.extend_from_slice(public_key);
    tx.extend_from_slice(&(base_fee as u32).to_be_bytes());
    // seqNum (SequenceNumber = Int64)
    tx.extend_from_slice(&(sequence as i64).to_be_bytes());
    // timeBounds: optional=0 (none)
    tx.extend_from_slice(&0u32.to_be_bytes());
    match memo {
        // MEMO_NONE
        None => tx.extend_from_slice(&0u32.to_be_bytes()),
        // MEMO_TEXT: string<28>
        Some(StellarMemo::Text(text)) => {
            if text.is_empty() || text.len() > 28 {
                return Err(SendError::invalid("Stellar: a text memo is 1 to 28 bytes"));
            }
            tx.extend_from_slice(&1u32.to_be_bytes());
            xdr_write_bytes(&mut tx, text.as_bytes());
        }
        // MEMO_ID: uint64
        Some(StellarMemo::Id(id)) => {
            tx.extend_from_slice(&2u32.to_be_bytes());
            tx.extend_from_slice(&id.to_be_bytes());
        }
    }
    // operations: array of 1
    tx.extend_from_slice(&1u32.to_be_bytes());
    // Operation: sourceAccount optional=0 (no override)
    tx.extend_from_slice(&0u32.to_be_bytes());
    match operation {
        StellarOperation::Payment { to, asset, stroops } => {
            tx.extend_from_slice(&1u32.to_be_bytes()); // PAYMENT op type
            // PaymentOp: destination (PUBLIC_KEY_TYPE_ED25519 + key)
            tx.extend_from_slice(&0u32.to_be_bytes());
            tx.extend_from_slice(*to);
            encode_asset(&mut tx, *asset)?;
            // amount: Int64
            tx.extend_from_slice(&stroops.to_be_bytes());
        }
        StellarOperation::ChangeTrust { asset, limit } => {
            tx.extend_from_slice(&6u32.to_be_bytes()); // CHANGE_TRUST op type
            // ChangeTrustAsset: a credit asset, as an Asset.
            encode_asset(&mut tx, Some(asset))?;
            tx.extend_from_slice(&limit.to_be_bytes());
        }
        StellarOperation::AccountMerge { to } => {
            tx.extend_from_slice(&8u32.to_be_bytes()); // ACCOUNT_MERGE op type
            // destination: MuxedAccount KEY_TYPE_ED25519 + key
            tx.extend_from_slice(&0u32.to_be_bytes());
            tx.extend_from_slice(*to);
        }
    }
    // ext: 0
    tx.extend_from_slice(&0u32.to_be_bytes());
    Ok(tx)
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
            None,
            None,
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

    /// AccountMerge exactly as the Stellar SDK builds and signs it.
    #[test]
    fn account_merge_matches_the_stellar_sdk() {
        use base64::Engine;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/account-closing.json"))
                .unwrap();
        let vector = &fixture["stellar_account_merge"];
        let seed: [u8; 32] = hex::decode(vector["seed"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        let public = key.verifying_key().to_bytes();
        let source = vector["source"].as_str().unwrap();
        let envelope = build_signed_account_merge_xdr(
            source,
            vector["destination"].as_str().unwrap(),
            None,
            vector["fee"].as_str().unwrap().parse().unwrap(),
            vector["sequence"].as_str().unwrap().parse().unwrap(),
            vector["network_passphrase"].as_str().unwrap().as_bytes(),
            &key.to_keypair_bytes(),
            &public,
        )
        .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(envelope),
            vector["envelope_b64"].as_str().unwrap()
        );
        assert!(
            build_signed_account_merge_xdr(
                source,
                source,
                None,
                100,
                1,
                b"Test SDF Network ; September 2015",
                &key.to_keypair_bytes(),
                &public,
            )
            .is_err()
        );
    }

    /// Text and ID memos exactly as the Stellar SDK encodes them, on native
    /// and credit-asset payments and on AccountMerge.
    #[test]
    fn memos_match_the_stellar_sdk() {
        use crate::registry::PaymentMemoKind;
        use crate::send::payment_memo::PaymentMemo;
        use base64::Engine;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/payment-memos.json")).unwrap();
        let vectors = &fixture["stellar"];
        let seed: [u8; 32] = hex::decode(vectors["seed"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        let public = key.verifying_key().to_bytes();
        let text = |value: &serde_json::Value| value.as_str().unwrap().to_string();
        let passphrase = text(&vectors["network_passphrase"]);
        let memo = |vector: &serde_json::Value| PaymentMemo {
            kind: serde_json::from_value::<PaymentMemoKind>(vector["kind"].clone()).unwrap(),
            value: text(&vector["memo"]),
        };
        let encode = base64::engine::general_purpose::STANDARD;
        for vector in vectors["payments"].as_array().unwrap() {
            let memo = memo(vector)
                .validated(crate::registry::Chain::StellarTestnet)
                .unwrap();
            let asset = vector["asset"]
                .as_str()
                .map(|asset| StellarAsset::parse(asset).unwrap());
            let envelope = build_signed_payment_xdr(
                &text(&vectors["source"]),
                &text(&vectors["destination"]),
                PaymentMemo::stellar(Some(&memo)).unwrap(),
                asset.as_ref(),
                crate::decimal::to_units(&text(&vector["amount"]), 7).unwrap() as i64,
                100,
                text(&vector["sequence"]).parse().unwrap(),
                passphrase.as_bytes(),
                &key.to_keypair_bytes(),
                &public,
            )
            .unwrap();
            assert_eq!(
                encode.encode(envelope),
                text(&vector["envelope_b64"]),
                "{vector}"
            );
        }
        let vector = &vectors["account_merge"];
        let memo = memo(vector);
        let envelope = build_signed_account_merge_xdr(
            &text(&vectors["source"]),
            &text(&vectors["destination"]),
            PaymentMemo::stellar(Some(&memo)).unwrap(),
            100,
            text(&vector["sequence"]).parse().unwrap(),
            passphrase.as_bytes(),
            &key.to_keypair_bytes(),
            &public,
        )
        .unwrap();
        assert_eq!(encode.encode(envelope), text(&vector["envelope_b64"]));
    }

    /// Credit-asset payments and trustlines exactly as the Stellar SDK
    /// builds and signs them, for four- and twelve-character codes.
    #[test]
    fn credit_assets_match_the_stellar_sdk() {
        use base64::Engine;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/issued-assets.json")).unwrap();
        let vectors = &fixture["stellar"];
        let seed: [u8; 32] = hex::decode(vectors["seed"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        let public = key.verifying_key().to_bytes();
        let text = |value: &serde_json::Value| value.as_str().unwrap().to_string();
        let passphrase = text(&vectors["network_passphrase"]);
        let stroops = |amount: &serde_json::Value| {
            crate::decimal::to_units(amount.as_str().unwrap(), 7).unwrap() as i64
        };
        let encode = base64::engine::general_purpose::STANDARD;
        for vector in vectors["payments"].as_array().unwrap() {
            let asset = StellarAsset::parse(&text(&vector["asset"])).unwrap();
            let envelope = build_signed_payment_xdr(
                &text(&vectors["source"]),
                &text(&vectors["destination"]),
                None,
                Some(&asset),
                stroops(&vector["amount"]),
                100,
                text(&vector["sequence"]).parse().unwrap(),
                passphrase.as_bytes(),
                &key.to_keypair_bytes(),
                &public,
            )
            .unwrap();
            assert_eq!(encode.encode(envelope), text(&vector["envelope_b64"]));
        }
        for vector in vectors["trustlines"].as_array().unwrap() {
            let asset = StellarAsset::parse(&text(&vector["asset"])).unwrap();
            let envelope = build_signed_change_trust_xdr(
                &text(&vectors["source"]),
                &asset,
                stroops(&vector["limit"]),
                100,
                text(&vector["sequence"]).parse().unwrap(),
                passphrase.as_bytes(),
                &key.to_keypair_bytes(),
                &public,
            )
            .unwrap();
            assert_eq!(encode.encode(envelope), text(&vector["envelope_b64"]));
        }
        assert_eq!(
            stroops(&vectors["trustlines"][0]["limit"]),
            MAX_TRUST_LIMIT,
            "the SDK's default limit is the largest"
        );
        let own = StellarAsset::new("OWN", &text(&vectors["source"])).unwrap();
        assert!(
            build_signed_change_trust_xdr(
                &text(&vectors["source"]),
                &own,
                1,
                100,
                1,
                passphrase.as_bytes(),
                &key.to_keypair_bytes(),
                &public,
            )
            .is_err()
        );
    }
}

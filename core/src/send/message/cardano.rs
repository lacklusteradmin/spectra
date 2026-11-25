//! CIP-8 as CIP-30's `signData` returns it: a COSE_Sign1 whose protected
//! header names EdDSA and the address, over the message as its payload,
//! and the COSE_Key of the payment key; both hex, in CIP-30's
//! `{"signature", "key"}` object.

use super::*;

fn bytes_header(length: usize, major: u8) -> Vec<u8> {
    let major = major << 5;
    match length {
        0..=23 => vec![major | length as u8],
        24..=0xff => vec![major | 24, length as u8],
        0x100..=0xffff => [vec![major | 25], (length as u16).to_be_bytes().to_vec()].concat(),
        _ => [vec![major | 26], (length as u32).to_be_bytes().to_vec()].concat(),
    }
}

fn bstr(bytes: &[u8]) -> Vec<u8> {
    [bytes_header(bytes.len(), 2), bytes.to_vec()].concat()
}

/// `{1: -8, "address": address}`: EdDSA and the signing address.
fn protected(address: &[u8]) -> Vec<u8> {
    [&[0xa2, 0x01, 0x27, 0x67][..], b"address", &bstr(address)].concat()
}

/// `["Signature1", protected, h'', payload]`, what the key signs.
fn sig_structure(protected: &[u8], payload: &[u8]) -> Vec<u8> {
    [
        &[0x84, 0x6a][..],
        b"Signature1",
        &bstr(protected),
        &[0x40],
        &bstr(payload),
    ]
    .concat()
}

/// The address's payment key hash, where its payment credential is a
/// key rather than a script.
fn payment_key_hash(address: &[u8]) -> Option<&[u8]> {
    let header = *address.first()?;
    (header >> 4 <= 7 && header & 0x10 == 0).then(|| address.get(1..29))?
}

fn key_hash(public: &[u8; 32]) -> [u8; 28] {
    crate::derivation::primitives::blake2b::<28>(&[public])
}

/// Whether `address` signs with a payment key at all.
pub(super) fn signs(address: &str) -> bool {
    crate::derivation::cardano::decode_cardano_addr_bytes(address)
        .is_ok_and(|bytes| payment_key_hash(&bytes).is_some())
}

pub(super) fn sign(key: &[u8; 64], address: &str, message: &str) -> Result<String, SendError> {
    let address = crate::derivation::cardano::decode_cardano_addr_bytes(address)
        .map_err(|e| SendError::Invalid(e.to_string().into()))?;
    let public = crate::derivation::ed25519_extended::public_key(key)
        .map_err(|e| SendError::Invalid(e.to_string().into()))?;
    if payment_key_hash(&address) != Some(key_hash(&public).as_slice()) {
        return Err(SendError::Invalid(
            "This key is not the address's payment key".into(),
        ));
    }
    let protected = protected(&address);
    let signature = crate::derivation::ed25519_extended::sign(
        key,
        &public,
        &sig_structure(&protected, message.as_bytes()),
    )?;
    let sign1 = [
        &[0x84][..],
        &bstr(&protected),
        // {"hashed": false}: the payload is the message itself.
        &[0xa1, 0x66],
        b"hashed",
        &[0xf4],
        &bstr(message.as_bytes()),
        &bstr(&signature),
    ]
    .concat();
    // {1: 1 (OKP), 3: -8 (EdDSA), -1: 6 (Ed25519), -2: the key}.
    let cose_key = [
        &[0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21][..],
        &bstr(&public),
    ]
    .concat();
    // CIP-30's DataSignature, signature first.
    Ok(format!(
        r#"{{"signature":"{}","key":"{}"}}"#,
        hex::encode(sign1),
        hex::encode(cose_key)
    ))
}

pub(super) fn verify(address: &str, message: &str, signature: &str) -> bool {
    verified(address, message, signature).unwrap_or(false)
}

fn verified(address: &str, message: &str, signature: &str) -> Option<bool> {
    use ciborium::Value;
    let address = crate::derivation::cardano::decode_cardano_addr_bytes(address).ok()?;
    let object: serde_json::Value = serde_json::from_str(signature).ok()?;
    let decode = |field: &str| -> Option<Value> {
        let bytes = hex::decode(object.get(field)?.as_str()?).ok()?;
        ciborium::from_reader(bytes.as_slice()).ok()
    };
    let entry = |map: &[(Value, Value)], key: Value| {
        map.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    };
    let Value::Map(cose_key) = decode("key")? else {
        return None;
    };
    let public: [u8; 32] = entry(&cose_key, Value::from(-2))?
        .into_bytes()
        .ok()?
        .try_into()
        .ok()?;
    if entry(&cose_key, Value::from(1)) != Some(Value::from(1))
        || entry(&cose_key, Value::from(3)) != Some(Value::from(-8))
        || payment_key_hash(&address) != Some(key_hash(&public).as_slice())
    {
        return Some(false);
    }
    let sign1 = match decode("signature")? {
        Value::Tag(18, inner) => *inner,
        value => value,
    };
    let Value::Array(parts) = sign1 else {
        return None;
    };
    let [protected, unprotected, payload, signature] = parts.as_slice() else {
        return None;
    };
    let protected = protected.as_bytes()?;
    let Value::Map(headers) = ciborium::from_reader::<Value, _>(protected.as_slice()).ok()? else {
        return None;
    };
    let hashed = unprotected
        .as_map()
        .and_then(|map| entry(map, Value::from("hashed")))
        .is_some_and(|hashed| hashed == Value::from(true));
    Some(
        !hashed
            && entry(&headers, Value::from(1)) == Some(Value::from(-8))
            && entry(&headers, Value::from("address")) == Some(Value::Bytes(address))
            && payload
                .as_bytes()
                .is_some_and(|payload| payload == message.as_bytes())
            && ed25519_verify(
                &public,
                &sig_structure(protected, message.as_bytes()),
                signature.as_bytes()?,
            ),
    )
}

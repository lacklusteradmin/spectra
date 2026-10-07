//! The message schemes of networks whose wallets sign apart from a
//! transaction format of their own: Stellar's SEP-53, Cardano's CIP-8 data
//! signatures as CIP-30's `signData` returns them, Kaspa's personal
//! messages and Monero's `SigV2`. Each binds the text to the purpose with a
//! prefix, a keyed hash or a COSE structure, so no signature here can
//! authorise a transaction.

use super::*;

/// SEP-53: Ed25519 over SHA-256 of the prefix and the message, in base64.
pub(super) mod stellar {
    use super::*;

    fn digest(message: &str) -> [u8; 32] {
        Sha256::digest([b"Stellar Signed Message:\n".as_slice(), message.as_bytes()].concat())
            .into()
    }

    pub(in crate::send) fn sign(seed: &[u8; 32], message: &str) -> String {
        use ed25519_dalek::Signer as _;
        let key = ed25519_dalek::SigningKey::from_bytes(seed);
        base64_encode(&key.sign(&digest(message)).to_bytes())
    }

    pub(in crate::send) fn verify(address: &str, message: &str, signature: &str) -> bool {
        let (Ok(public), Some(signature)) = (
            crate::derivation::stellar::decode_stellar_address(address),
            base64_decode(signature),
        ) else {
            return false;
        };
        ed25519_verify(&public, &digest(message), &signature)
    }
}

/// CIP-8 as CIP-30's `signData` returns it: a COSE_Sign1 whose protected
/// header names EdDSA and the address, over the message as its payload,
/// and the COSE_Key of the payment key; both hex, in CIP-30's
/// `{"signature", "key"}` object.
pub(super) mod cardano {
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
        let hash = blake2b_simd::Params::new().hash_length(28).hash(public);
        hash.as_bytes().try_into().expect("28-byte hash")
    }

    /// Whether `address` signs with a payment key at all.
    pub(in crate::send) fn signs(address: &str) -> bool {
        crate::derivation::cardano::decode_cardano_addr_bytes(address)
            .is_ok_and(|bytes| payment_key_hash(&bytes).is_some())
    }

    pub(in crate::send) fn sign(
        key: &[u8; 64],
        address: &str,
        message: &str,
    ) -> Result<String, SendError> {
        let address = crate::derivation::cardano::decode_cardano_addr_bytes(address)
            .map_err(|e| SendError::Invalid(e.to_string().into()))?;
        let public = crate::derivation::cardano::public_from_extended_key(key)
            .map_err(|e| SendError::Invalid(e.to_string().into()))?;
        if payment_key_hash(&address) != Some(key_hash(&public).as_slice()) {
            return Err(SendError::Invalid(
                "This key is not the address's payment key".into(),
            ));
        }
        let protected = protected(&address);
        let signature = crate::send::cardano::sign_extended(
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

    pub(in crate::send) fn verify(address: &str, message: &str, signature: &str) -> bool {
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
        let Value::Map(headers) = ciborium::from_reader::<Value, _>(protected.as_slice()).ok()?
        else {
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
}

/// Kaspa's personal messages: BIP-340 Schnorr over the message's Blake2b
/// hash keyed `PersonalMessageSigningHash`, in hex, for Schnorr addresses.
pub(super) mod kaspa {
    use super::*;

    fn digest(message: &str) -> [u8; 32] {
        blake2b_simd::Params::new()
            .hash_length(32)
            .key(b"PersonalMessageSigningHash")
            .hash(message.as_bytes())
            .as_bytes()
            .try_into()
            .expect("32-byte hash")
    }

    /// The x-only key of a Schnorr address on `chain`'s network.
    fn public(chain: Chain, address: &str) -> Option<secp256k1::XOnlyPublicKey> {
        let (version, payload, testnet) =
            crate::derivation::kaspa::decode_kaspa_address(address).ok()?;
        (version == 0 && testnet == chain.is_testnet())
            .then(|| secp256k1::XOnlyPublicKey::from_slice(&payload).ok())?
    }

    pub(in crate::send) fn signs(chain: Chain, address: &str) -> bool {
        public(chain, address).is_some()
    }

    pub(in crate::send) fn sign(seed: &[u8; 32], message: &str) -> Result<String, SendError> {
        let secp = Secp256k1::new();
        let keypair = secp256k1::Keypair::from_secret_key(&secp, &secret_key(seed)?);
        let signature =
            secp.sign_schnorr_no_aux_rand(&Message::from_digest(digest(message)), &keypair);
        Ok(hex::encode(signature.serialize()))
    }

    pub(in crate::send) fn verify(
        chain: Chain,
        address: &str,
        message: &str,
        signature: &str,
    ) -> bool {
        let (Some(public), Some(signature)) = (
            public(chain, address),
            hex::decode(signature)
                .ok()
                .and_then(|bytes| secp256k1::schnorr::Signature::from_slice(&bytes).ok()),
        ) else {
            return false;
        };
        Secp256k1::verification_only()
            .verify_schnorr(&signature, &Message::from_digest(digest(message)), &public)
            .is_ok()
    }
}

/// Monero's `SigV2`: wallet2's Schnorr signature with the spend key over a
/// hash bound to the address's keys, `SigV2` and Monero base58 of `c ‖ r`.
pub(super) mod monero {
    use super::*;
    use curve25519_dalek::{
        constants::ED25519_BASEPOINT_TABLE, edwards::CompressedEdwardsY, scalar::Scalar,
    };
    use monero_wallet::address::MoneroAddress;

    /// Signed with the spend key (0) or the view key (1).
    fn message_hash(message: &str, spend: &[u8; 32], view: &[u8; 32], mode: u8) -> [u8; 32] {
        let mut data = b"MoneroMessageSignature\0".to_vec();
        data.extend(spend);
        data.extend(view);
        data.push(mode);
        let mut length = message.len();
        loop {
            let byte = (length & 0x7f) as u8;
            length >>= 7;
            if length == 0 {
                data.push(byte);
                break;
            }
            data.push(byte | 0x80);
        }
        data.extend(message.as_bytes());
        monero_wallet::primitives::keccak256(&data)
    }

    fn challenge(hash: &[u8; 32], public: &[u8; 32], commitment: &[u8; 32]) -> Scalar {
        Scalar::from_bytes_mod_order(monero_wallet::primitives::keccak256(
            [hash.as_slice(), public, commitment].concat(),
        ))
    }

    fn keys(address: &str) -> Option<([u8; 32], [u8; 32])> {
        let address = MoneroAddress::from_str_with_unchecked_network(address).ok()?;
        Some((
            address.spend().compress().to_bytes(),
            address.view().compress().to_bytes(),
        ))
    }

    pub(in crate::send) fn signs(address: &str) -> bool {
        keys(address).is_some()
    }

    pub(in crate::send) fn sign(
        spend_secret: &[u8; 32],
        address: &str,
        message: &str,
    ) -> Result<String, SendError> {
        let (spend, view) =
            keys(address).ok_or_else(|| SendError::Invalid("Not a Monero address".into()))?;
        let secret = Option::<Scalar>::from(Scalar::from_canonical_bytes(*spend_secret))
            .ok_or_else(|| SendError::Invalid("Not a Monero spend key".into()))?;
        if (&secret * ED25519_BASEPOINT_TABLE).compress().to_bytes() != spend {
            return Err(SendError::Invalid(
                "This key is not the address's spend key".into(),
            ));
        }
        let hash = message_hash(message, &spend, &view, 0);
        let mut wide = zeroize::Zeroizing::new([0u8; 64]);
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, wide.as_mut());
        let nonce = Scalar::from_bytes_mod_order_wide(&wide);
        let commitment = (&nonce * ED25519_BASEPOINT_TABLE).compress().to_bytes();
        let c = challenge(&hash, &spend, &commitment);
        let r = nonce - c * secret;
        let signature = [c.to_bytes(), r.to_bytes()].concat();
        Ok(format!(
            "SigV2{}",
            crate::derivation::monero::monero_base58_encode(&signature)
        ))
    }

    pub(in crate::send) fn verify(address: &str, message: &str, signature: &str) -> bool {
        let (Some((spend, view)), Some(bytes)) = (
            keys(address),
            signature
                .strip_prefix("SigV2")
                .and_then(crate::derivation::monero::monero_base58_decode),
        ) else {
            return false;
        };
        let Ok::<[u8; 64], _>(bytes) = bytes.try_into() else {
            return false;
        };
        let (Some(c), Some(r)) = (
            Option::<Scalar>::from(Scalar::from_canonical_bytes(
                bytes[..32].try_into().unwrap(),
            )),
            Option::<Scalar>::from(Scalar::from_canonical_bytes(
                bytes[32..].try_into().unwrap(),
            )),
        ) else {
            return false;
        };
        // Either key may have signed; the hash names which.
        [(0u8, spend), (1u8, view)]
            .into_iter()
            .any(|(mode, public)| {
                let Some(point) = CompressedEdwardsY(public).decompress() else {
                    return false;
                };
                let hash = message_hash(message, &spend, &view, mode);
                let commitment = (&r * ED25519_BASEPOINT_TABLE + c * point)
                    .compress()
                    .to_bytes();
                challenge(&hash, &public, &commitment) == c
            })
    }
}

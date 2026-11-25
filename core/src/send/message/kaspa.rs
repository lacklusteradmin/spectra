//! Kaspa's personal messages: BIP-340 Schnorr over the message's Blake2b
//! hash keyed `PersonalMessageSigningHash`, in hex, for Schnorr addresses.

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

pub(super) fn signs(chain: Chain, address: &str) -> bool {
    public(chain, address).is_some()
}

pub(super) fn sign(seed: &[u8; 32], message: &str) -> Result<String, SendError> {
    let secp = Secp256k1::new();
    let keypair = secp256k1::Keypair::from_secret_key(&secp, &secret_key(seed)?);
    let signature = secp.sign_schnorr_no_aux_rand(&Message::from_digest(digest(message)), &keypair);
    Ok(hex::encode(signature.serialize()))
}

pub(super) fn verify(chain: Chain, address: &str, message: &str, signature: &str) -> bool {
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

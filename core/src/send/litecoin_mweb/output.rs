//! Outputs: paying a stealth address, and recognizing the wallet's own.
//!
//! The sender picks a key `kₛ`; with `n = BLAKE3('N' ‖ kₛ)[..16]` and
//! `s = BLAKE3('S' ‖ Aᵢ ‖ Bᵢ ‖ v ‖ n)` (v as eight little-endian bytes), the
//! shared secret is `t = BLAKE3('D' ‖ s·Aᵢ)`. The output's key is
//! `Kₒ = Bᵢ·BLAKE3('O' ‖ t)`, its key exchange `Kₑ = s·Bᵢ`, its view tag the
//! first byte of `BLAKE3('T' ‖ s·Aᵢ)`; the value and nonce travel masked by
//! `BLAKE3('Y' ‖ t)` and `BLAKE3('X' ‖ t)`, and the commitment is a switch
//! commitment blinded by `BLAKE3('B' ‖ t)`. The recipient recovers
//! `s·Aᵢ = a·Kₑ`, then `Bᵢ = Kₒ / BLAKE3('O' ‖ t)`, and owns the output when
//! `Bᵢ` is one of its addresses and every derived value checks out.

use secp256k1::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};

use super::keys::{StealthAddress, ViewKeys};
use super::primitives::{self, hashed, tag};
use crate::api::litecoin_p2p::wire::{Output, OutputMessage, RangeProof, StandardFields};
use crate::send::error::SendError;

/// The masks a shared secret gives an output's blinding, value and nonce.
struct Masks {
    blind: [u8; 32],
    value: u64,
    nonce: [u8; 16],
}

fn masks(shared: &[u8; 32]) -> Masks {
    Masks {
        blind: hashed(tag::BLIND, shared),
        value: u64::from_le_bytes(
            hashed(tag::VALUE_MASK, shared)[..8]
                .try_into()
                .expect("8 bytes"),
        ),
        nonce: hashed(tag::NONCE_MASK, shared)[..16]
            .try_into()
            .expect("16 bytes"),
    }
}

fn send_key(address: &StealthAddress, value: u64, nonce: &[u8; 16]) -> [u8; 32] {
    let mut preimage = address.scan.serialize().to_vec();
    preimage.extend(address.spend.serialize());
    preimage.extend(value.to_le_bytes());
    preimage.extend(nonce);
    hashed(tag::SEND_KEY, &preimage)
}

fn xor16(a: &[u8; 16], b: &[u8; 16]) -> [u8; 16] {
    std::array::from_fn(|i| a[i] ^ b[i])
}

/// An output this wallet made, with what spending or reviewing it needs.
pub(crate) struct CreatedOutput {
    pub output: Output,
    /// The switched blinding its commitment hides the value under.
    pub blind: SecretKey,
}

/// Pay `value` to `address`, signed by the sender key `sender`.
pub(crate) fn create_output(
    address: &StealthAddress,
    value: u64,
    sender: &SecretKey,
) -> Result<CreatedOutput, SendError> {
    let nonce: [u8; 16] = hashed(tag::NONCE, &sender.secret_bytes())[..16]
        .try_into()
        .expect("16 bytes");
    let s = send_key(address, value, &nonce);
    let shared_point = primitives::mul(&address.scan, &s)?.serialize();
    let shared = hashed(tag::DERIVE, &shared_point);
    let output_key = primitives::mul(&address.spend, &hashed(tag::OUT_KEY, &shared))?;
    let key_exchange = primitives::mul(&address.spend, &s)?;
    let masks = masks(&shared);
    let blind = primitives::blind_switch(&primitives::secret(masks.blind)?, value)?;
    let message = OutputMessage {
        standard: Some(StandardFields {
            key_exchange,
            view_tag: hashed(tag::TAG, &shared_point)[0],
            masked_value: value ^ masks.value,
            masked_nonce: xor16(&nonce, &masks.nonce),
        }),
        extra: None,
    };
    let range_proof = primitives::prove_range(value, &blind, &message.encode())?;
    let mut output = Output {
        commitment: primitives::commit(&blind, value)?,
        sender: primitives::public(sender),
        receiver: output_key,
        message,
        range_proof: RangeProof::Full(Box::new(range_proof)),
        signature: [0; 64],
    };
    output.signature = primitives::sign(sender, &output.signature_message())?;
    Ok(CreatedOutput { output, blind })
}

/// An output the wallet owns: what spending it needs, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnedOutput {
    #[serde(with = "hex::serde")]
    pub output_id: [u8; 32],
    pub address_index: u32,
    /// In litoshis.
    pub value: u64,
    #[serde(with = "hex::serde")]
    pub commitment: [u8; 33],
    /// `Kₒ`, which an input spending the output names.
    #[serde(with = "hex::serde")]
    pub output_key: [u8; 33],
    /// The shared secret `t`: the blinding and the output key's secret
    /// derive from it.
    #[serde(with = "hex::serde")]
    pub shared_secret: [u8; 32],
}

impl OwnedOutput {
    /// The blinding the output's commitment was switched from.
    pub(crate) fn mask_blind(&self) -> Result<SecretKey, SendError> {
        primitives::secret(hashed(tag::BLIND, &self.shared_secret))
    }

    /// The secret of `Kₒ`: the address's spend secret times
    /// `BLAKE3('O' ‖ t)`.
    pub(crate) fn output_secret(&self, address_secret: &SecretKey) -> Result<SecretKey, SendError> {
        primitives::mul_secrets(address_secret, &hashed(tag::OUT_KEY, &self.shared_secret))
    }
}

/// Whether `output` pays one of the wallet's addresses, and what it holds.
/// `addresses` maps each recognized spend key to its index.
pub(crate) fn rewind(
    output: &Output,
    view: &ViewKeys,
    addresses: &std::collections::HashMap<PublicKey, u32>,
) -> Option<OwnedOutput> {
    let fields = output.message.standard.as_ref()?;
    let shared_point = primitives::mul(&fields.key_exchange, &view.scan.secret_bytes())
        .ok()?
        .serialize();
    // One byte rules out almost every output that is not the wallet's.
    if hashed(tag::TAG, &shared_point)[0] != fields.view_tag {
        return None;
    }
    let shared = hashed(tag::DERIVE, &shared_point);
    let spend = primitives::div(&output.receiver, &hashed(tag::OUT_KEY, &shared)).ok()?;
    let address_index = *addresses.get(&spend)?;
    let address = StealthAddress {
        scan: primitives::mul(&spend, &view.scan.secret_bytes()).ok()?,
        spend,
    };
    let masks = masks(&shared);
    let value = fields.masked_value ^ masks.value;
    let blind = primitives::secret(masks.blind).ok()?;
    if primitives::switch_commit(&blind, value).ok()? != output.commitment {
        return None;
    }
    let nonce = xor16(&fields.masked_nonce, &masks.nonce);
    let s = send_key(&address, value, &nonce);
    if primitives::mul(&address.spend, &s).ok()? != fields.key_exchange {
        return None;
    }
    Some(OwnedOutput {
        output_id: output.id(),
        address_index,
        value,
        commitment: output.commitment.0,
        output_key: output.receiver.serialize(),
        shared_secret: shared,
    })
}

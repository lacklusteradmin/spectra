//! A Cardano native-script account's transactions: a transfer from the
//! script's address carrying the script and its cosigners' vkey witnesses,
//! each over the hash of the body it reviewed.
//!
//! A transaction read from elsewhere is decoded and encoded again as
//! `send::cardano` writes one: one that does not come out byte for byte the
//! same carries something no review here shows, and is refused. What its
//! inputs hold is not in it: the caller reads them from the network.

use ciborium::value::Value;

use crate::api::cardano_asset::CardanoAssetId;
use crate::derivation::cardano_script::NativeScript;
use crate::send::cardano::{
    CardanoAssetAmount, CardanoInput, CardanoOutput, CardanoScriptSpend, PreparedCardanoTransaction,
};
use crate::send::error::SendError;

/// A vkey witness: the key and its signature of the body's hash.
pub(crate) type Witness = ([u8; 32], [u8; 64]);

/// A transaction's body as decoded, its inputs by outpoint alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedTransaction {
    pub inputs: Vec<(String, u32)>,
    pub outputs: Vec<CardanoOutput>,
    pub fee: u64,
    pub ttl: u64,
    pub validity_start: Option<u64>,
    pub witnesses: Vec<Witness>,
}

fn unread() -> SendError {
    SendError::invalid("The transaction is not a script spend as Spectra writes one.")
}

fn uint(value: &Value) -> Result<u64, SendError> {
    match value {
        Value::Integer(integer) => u64::try_from(i128::from(*integer)).map_err(|_| unread()),
        _ => Err(unread()),
    }
}

fn bytes(value: &Value) -> Result<&[u8], SendError> {
    match value {
        Value::Bytes(bytes) => Ok(bytes),
        _ => Err(unread()),
    }
}

fn array(value: &Value) -> Result<&[Value], SendError> {
    match value {
        Value::Array(items) => Ok(items),
        _ => Err(unread()),
    }
}

fn set(value: &Value) -> Result<&[Value], SendError> {
    match value {
        Value::Tag(258, inner) => array(inner),
        _ => Err(unread()),
    }
}

fn address(bytes: &[u8]) -> Result<String, SendError> {
    let header = *bytes.first().ok_or_else(unread)?;
    let hrp = if header & 0x0f == 1 {
        "addr"
    } else {
        "addr_test"
    };
    bech32::encode::<bech32::Bech32>(bech32::Hrp::parse(hrp).map_err(SendError::invalid)?, bytes)
        .map_err(SendError::invalid)
}

fn output(value: &Value) -> Result<CardanoOutput, SendError> {
    let [address_bytes, amount] = array(value)? else {
        return Err(unread());
    };
    let (lovelace, assets) = match amount {
        Value::Integer(_) => (uint(amount)?, Vec::new()),
        Value::Array(parts) => {
            let [coin, Value::Map(policies)] = parts.as_slice() else {
                return Err(unread());
            };
            let mut assets = Vec::new();
            for (policy, names) in policies {
                let Value::Map(names) = names else {
                    return Err(unread());
                };
                for (name, quantity) in names {
                    assets.push(CardanoAssetAmount {
                        asset: CardanoAssetId::new(
                            &hex::encode(bytes(policy)?),
                            &hex::encode(bytes(name)?),
                        )
                        .map_err(|_| unread())?
                        .identifier(),
                        quantity: uint(quantity)?,
                    });
                }
            }
            (uint(coin)?, assets)
        }
        _ => return Err(unread()),
    };
    Ok(CardanoOutput {
        address: address(bytes(address_bytes)?)?,
        lovelace,
        assets,
    })
}

/// `raw` as a transaction of `script`'s, its body and its vkey witnesses;
/// refused unless the witness set carries exactly that script.
pub(crate) fn decode(raw: &[u8], script: &NativeScript) -> Result<DecodedTransaction, SendError> {
    let value: Value = ciborium::de::from_reader(raw).map_err(|_| unread())?;
    let [
        Value::Map(body),
        Value::Map(witness_set),
        Value::Bool(true),
        Value::Null,
    ] = array(&value)?
    else {
        return Err(unread());
    };
    let (mut inputs, mut outputs, mut fee, mut ttl, mut start) = (None, None, None, None, None);
    for (key, value) in body {
        match uint(key)? {
            0 => {
                inputs = Some(
                    set(value)?
                        .iter()
                        .map(|input| {
                            let [hash, index] = array(input)? else {
                                return Err(unread());
                            };
                            Ok((
                                hex::encode(bytes(hash)?),
                                u32::try_from(uint(index)?).map_err(|_| unread())?,
                            ))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            1 => {
                outputs = Some(
                    array(value)?
                        .iter()
                        .map(output)
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            2 => fee = Some(uint(value)?),
            3 => ttl = Some(uint(value)?),
            8 => start = Some(uint(value)?),
            _ => return Err(unread()),
        }
    }
    let script_value: Value =
        ciborium::de::from_reader(script.cbor().as_slice()).map_err(|_| unread())?;
    let mut witnesses = Vec::new();
    let mut carries_script = false;
    for (key, value) in witness_set {
        match uint(key)? {
            0 => {
                for witness in set(value)? {
                    let [vkey, signature] = array(witness)? else {
                        return Err(unread());
                    };
                    witnesses.push((
                        bytes(vkey)?.try_into().map_err(|_| unread())?,
                        bytes(signature)?.try_into().map_err(|_| unread())?,
                    ));
                }
            }
            1 => carries_script = set(value)? == [script_value.clone()],
            _ => return Err(unread()),
        }
    }
    if !carries_script {
        return Err(SendError::invalid(
            "The transaction does not carry this account's script.",
        ));
    }
    Ok(DecodedTransaction {
        inputs: inputs.ok_or_else(unread)?,
        outputs: outputs.ok_or_else(unread)?,
        fee: fee.ok_or_else(unread)?,
        ttl: ttl.ok_or_else(unread)?,
        validity_start: start,
        witnesses,
    })
}

/// The script spend a transaction of `script` takes: the script, every key
/// it names as a possible signer, and `validity_start`.
pub(crate) fn spend(script: &NativeScript, validity_start: Option<u64>) -> CardanoScriptSpend {
    CardanoScriptSpend {
        script: hex::encode(script.cbor()),
        signers: script.key_hashes().len(),
        validity_start,
    }
}

impl DecodedTransaction {
    /// The prepared transaction this decodes to, given what its inputs hold
    /// (`held`, read from the network); refused unless it encodes back to
    /// `raw` byte for byte.
    pub(crate) fn prepared(
        &self,
        script: &NativeScript,
        held: &[CardanoInput],
        raw: &[u8],
    ) -> Result<PreparedCardanoTransaction, SendError> {
        let inputs = self
            .inputs
            .iter()
            .map(|(hash, index)| {
                held.iter()
                    .find(|input| input.tx_hash == *hash && input.tx_index == *index)
                    .cloned()
                    .ok_or_else(|| {
                        SendError::invalid(
                            "The transaction spends an output this account does not hold, or one already spent.",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let prepared = PreparedCardanoTransaction {
            inputs,
            outputs: self.outputs.clone(),
            fee: self.fee,
            ttl: self.ttl,
            script: Some(spend(script, self.validity_start)),
        };
        if prepared.encode_signed(&self.witnesses)? != raw {
            return Err(unread());
        }
        prepared.conserves()?;
        Ok(prepared)
    }
}

/// The key hashes `witnesses` prove for `body`: each a valid signature of
/// the body's hash by a key the script names, each key once.
pub(crate) fn signed_keys(
    prepared: &PreparedCardanoTransaction,
    script: &NativeScript,
    witnesses: &[Witness],
) -> Result<Vec<[u8; 28]>, SendError> {
    let hash = prepared.transaction_hash()?;
    let named = script.key_hashes();
    let mut signed: Vec<[u8; 28]> = Vec::new();
    for (vkey, signature) in witnesses {
        let key_hash = crate::derivation::cardano::verification_key_hash(vkey);
        let valid = ed25519_dalek::VerifyingKey::from_bytes(vkey).is_ok_and(|key| {
            key.verify_strict(&hash, &ed25519_dalek::Signature::from_bytes(signature))
                .is_ok()
        });
        if !valid || !named.contains(&key_hash) {
            return Err(SendError::invalid(
                "A witness is not a valid signature by one of the script's keys.",
            ));
        }
        if signed.contains(&key_hash) {
            return Err(SendError::invalid("A key signed twice."));
        }
        signed.push(key_hash);
    }
    Ok(signed)
}

#[cfg(test)]
#[path = "tests/cardano_multisig.rs"]
mod tests;

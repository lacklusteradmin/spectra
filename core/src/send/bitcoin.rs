//! Bitcoin-format transparent transactions spending a wallet account's
//! inputs, each signed with its own address's key: P2PKH, nested and native
//! P2WPKH, and Taproot key-path spends. Bitcoin and Litecoin sign here; the
//! caller decides which of these scripts its network spends.

use std::collections::HashSet;
use std::str::FromStr;

use bitcoin::hashes::Hash as _;
use bitcoin::key::TapTweak;
use bitcoin::script::{Builder, PushBytesBuf};
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::sighash::{EcdsaSighashType, Prevouts, SighashCache, TapSighashType};
use bitcoin::transaction::Version;
use bitcoin::{
    Amount, CompressedPublicKey, OutPoint, Script, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};

use crate::send::error::SendError;

/// The script an input spends, which decides its signature hash and where
/// the signature goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputKind {
    P2pkh,
    P2wpkh,
    /// P2WPKH nested in P2SH, the only P2SH script a key alone spends.
    NestedP2wpkh,
    /// A Taproot output spent by its key path: the key tweaked with no
    /// script tree, as BIP-86 derives it.
    P2tr,
}

impl InputKind {
    pub(crate) fn from_script(script: &Script) -> Result<Self, SendError> {
        if script.is_p2pkh() {
            Ok(Self::P2pkh)
        } else if script.is_p2wpkh() {
            Ok(Self::P2wpkh)
        } else if script.is_p2sh() {
            Ok(Self::NestedP2wpkh)
        } else if script.is_p2tr() {
            Ok(Self::P2tr)
        } else {
            Err(SendError::Invalid(
                "Input must be P2PKH, P2WPKH, P2SH-P2WPKH or P2TR".into(),
            ))
        }
    }

    /// The script `key` owns as this kind.
    pub(crate) fn script(self, key: &CompressedPublicKey) -> ScriptBuf {
        match self {
            Self::P2pkh => ScriptBuf::new_p2pkh(&key.pubkey_hash()),
            Self::P2wpkh => ScriptBuf::new_p2wpkh(&key.wpubkey_hash()),
            Self::NestedP2wpkh => ScriptBuf::new_p2wpkh(&key.wpubkey_hash()).to_p2sh(),
            Self::P2tr => ScriptBuf::new_p2tr(&Secp256k1::verification_only(), key.0.into(), None),
        }
    }

    pub(crate) fn validate_key(
        self,
        script: &Script,
        key: &CompressedPublicKey,
    ) -> Result<(), SendError> {
        if script != self.script(key).as_script() {
            return Err(SendError::Invalid(
                "Input script does not match the supplied private key".into(),
            ));
        }
        Ok(())
    }
}

/// One input and the key of the address it pays.
pub(crate) struct SigningInput<'a> {
    pub utxo: &'a (String, u32, u64, Vec<u8>),
    pub private_key: &'a [u8],
}

/// The compressed public key of a 32-byte secret.
pub(crate) fn public_key(
    private_key: &[u8],
) -> Result<(SecretKey, CompressedPublicKey), SendError> {
    let secret = SecretKey::from_slice(private_key).map_err(SendError::invalid)?;
    let public = CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
        &Secp256k1::new(),
        &secret,
    ));
    Ok((secret, public))
}

/// Sign `inputs`, each with its own key, into a transaction paying `outputs`
/// in order, every input with `sequence`. Each input's script must be the one
/// its key owns. Signatures commit to all inputs and outputs: SIGHASH_ALL,
/// and Taproot's default.
pub(crate) fn sign_inputs(
    inputs: &[SigningInput<'_>],
    outputs: &[(Vec<u8>, u64)],
    version: Version,
    sequence: Sequence,
) -> Result<Vec<u8>, SendError> {
    if inputs.is_empty() || outputs.is_empty() {
        return Err(SendError::Invalid(
            "transaction must have inputs and outputs".into(),
        ));
    }
    let secp = Secp256k1::new();
    let mut seen = HashSet::new();
    let mut identities = Vec::with_capacity(inputs.len());
    let mut tx_inputs = Vec::with_capacity(inputs.len());
    let mut prevouts = Vec::with_capacity(inputs.len());
    for input in inputs {
        let (txid, vout, value, script) = input.utxo;
        let script = Script::from_bytes(script);
        let kind = InputKind::from_script(script)?;
        let (secret, key) = public_key(input.private_key)?;
        kind.validate_key(script, &key)?;
        let outpoint = OutPoint {
            txid: Txid::from_str(txid).map_err(SendError::invalid)?,
            vout: *vout,
        };
        if outpoint.is_null() || !seen.insert(outpoint) {
            return Err(SendError::Invalid("invalid or duplicate input".into()));
        }
        identities.push((kind, secret, key));
        prevouts.push(TxOut {
            value: Amount::from_sat(*value),
            script_pubkey: script.to_owned(),
        });
        tx_inputs.push(TxIn {
            previous_output: outpoint,
            script_sig: ScriptBuf::new(),
            sequence,
            witness: Witness::new(),
        });
    }
    let mut tx = Transaction {
        version,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: tx_inputs,
        output: outputs
            .iter()
            .map(|(script, value)| TxOut {
                value: Amount::from_sat(*value),
                script_pubkey: ScriptBuf::from_bytes(script.clone()),
            })
            .collect(),
    };

    let mut cache = SighashCache::new(&mut tx);
    let mut signatures = Vec::with_capacity(inputs.len());
    for (index, (kind, secret, key)) in identities.iter().enumerate() {
        let signature = match kind {
            InputKind::P2tr => {
                let digest = cache
                    .taproot_key_spend_signature_hash(
                        index,
                        &Prevouts::All(&prevouts),
                        TapSighashType::Default,
                    )
                    .map_err(SendError::invalid)?;
                let keypair = bitcoin::key::Keypair::from_secret_key(&secp, secret)
                    .tap_tweak(&secp, None)
                    .to_keypair();
                bitcoin::taproot::Signature {
                    signature: secp.sign_schnorr(
                        &Message::from_digest(digest.to_raw_hash().to_byte_array()),
                        &keypair,
                    ),
                    sighash_type: TapSighashType::Default,
                }
                .to_vec()
            }
            InputKind::P2pkh => {
                let digest = cache
                    .legacy_signature_hash(
                        index,
                        &prevouts[index].script_pubkey,
                        EcdsaSighashType::All as u32,
                    )
                    .map_err(SendError::invalid)?;
                ecdsa(&secp, secret, digest.to_byte_array())
            }
            InputKind::P2wpkh | InputKind::NestedP2wpkh => {
                let digest = cache
                    .p2wpkh_signature_hash(
                        index,
                        &InputKind::P2wpkh.script(key),
                        prevouts[index].value,
                        EcdsaSighashType::All,
                    )
                    .map_err(SendError::invalid)?;
                ecdsa(&secp, secret, digest.to_byte_array())
            }
        };
        signatures.push(signature);
    }
    let tx = cache.into_transaction();
    for (index, signature) in signatures.into_iter().enumerate() {
        let (kind, _, key) = &identities[index];
        match kind {
            InputKind::P2pkh => {
                let signature = PushBytesBuf::try_from(signature).map_err(SendError::invalid)?;
                tx.input[index].script_sig = Builder::new()
                    .push_slice(signature)
                    .push_slice(key.to_bytes())
                    .into_script();
            }
            InputKind::P2wpkh | InputKind::NestedP2wpkh => {
                if *kind == InputKind::NestedP2wpkh {
                    let redeem = PushBytesBuf::try_from(InputKind::P2wpkh.script(key).into_bytes())
                        .map_err(SendError::invalid)?;
                    tx.input[index].script_sig = Builder::new().push_slice(redeem).into_script();
                }
                tx.input[index].witness =
                    Witness::from_slice(&[signature, key.to_bytes().to_vec()]);
            }
            InputKind::P2tr => tx.input[index].witness = Witness::from_slice(&[signature]),
        }
    }
    Ok(bitcoin::consensus::serialize(&*tx))
}

/// A low-S DER ECDSA signature with SIGHASH_ALL appended.
fn ecdsa(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    secret: &SecretKey,
    digest: [u8; 32],
) -> Vec<u8> {
    let mut signature = secp
        .sign_ecdsa(&Message::from_digest(digest), secret)
        .serialize_der()
        .to_vec();
    signature.push(EcdsaSighashType::All as u8);
    signature
}

/// Upper bound for a signed transaction's virtual size, with mixed legacy,
/// witness and Taproot inputs and CompactSize count boundaries, under
/// BIP141 weight. A low-S DER signature plus its SIGHASH byte is at most 72
/// bytes; a default-hash Schnorr signature is 64.
pub(crate) fn estimate_vsize<'a>(
    input_scripts: impl IntoIterator<Item = &'a [u8]>,
    output_script_lengths: impl IntoIterator<Item = usize>,
) -> Result<u64, SendError> {
    let compact_size = |value: u64| -> u64 {
        match value {
            0..=0xfc => 1,
            0xfd..=0xffff => 3,
            0x1_0000..=0xffff_ffff => 5,
            _ => 9,
        }
    };
    let overflow = || SendError::Invalid("transaction size overflow".into());
    let mut input_count = 0u64;
    let mut inputs_size = 0u64;
    let mut witness_size = 0u64;
    let mut has_witness = false;
    for script in input_scripts {
        let kind = InputKind::from_script(Script::from_bytes(script))?;
        let (base, witness) = match kind {
            InputKind::P2pkh => (148, 1), // empty stack if another input has a witness
            InputKind::P2wpkh => (41, 108),
            InputKind::NestedP2wpkh => (64, 108),
            InputKind::P2tr => (41, 66),
        };
        has_witness |= kind != InputKind::P2pkh;
        input_count = input_count.checked_add(1).ok_or_else(overflow)?;
        inputs_size = inputs_size.checked_add(base).ok_or_else(overflow)?;
        witness_size = witness_size.checked_add(witness).ok_or_else(overflow)?;
    }
    if input_count == 0 {
        return Err(SendError::Invalid("transaction must have inputs".into()));
    }
    let mut output_count = 0u64;
    let mut outputs_size = 0u64;
    for len in output_script_lengths {
        let len = u64::try_from(len).map_err(SendError::invalid)?;
        output_count = output_count.checked_add(1).ok_or_else(overflow)?;
        outputs_size = 8u64
            .checked_add(compact_size(len))
            .and_then(|size| size.checked_add(len))
            .and_then(|size| size.checked_add(outputs_size))
            .ok_or_else(overflow)?;
    }
    let base_size = 8u64
        .checked_add(compact_size(input_count))
        .and_then(|size| size.checked_add(compact_size(output_count)))
        .and_then(|size| size.checked_add(inputs_size))
        .and_then(|size| size.checked_add(outputs_size))
        .ok_or_else(overflow)?;
    let witness_size = if has_witness {
        witness_size.checked_add(2).ok_or_else(overflow)? // marker and witness flag
    } else {
        0
    };
    let weight = base_size
        .checked_mul(4)
        .and_then(|size| size.checked_add(witness_size))
        .ok_or_else(overflow)?;
    Ok(weight.div_ceil(4))
}

#[cfg(test)]
#[path = "tests/bitcoin.rs"]
pub(crate) mod tests;

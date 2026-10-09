//! A P2SH multisig account's partially signed transactions, on the networks
//! without SegWit: Bitcoin Cash, carried as BCHN's PSBTs (BIP-174's layout,
//! each input's spent output in place of the transaction it came from,
//! signed with SIGHASH_ALL|FORKID over BIP-143's digest), and Dogecoin,
//! carried as Dogecoin Core's `signrawtransaction` passes one between
//! signers (the transaction itself, each input's scriptSig holding the
//! signatures so far and the redeem script, signed over the legacy digest).
//!
//! Neither carries what a Dogecoin input spends, so a spend is kept here as
//! the unsigned transaction with each input's amount and place in the
//! account and its signatures, and written out in the network's form. Every
//! signature is verified against the key the account has at the input's
//! place before it counts; Bitcoin Cash's Schnorr multisig signatures are
//! refused, since a scriptSig cannot mix them with ECDSA ones.

use std::collections::BTreeMap;

use bitcoin::bip32::Xpriv;
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, ecdsa::Signature};
use bitcoin::{OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use serde::{Deserialize, Serialize};

use crate::derivation::multisig::{MultisigPolicy, Place, UtxoMultisigScript};
use crate::registry::Chain;
use crate::send::bitcoin_wire::{dsha256, varint};
use crate::send::error::SendError;
use crate::send::psbt::{Review, ReviewedInput, ReviewedOutput};

const SIGHASH_ALL: u8 = 0x01;
const SIGHASH_ALL_FORKID: u8 = 0x41;

/// A spend as the account keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct P2shSpend {
    /// The unsigned transaction, hex.
    #[serde(with = "transaction_hex")]
    pub transaction: Transaction,
    pub inputs: Vec<SpentInput>,
    /// Each output's place in the account when it is change.
    pub change: Vec<Option<Place>>,
}

/// What an input spends, and its signatures by cosigner (policy order),
/// each DER with its hash type, hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpentInput {
    pub value: u64,
    pub place: Place,
    /// Kept as `[cosigner, signature]` pairs: a session is stored inside an
    /// internally tagged enum, which cannot read numeric map keys back.
    #[serde(with = "signature_pairs")]
    pub signatures: BTreeMap<usize, String>,
}

mod signature_pairs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<usize, String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<usize, String>, D::Error> {
        Ok(Vec::<(usize, String)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

mod transaction_hex {
    use bitcoin::Transaction;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(tx: &Transaction, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(bitcoin::consensus::serialize(tx)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Transaction, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = hex::decode(text).map_err(serde::de::Error::custom)?;
        bitcoin::consensus::deserialize(&bytes).map_err(serde::de::Error::custom)
    }
}

/// The network's signature form: Bitcoin Cash's fork id, or `None` for the
/// legacy digest.
fn fork_id(chain: Chain) -> Option<u32> {
    chain.sighash_fork_id().ok()
}

fn hash_type(chain: Chain) -> u8 {
    if fork_id(chain).is_some() {
        SIGHASH_ALL_FORKID
    } else {
        SIGHASH_ALL
    }
}

/// The transaction version each network's own wallet writes.
fn version(chain: Chain) -> bitcoin::transaction::Version {
    if fork_id(chain).is_some() {
        bitcoin::transaction::Version::TWO
    } else {
        bitcoin::transaction::Version::ONE
    }
}

fn require_p2sh(policy: &MultisigPolicy) -> Result<(), SendError> {
    if policy.script != UtxoMultisigScript::Sh {
        return Err(SendError::invalid(
            "Only a P2SH multisig account spends this way.",
        ));
    }
    Ok(())
}

fn serialize_output(output: &TxOut) -> Vec<u8> {
    let mut out = output.value.to_sat().to_le_bytes().to_vec();
    out.extend(varint(output.script_pubkey.len()));
    out.extend(output.script_pubkey.as_bytes());
    out
}

/// The digest input `index` signs, its scriptCode the redeem script.
fn sighash(
    chain: Chain,
    tx: &Transaction,
    index: usize,
    redeem: &ScriptBuf,
    value: u64,
) -> Result<[u8; 32], SendError> {
    match fork_id(chain) {
        Some(fork) => {
            let mut prevouts = Vec::new();
            let mut sequences = Vec::new();
            for input in &tx.input {
                prevouts.extend(bitcoin::consensus::serialize(&input.previous_output));
                sequences.extend(input.sequence.0.to_le_bytes());
            }
            let outputs: Vec<u8> = tx.output.iter().flat_map(serialize_output).collect();
            let input = &tx.input[index];
            let mut preimage = tx.version.0.to_le_bytes().to_vec();
            preimage.extend(dsha256(&prevouts));
            preimage.extend(dsha256(&sequences));
            preimage.extend(bitcoin::consensus::serialize(&input.previous_output));
            preimage.extend(varint(redeem.len()));
            preimage.extend(redeem.as_bytes());
            preimage.extend(value.to_le_bytes());
            preimage.extend(input.sequence.0.to_le_bytes());
            preimage.extend(dsha256(&outputs));
            preimage.extend(tx.lock_time.to_consensus_u32().to_le_bytes());
            preimage.extend((fork << 8 | u32::from(SIGHASH_ALL_FORKID)).to_le_bytes());
            Ok(dsha256(&preimage))
        }
        None => {
            let mut copy = tx.clone();
            for (other, input) in copy.input.iter_mut().enumerate() {
                input.script_sig = if other == index {
                    redeem.clone()
                } else {
                    ScriptBuf::new()
                };
            }
            let mut preimage = bitcoin::consensus::serialize(&copy);
            preimage.extend(u32::from(SIGHASH_ALL).to_le_bytes());
            Ok(dsha256(&preimage))
        }
    }
}

/// The cosigner whose key at the input's place made `signature`, refused
/// unless it is a strict, low-S ECDSA signature with the network's hash
/// type over the input's digest.
fn signer_of(
    policy: &MultisigPolicy,
    chain: Chain,
    tx: &Transaction,
    index: usize,
    input: &SpentInput,
    signature: &[u8],
) -> Result<usize, SendError> {
    let invalid = || {
        SendError::refused(
            "Input %@ carries a signature that is not a valid one from the wallet's keys.",
            [index.to_string()],
        )
    };
    let (&kind, der) = signature.split_last().ok_or_else(invalid)?;
    if signature.len() == 65 {
        return Err(SendError::invalid(
            "Schnorr multisig signatures are not read; sign with ECDSA.",
        ));
    }
    let parsed = Signature::from_der(der).map_err(|_| invalid())?;
    let mut normalized = parsed;
    normalized.normalize_s();
    if kind != hash_type(chain) || normalized != parsed {
        return Err(invalid());
    }
    let redeem = policy.witness_script(input.place)?;
    let message = Message::from_digest(sighash(chain, tx, index, &redeem, input.value)?);
    let secp = Secp256k1::verification_only();
    policy
        .keys(input.place)?
        .iter()
        .position(|(key, _, _)| secp.verify_ecdsa(&message, &parsed, key).is_ok())
        .ok_or_else(invalid)
}

/// The unsigned transaction spending `inputs` (outpoint, amount, place) to
/// `payments`, then change to `change`, as the network's wallet writes one.
pub(crate) fn build(
    policy: &MultisigPolicy,
    chain: Chain,
    inputs: &[(OutPoint, u64, Place)],
    payments: &[(ScriptBuf, u64)],
    change: Option<(Place, u64)>,
) -> Result<P2shSpend, SendError> {
    require_p2sh(policy)?;
    let mut output: Vec<TxOut> = payments
        .iter()
        .map(|(script, value)| TxOut {
            value: bitcoin::Amount::from_sat(*value),
            script_pubkey: script.clone(),
        })
        .collect();
    let mut places = vec![None; output.len()];
    if let Some((place, value)) = change {
        output.push(TxOut {
            value: bitcoin::Amount::from_sat(value),
            script_pubkey: policy.script_pubkey(place)?,
        });
        places.push(Some(place));
    }
    Ok(P2shSpend {
        transaction: Transaction {
            version: version(chain),
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: inputs
                .iter()
                .map(|(outpoint, _, _)| TxIn {
                    previous_output: *outpoint,
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence::MAX,
                    witness: Witness::new(),
                })
                .collect(),
            output,
        },
        inputs: inputs
            .iter()
            .map(|(_, value, place)| SpentInput {
                value: *value,
                place: *place,
                signatures: BTreeMap::new(),
            })
            .collect(),
        change: places,
    })
}

/// `spend` as the account's: every input its own, change its own script,
/// every signature valid; the transaction id is the finished transaction's
/// once complete, since its scriptSigs are part of it.
pub(crate) fn review(
    policy: &MultisigPolicy,
    chain: Chain,
    spend: &P2shSpend,
) -> Result<Review, SendError> {
    require_p2sh(policy)?;
    let tx = &spend.transaction;
    if tx.input.is_empty()
        || tx.output.is_empty()
        || tx.input.len() != spend.inputs.len()
        || tx.output.len() != spend.change.len()
        || tx
            .input
            .iter()
            .any(|input| !input.script_sig.is_empty() || !input.witness.is_empty())
    {
        return Err(SendError::invalid(
            "The transaction is not a spend as Spectra keeps one.",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut inputs = Vec::with_capacity(tx.input.len());
    for (index, (txin, input)) in tx.input.iter().zip(&spend.inputs).enumerate() {
        if !seen.insert(txin.previous_output) {
            return Err(SendError::invalid(
                "The transaction spends an output twice.",
            ));
        }
        let mut signed_by = Vec::new();
        for (cosigner, signature) in &input.signatures {
            let bytes = hex::decode(signature).map_err(SendError::invalid)?;
            if signer_of(policy, chain, tx, index, input, &bytes)? != *cosigner {
                return Err(SendError::invalid(
                    "A signature is filed under another cosigner.",
                ));
            }
            signed_by.push(*cosigner);
        }
        let script = policy.script_pubkey(input.place)?;
        inputs.push(ReviewedInput {
            outpoint: txin.previous_output,
            value: input.value,
            place: input.place,
            address: crate::derivation::utxo_address::script_address(chain, script.as_bytes())
                .ok_or_else(|| SendError::invalid("The network has no address for this script."))?,
            signed_by,
        });
    }
    let mut outputs = Vec::with_capacity(tx.output.len());
    for (txout, change) in tx.output.iter().zip(&spend.change) {
        if let Some(place) = change
            && txout.script_pubkey != policy.script_pubkey(*place)?
        {
            return Err(SendError::invalid(
                "An output claims to be the wallet's change but pays another script.",
            ));
        }
        outputs.push(ReviewedOutput {
            address: crate::derivation::utxo_address::script_address(
                chain,
                txout.script_pubkey.as_bytes(),
            )
            .unwrap_or_else(|| format!("script {}", txout.script_pubkey.to_hex_string())),
            value: txout.value.to_sat(),
            change: *change,
        });
    }
    let spent = inputs
        .iter()
        .try_fold(0u64, |sum, input| sum.checked_add(input.value));
    let paid = outputs
        .iter()
        .try_fold(0u64, |sum, output| sum.checked_add(output.value));
    let fee = spent
        .zip(paid)
        .and_then(|(spent, paid)| spent.checked_sub(paid))
        .ok_or_else(|| SendError::invalid("The transaction pays out more than it spends."))?;
    let digest = {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        hash.update(chain.str_id().as_bytes());
        hash.update(bitcoin::consensus::serialize(tx));
        for input in &inputs {
            hash.update(input.value.to_le_bytes());
        }
        hex::encode(hash.finalize())
    };
    let complete = inputs
        .iter()
        .all(|input| input.signed_by.len() >= policy.threshold);
    let txid = if complete {
        finalize(policy, spend)?.compute_txid()
    } else {
        tx.compute_txid()
    };
    Ok(Review {
        txid: txid.to_string(),
        digest,
        inputs,
        outputs,
        fee,
        complete,
    })
}

/// Sign every input as cosigner `cosigner`, whose account key is
/// `account`, after `spend` reviews as the account's.
pub(crate) fn sign(
    policy: &MultisigPolicy,
    chain: Chain,
    spend: &mut P2shSpend,
    cosigner: usize,
    account: &Xpriv,
) -> Result<(), SendError> {
    review(policy, chain, spend)?;
    let secp = Secp256k1::new();
    for index in 0..spend.inputs.len() {
        let input = &spend.inputs[index];
        if input.signatures.contains_key(&cosigner) {
            return Err(SendError::invalid(
                "This cosigner already signed the transaction.",
            ));
        }
        let (branch, child) = input.place;
        let key = account
            .derive_priv(
                &secp,
                &[
                    bitcoin::bip32::ChildNumber::from_normal_idx(branch)
                        .map_err(SendError::invalid)?,
                    bitcoin::bip32::ChildNumber::from_normal_idx(child)
                        .map_err(SendError::invalid)?,
                ],
            )
            .map_err(SendError::invalid)?
            .private_key;
        let public = PublicKey::from_secret_key(&secp, &key);
        if policy.keys(input.place)?[cosigner].0 != public {
            return Err(SendError::invalid(
                "The cosigner's key is not the account's key here.",
            ));
        }
        let redeem = policy.witness_script(input.place)?;
        let digest = sighash(chain, &spend.transaction, index, &redeem, input.value)?;
        let mut signature = secp
            .sign_ecdsa(&Message::from_digest(digest), &key)
            .serialize_der()
            .to_vec();
        signature.push(hash_type(chain));
        spend.inputs[index]
            .signatures
            .insert(cosigner, hex::encode(signature));
    }
    review(policy, chain, spend)?;
    Ok(())
}

/// Join `incoming`'s signatures to `held`'s: the same transaction, spending
/// the same amounts from the same places.
pub(crate) fn combine(
    policy: &MultisigPolicy,
    chain: Chain,
    held: &mut P2shSpend,
    incoming: P2shSpend,
) -> Result<(), SendError> {
    review(policy, chain, &incoming)?;
    if held.transaction != incoming.transaction
        || held.change != incoming.change
        || held
            .inputs
            .iter()
            .zip(&incoming.inputs)
            .any(|(own, other)| own.value != other.value || own.place != other.place)
    {
        return Err(SendError::invalid(
            "The transactions differ; they cannot be joined.",
        ));
    }
    for (own, other) in held.inputs.iter_mut().zip(incoming.inputs) {
        for (cosigner, signature) in other.signatures {
            own.signatures.entry(cosigner).or_insert(signature);
        }
    }
    review(policy, chain, held)?;
    Ok(())
}

fn push(script: &mut Vec<u8>, data: &[u8]) {
    match data.len() {
        0..=0x4b => script.push(data.len() as u8),
        0x4c..=0xff => script.extend([0x4c, data.len() as u8]),
        _ => {
            script.push(0x4d);
            script.extend((data.len() as u16).to_le_bytes());
        }
    }
    script.extend(data);
}

/// The cosigners whose signatures `input` carries, in the redeem script's
/// key order, at most `limit`.
fn signers_in_key_order(
    policy: &MultisigPolicy,
    input: &SpentInput,
    limit: usize,
) -> Result<Vec<usize>, SendError> {
    let mut order: Vec<(usize, [u8; 33])> = policy
        .keys(input.place)?
        .iter()
        .enumerate()
        .map(|(cosigner, (key, _, _))| (cosigner, key.serialize()))
        .collect();
    order.sort_by_key(|(_, key)| *key);
    Ok(order
        .into_iter()
        .map(|(cosigner, _)| cosigner)
        .filter(|cosigner| input.signatures.contains_key(cosigner))
        .take(limit)
        .collect())
}

/// `OP_0`, at most `limit` signatures in the redeem script's key order, then
/// the redeem script.
fn script_sig(
    policy: &MultisigPolicy,
    input: &SpentInput,
    limit: usize,
) -> Result<ScriptBuf, SendError> {
    let mut script = vec![0x00];
    for cosigner in signers_in_key_order(policy, input, limit)? {
        push(
            &mut script,
            &hex::decode(&input.signatures[&cosigner]).map_err(SendError::invalid)?,
        );
    }
    push(&mut script, policy.witness_script(input.place)?.as_bytes());
    Ok(ScriptBuf::from(script))
}

/// The finished transaction: each input's scriptSig the threshold's first
/// signatures in key order and its redeem script.
pub(crate) fn finalize(
    policy: &MultisigPolicy,
    spend: &P2shSpend,
) -> Result<Transaction, SendError> {
    let mut tx = spend.transaction.clone();
    for (txin, input) in tx.input.iter_mut().zip(&spend.inputs) {
        if input.signatures.len() < policy.threshold {
            return Err(SendError::invalid(
                "Not every input carries the threshold's signatures yet.",
            ));
        }
        txin.script_sig = script_sig(policy, input, policy.threshold)?;
    }
    Ok(tx)
}

/// The size of a transaction spending `inputs` of the account to outputs
/// of `scripts` lengths, every input carrying the threshold's signatures at
/// their longest.
pub(crate) fn estimate_size(policy: &MultisigPolicy, inputs: usize, scripts: Vec<usize>) -> usize {
    let redeem = 3 + policy.cosigners.len() * 34;
    let redeem_push = redeem + if redeem > 0xff { 3 } else { 2 };
    let script_sig = 1 + policy.threshold * 74 + redeem_push;
    let input = 36 + varint(script_sig).len() + script_sig + 4;
    let outputs: usize = scripts
        .iter()
        .map(|script| 8 + varint(*script).len() + script)
        .sum();
    4 + varint(inputs).len() + inputs * input + varint(scripts.len()).len() + outputs + 4
}

/// The spend as the network passes it between signers: a BCHN PSBT in
/// base64, or Dogecoin Core's partially signed transaction in hex.
pub(crate) fn encode(
    policy: &MultisigPolicy,
    chain: Chain,
    spend: &P2shSpend,
) -> Result<String, SendError> {
    if fork_id(chain).is_some() {
        return bch_psbt::encode(policy, spend);
    }
    let mut tx = spend.transaction.clone();
    for (txin, input) in tx.input.iter_mut().zip(&spend.inputs) {
        if !input.signatures.is_empty() {
            txin.script_sig = script_sig(policy, input, usize::MAX)?;
        }
    }
    Ok(hex::encode(bitcoin::consensus::serialize(&tx)))
}

/// What the account knows of an outpoint it holds, and of a script it pays
/// itself: amounts and places a Dogecoin transaction does not carry.
pub(crate) struct Holdings<'a> {
    pub unspent: &'a dyn Fn(&OutPoint) -> Option<(u64, Place)>,
    pub change: &'a dyn Fn(&ScriptBuf) -> Option<Place>,
}

/// A spend another signer or coordinator wrote, in the network's form.
pub(crate) fn decode(
    policy: &MultisigPolicy,
    chain: Chain,
    text: &str,
    holdings: &Holdings<'_>,
) -> Result<P2shSpend, SendError> {
    require_p2sh(policy)?;
    let spend = if fork_id(chain).is_some() {
        bch_psbt::decode(policy, chain, text)?
    } else {
        decode_partial(policy, chain, text, holdings)?
    };
    review(policy, chain, &spend)?;
    Ok(spend)
}

/// A Dogecoin transaction as `signrawtransaction` leaves it: each scriptSig
/// empty, or `OP_0`, signatures and `OP_0` placeholders, and the redeem
/// script.
fn decode_partial(
    policy: &MultisigPolicy,
    chain: Chain,
    text: &str,
    holdings: &Holdings<'_>,
) -> Result<P2shSpend, SendError> {
    let bytes = hex::decode(text.trim())
        .map_err(|_| SendError::invalid("Not a transaction: it is not hex."))?;
    let mut tx: Transaction = bitcoin::consensus::deserialize(&bytes)
        .map_err(|_| SendError::invalid("Not a transaction."))?;
    let mut inputs = Vec::with_capacity(tx.input.len());
    let mut scripts = Vec::with_capacity(tx.input.len());
    for (index, txin) in tx.input.iter_mut().enumerate() {
        let (value, place) = (holdings.unspent)(&txin.previous_output).ok_or_else(|| {
            SendError::refused(
                "Input %@ is not one of this wallet's unspent outputs.",
                [index.to_string()],
            )
        })?;
        scripts.push(std::mem::take(&mut txin.script_sig));
        if !txin.witness.is_empty() {
            return Err(SendError::invalid(
                "A Dogecoin transaction carries no witness.",
            ));
        }
        inputs.push(SpentInput {
            value,
            place,
            signatures: BTreeMap::new(),
        });
    }
    for (index, script) in scripts.iter().enumerate() {
        if script.is_empty() {
            continue;
        }
        let mut pushes = Vec::new();
        for instruction in script.instructions() {
            match instruction
                .map_err(|_| SendError::invalid("An input's scriptSig does not parse."))?
            {
                bitcoin::script::Instruction::PushBytes(bytes) => {
                    pushes.push(bytes.as_bytes().to_vec())
                }
                bitcoin::script::Instruction::Op(_) => {
                    return Err(SendError::invalid(
                        "An input's scriptSig is not a multisig one.",
                    ));
                }
            }
        }
        let [dummy, signatures @ .., redeem] = pushes.as_slice() else {
            return Err(SendError::invalid(
                "An input's scriptSig is not a multisig one.",
            ));
        };
        if !dummy.is_empty()
            || redeem.as_slice() != policy.witness_script(inputs[index].place)?.as_bytes()
        {
            return Err(SendError::refused(
                "Input %@'s script is not the wallet's script at its place.",
                [index.to_string()],
            ));
        }
        for signature in signatures.iter().filter(|signature| !signature.is_empty()) {
            let cosigner = signer_of(policy, chain, &tx, index, &inputs[index], signature)?;
            inputs[index]
                .signatures
                .insert(cosigner, hex::encode(signature));
        }
    }
    let change = tx
        .output
        .iter()
        .map(|output| (holdings.change)(&output.script_pubkey))
        .collect();
    Ok(P2shSpend {
        transaction: tx,
        inputs,
        change,
    })
}

/// BCHN's PSBT: BIP-174's version-0 maps, each input's spent output
/// (`PSBT_IN_UTXO`, key 0) a serialized `CTxOut`, no witness fields.
mod bch_psbt {
    use super::*;
    use bitcoin::bip32::{ChildNumber, DerivationPath, Fingerprint};

    const MAGIC: &[u8] = b"psbt\xff";

    fn unread() -> SendError {
        SendError::invalid("Not a Bitcoin Cash PSBT as BCHN writes one.")
    }

    fn record(out: &mut Vec<u8>, key: &[u8], value: &[u8]) {
        out.extend(varint(key.len()));
        out.extend(key);
        out.extend(varint(value.len()));
        out.extend(value);
    }

    fn derivation(fingerprint: Fingerprint, path: &DerivationPath) -> Vec<u8> {
        let mut value = fingerprint.as_bytes().to_vec();
        for step in path {
            value.extend(u32::from(*step).to_le_bytes());
        }
        value
    }

    /// Each key at `place` and its origin, by key, as BCHN's map orders them.
    fn derivations(
        policy: &MultisigPolicy,
        place: Place,
    ) -> Result<Vec<([u8; 33], Vec<u8>)>, SendError> {
        let mut keys: Vec<([u8; 33], Vec<u8>)> = policy
            .keys(place)?
            .into_iter()
            .map(|(key, fingerprint, path)| (key.serialize(), derivation(fingerprint, &path)))
            .collect();
        keys.sort();
        Ok(keys)
    }

    pub(super) fn encode(policy: &MultisigPolicy, spend: &P2shSpend) -> Result<String, SendError> {
        use base64::Engine;
        let mut out = MAGIC.to_vec();
        record(
            &mut out,
            &[0x00],
            &bitcoin::consensus::serialize(&spend.transaction),
        );
        out.push(0x00);
        for input in &spend.inputs {
            let utxo = TxOut {
                value: bitcoin::Amount::from_sat(input.value),
                script_pubkey: policy.script_pubkey(input.place)?,
            };
            record(&mut out, &[0x00], &serialize_output(&utxo));
            let keys = policy.keys(input.place)?;
            // CKeyID order: by each key's HASH160.
            let mut signatures: Vec<([u8; 20], [u8; 33], Vec<u8>)> = input
                .signatures
                .iter()
                .map(|(cosigner, signature)| {
                    let key = keys[*cosigner].0.serialize();
                    Ok((
                        crate::derivation::bitcoin::hash160(&key),
                        key,
                        hex::decode(signature).map_err(SendError::invalid)?,
                    ))
                })
                .collect::<Result<_, SendError>>()?;
            signatures.sort();
            for (_, key, signature) in signatures {
                record(&mut out, &[&[0x02][..], &key].concat(), &signature);
            }
            record(
                &mut out,
                &[0x04],
                policy.witness_script(input.place)?.as_bytes(),
            );
            for (key, origin) in derivations(policy, input.place)? {
                record(&mut out, &[&[0x06][..], &key].concat(), &origin);
            }
            out.push(0x00);
        }
        for change in &spend.change {
            if let Some(place) = change {
                record(&mut out, &[0x00], policy.witness_script(*place)?.as_bytes());
                for (key, origin) in derivations(policy, *place)? {
                    record(&mut out, &[&[0x02][..], &key].concat(), &origin);
                }
            }
            out.push(0x00);
        }
        Ok(base64::engine::general_purpose::STANDARD.encode(out))
    }

    struct Cursor<'a>(&'a [u8]);

    impl<'a> Cursor<'a> {
        fn take(&mut self, n: usize) -> Result<&'a [u8], SendError> {
            if self.0.len() < n {
                return Err(unread());
            }
            let (head, rest) = self.0.split_at(n);
            self.0 = rest;
            Ok(head)
        }

        fn varint(&mut self) -> Result<usize, SendError> {
            let first = self.take(1)?[0];
            Ok(match first {
                0xfd => u16::from_le_bytes(self.take(2)?.try_into().expect("two")) as usize,
                0xfe => u32::from_le_bytes(self.take(4)?.try_into().expect("four")) as usize,
                0xff => return Err(unread()),
                n => n as usize,
            })
        }

        /// One map, its records in order, until the separator.
        fn map(&mut self) -> Result<Vec<(&'a [u8], &'a [u8])>, SendError> {
            let mut records = Vec::new();
            loop {
                let length = self.varint()?;
                if length == 0 {
                    return Ok(records);
                }
                let key = self.take(length)?;
                let length = self.varint()?;
                records.push((key, self.take(length)?));
            }
        }
    }

    fn place_of(
        policy: &MultisigPolicy,
        records: &[(&[u8], &[u8])],
        kind: u8,
    ) -> Result<Option<Place>, SendError> {
        let mut found = None;
        for (key, value) in records.iter().filter(|(key, _)| key.first() == Some(&kind)) {
            let public = PublicKey::from_slice(&key[1..]).map_err(|_| unread())?;
            if value.len() < 4 || (value.len() - 4) % 4 != 0 {
                return Err(unread());
            }
            let fingerprint = Fingerprint::from(<[u8; 4]>::try_from(&value[..4]).expect("four"));
            let path: DerivationPath = value[4..]
                .chunks(4)
                .map(|step| ChildNumber::from(u32::from_le_bytes(step.try_into().expect("four"))))
                .collect::<Vec<_>>()
                .into();
            let Some(place) = policy.place_of(fingerprint, &path) else {
                continue;
            };
            if found.is_some_and(|known| known != place)
                || !policy.keys(place)?.iter().any(|(own, _, _)| *own == public)
            {
                return Err(SendError::invalid(
                    "The PSBT's key origins do not match the wallet's keys.",
                ));
            }
            found = Some(place);
        }
        Ok(found)
    }

    /// The spent output `value` holds: a `CTxOut`, or, as some writers
    /// keep it, the whole transaction `outpoint` names.
    fn spent_output(value: &[u8], outpoint: &OutPoint) -> Result<TxOut, SendError> {
        let mut cursor = Cursor(value);
        let parsed = (|| {
            let amount = u64::from_le_bytes(cursor.take(8)?.try_into().expect("eight"));
            let length = cursor.varint()?;
            let script = cursor.take(length)?.to_vec();
            if !cursor.0.is_empty() {
                return Err(unread());
            }
            Ok(TxOut {
                value: bitcoin::Amount::from_sat(amount),
                script_pubkey: ScriptBuf::from(script),
            })
        })();
        if let Ok(output) = parsed {
            return Ok(output);
        }
        let previous: Transaction = bitcoin::consensus::deserialize(value).map_err(|_| unread())?;
        if previous.compute_txid() != outpoint.txid {
            return Err(unread());
        }
        previous
            .output
            .get(outpoint.vout as usize)
            .cloned()
            .ok_or_else(unread)
    }

    pub(super) fn decode(
        policy: &MultisigPolicy,
        chain: Chain,
        text: &str,
    ) -> Result<P2shSpend, SendError> {
        use base64::Engine;
        let text: String = text.split_whitespace().collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| SendError::invalid("Not a PSBT: it is not base64."))?;
        let mut cursor = Cursor(&bytes);
        if cursor.take(MAGIC.len())? != MAGIC {
            return Err(SendError::invalid("Not a PSBT."));
        }
        let global = cursor.map()?;
        let [([0x00], unsigned)] = global.as_slice() else {
            return Err(unread());
        };
        let tx: Transaction = bitcoin::consensus::deserialize(unsigned).map_err(|_| unread())?;
        let mut inputs = Vec::with_capacity(tx.input.len());
        for (index, txin) in tx.input.iter().enumerate() {
            let records = cursor.map()?;
            let foreign = || {
                SendError::refused(
                    "Input %@ does not belong to this wallet.",
                    [index.to_string()],
                )
            };
            let place = place_of(policy, &records, 0x06)?.ok_or_else(foreign)?;
            let mut utxo = None;
            let mut raw_signatures = Vec::new();
            for (key, value) in &records {
                match (key.first(), key.len()) {
                    (Some(0x00), 1) => utxo = Some(spent_output(value, &txin.previous_output)?),
                    (Some(0x02), 34) => raw_signatures.push(value.to_vec()),
                    (Some(0x03), 1) if *value == [SIGHASH_ALL_FORKID, 0, 0, 0] => {}
                    (Some(0x04), 1) => {
                        if *value != policy.witness_script(place)?.as_bytes() {
                            return Err(SendError::refused(
                                "Input %@'s script is not the wallet's script at its key origins.",
                                [index.to_string()],
                            ));
                        }
                    }
                    (Some(0x06), 34) => {}
                    (Some(0x07), 1) => {
                        return Err(SendError::invalid(
                            "The PSBT is already finalized: broadcast its transaction instead.",
                        ));
                    }
                    _ => return Err(unread()),
                }
            }
            let utxo = utxo.ok_or_else(foreign)?;
            if utxo.script_pubkey != policy.script_pubkey(place)? {
                return Err(SendError::refused(
                    "Input %@'s script is not the wallet's script at its key origins.",
                    [index.to_string()],
                ));
            }
            inputs.push((
                SpentInput {
                    value: utxo.value.to_sat(),
                    place,
                    signatures: BTreeMap::new(),
                },
                raw_signatures,
            ));
        }
        let mut change = Vec::with_capacity(tx.output.len());
        for _ in &tx.output {
            let records = cursor.map()?;
            let place = place_of(policy, &records, 0x02)?;
            for (key, value) in &records {
                match (key.first(), key.len(), place) {
                    (Some(0x00), 1, Some(place)) => {
                        if *value != policy.witness_script(place)?.as_bytes() {
                            return Err(SendError::invalid(
                                "An output claims to be the wallet's change but pays another script.",
                            ));
                        }
                    }
                    (Some(0x02), 34, _) => {}
                    _ => return Err(unread()),
                }
            }
            change.push(place);
        }
        if !cursor.0.is_empty() {
            return Err(unread());
        }
        let mut spent: Vec<SpentInput> = Vec::with_capacity(inputs.len());
        for (index, (mut input, signatures)) in inputs.into_iter().enumerate() {
            for signature in signatures {
                let cosigner = signer_of(policy, chain, &tx, index, &input, &signature)?;
                input.signatures.insert(cosigner, hex::encode(signature));
            }
            spent.push(input);
        }
        Ok(P2shSpend {
            transaction: tx,
            inputs: spent,
            change,
        })
    }
}

#[cfg(test)]
#[path = "tests/p2sh_multisig.rs"]
mod tests;

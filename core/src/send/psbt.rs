//! A multisig account's partially signed transactions (BIP-174): built from
//! the account's outputs, read from another coordinator, signed with one
//! cosigner's key, combined with the other cosigners' signatures and
//! finalized once the threshold has signed every input.
//!
//! Every PSBT is judged against the account before anything is signed or
//! kept: each input must pay one of the account's addresses through that
//! address's own witness script, and each output is either change — the
//! account's script at the place its key origins name — or a payment. A
//! signature already in it must be one of the input's keys over the input's
//! sighash. What a cosigner reviews is the unsigned transaction and the
//! amounts it spends, which no signature changes.

use bitcoin::bip32::Xpriv;
use bitcoin::ecdsa::Signature;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Message, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime, transaction::Version,
};

use crate::derivation::multisig::{MultisigPolicy, Place};
use crate::registry::Chain;
use crate::send::error::SendError;

/// One input as a cosigner reviews it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewedInput {
    pub outpoint: OutPoint,
    pub value: u64,
    pub place: Place,
    pub address: String,
    /// The cosigners (by their order in the policy) whose valid signature
    /// the input carries.
    pub signed_by: Vec<usize>,
}

/// One output: a payment, or change back to the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewedOutput {
    pub address: String,
    pub value: u64,
    pub change: Option<Place>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Review {
    pub txid: String,
    /// SHA-256 over the unsigned transaction and each input's amount: what a
    /// signature is given for.
    pub digest: String,
    pub inputs: Vec<ReviewedInput>,
    pub outputs: Vec<ReviewedOutput>,
    pub fee: u64,
    /// Every input carries the threshold's signatures.
    pub complete: bool,
}

impl Review {
    /// The cosigners who signed every input.
    pub(crate) fn signed_by(&self) -> Vec<usize> {
        let mut signers = self
            .inputs
            .first()
            .map(|input| input.signed_by.clone())
            .unwrap_or_default();
        signers.retain(|cosigner| {
            self.inputs
                .iter()
                .all(|input| input.signed_by.contains(cosigner))
        });
        signers
    }
}

/// The account's address at `place` as a script.
fn script_at(policy: &MultisigPolicy, place: Place) -> Result<(ScriptBuf, ScriptBuf), SendError> {
    let witness = policy.witness_script(place)?;
    Ok((ScriptBuf::new_p2wsh(&witness.wscript_hash()), witness))
}

/// The unsigned transaction spending `inputs` (outpoint, amount, place) to
/// `payments`, then change to `change`, with every input's and the change
/// output's script and key origins, as BIP-174's updater writes them.
pub(crate) fn build(
    policy: &MultisigPolicy,
    inputs: &[(OutPoint, u64, Place)],
    payments: &[(ScriptBuf, u64)],
    change: Option<(Place, u64)>,
) -> Result<Psbt, SendError> {
    let mut outputs: Vec<TxOut> = payments
        .iter()
        .map(|(script, value)| TxOut {
            value: Amount::from_sat(*value),
            script_pubkey: script.clone(),
        })
        .collect();
    if let Some((place, value)) = change {
        outputs.push(TxOut {
            value: Amount::from_sat(value),
            script_pubkey: script_at(policy, place)?.0,
        });
    }
    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs
            .iter()
            .map(|(outpoint, _, _)| TxIn {
                previous_output: *outpoint,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            })
            .collect(),
        output: outputs,
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).map_err(SendError::invalid)?;
    for (input, (_, value, place)) in psbt.inputs.iter_mut().zip(inputs) {
        let (script_pubkey, witness) = script_at(policy, *place)?;
        input.witness_utxo = Some(TxOut {
            value: Amount::from_sat(*value),
            script_pubkey,
        });
        input.witness_script = Some(witness);
        input.bip32_derivation = policy
            .keys(*place)?
            .into_iter()
            .map(|(key, fingerprint, path)| (key, (fingerprint, path)))
            .collect();
    }
    if let Some((place, _)) = change {
        let output = psbt
            .outputs
            .last_mut()
            .expect("the change output was pushed");
        output.witness_script = Some(policy.witness_script(place)?);
        output.bip32_derivation = policy
            .keys(place)?
            .into_iter()
            .map(|(key, fingerprint, path)| (key, (fingerprint, path)))
            .collect();
    }
    Ok(psbt)
}

pub(crate) fn decode(text: &str) -> Result<Psbt, SendError> {
    use base64::Engine;
    let text: String = text.split_whitespace().collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| SendError::invalid("Not a PSBT: it is not base64."))?;
    Psbt::deserialize(&bytes).map_err(|_| SendError::invalid("Not a PSBT."))
}

pub(crate) fn encode(psbt: &Psbt) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(psbt.serialize())
}

/// The place the key origins in `derivation` name in the account, refused
/// when they disagree or a key is not the account's key there.
fn place_named(
    policy: &MultisigPolicy,
    derivation: &std::collections::BTreeMap<
        bitcoin::secp256k1::PublicKey,
        bitcoin::bip32::KeySource,
    >,
) -> Result<Option<Place>, SendError> {
    let mut found = None;
    for (key, (fingerprint, path)) in derivation {
        let Some(place) = policy.place_of(*fingerprint, path) else {
            continue;
        };
        if found.is_some_and(|known| known != place)
            || !policy.keys(place)?.iter().any(|(own, _, _)| own == key)
        {
            return Err(SendError::invalid(
                "The PSBT's key origins do not match the wallet's keys.",
            ));
        }
        found = Some(place);
    }
    Ok(found)
}

/// `psbt` as the account's: every input its own, every output change or a
/// payment, every signature valid; refused otherwise.
pub(crate) fn review(
    policy: &MultisigPolicy,
    chain: Chain,
    psbt: &Psbt,
) -> Result<Review, SendError> {
    let network = chain
        .bitcoin_network()
        .ok_or_else(|| SendError::invalid("PSBTs are Bitcoin transactions"))?;
    let tx = &psbt.unsigned_tx;
    if tx.input.is_empty() || tx.output.is_empty() {
        return Err(SendError::invalid(
            "A PSBT spends at least one input to at least one output.",
        ));
    }
    let secp = Secp256k1::verification_only();
    let mut cache = SighashCache::new(tx);
    let mut inputs = Vec::with_capacity(tx.input.len());
    let mut seen = std::collections::HashSet::new();
    for (index, (txin, input)) in tx.input.iter().zip(&psbt.inputs).enumerate() {
        if !seen.insert(txin.previous_output) {
            return Err(SendError::invalid("The PSBT spends an output twice."));
        }
        if input.final_script_witness.is_some() || input.final_script_sig.is_some() {
            return Err(SendError::invalid(
                "The PSBT is already finalized: broadcast its transaction instead.",
            ));
        }
        let foreign = || {
            SendError::refused(
                "Input %@ does not belong to this wallet.",
                [index.to_string()],
            )
        };
        let place = place_named(policy, &input.bip32_derivation)?.ok_or_else(foreign)?;
        let (script_pubkey, witness) = script_at(policy, place)?;
        let utxo = match (&input.witness_utxo, &input.non_witness_utxo) {
            (Some(utxo), _) => utxo.clone(),
            (None, Some(previous)) if previous.compute_txid() == txin.previous_output.txid => {
                previous
                    .output
                    .get(txin.previous_output.vout as usize)
                    .cloned()
                    .ok_or_else(foreign)?
            }
            _ => return Err(foreign()),
        };
        if utxo.script_pubkey != script_pubkey
            || input
                .witness_script
                .as_ref()
                .is_some_and(|named| *named != witness)
        {
            return Err(SendError::refused(
                "Input %@'s script is not the wallet's script at its key origins.",
                [index.to_string()],
            ));
        }
        if input
            .sighash_type
            .is_some_and(|kind| kind.ecdsa_hash_ty().ok() != Some(EcdsaSighashType::All))
        {
            return Err(SendError::invalid(
                "Only SIGHASH_ALL signatures are made or accepted.",
            ));
        }
        let sighash = cache
            .p2wsh_signature_hash(index, &witness, utxo.value, EcdsaSighashType::All)
            .map_err(SendError::invalid)?;
        let message = Message::from_digest(bitcoin::hashes::Hash::to_byte_array(sighash));
        let keys = policy.keys(place)?;
        let mut signed_by = Vec::new();
        for (key, signature) in &input.partial_sigs {
            if let Some(cosigner) = keys.iter().position(|(own, _, _)| own == &key.inner) {
                signed_by.push(cosigner);
            }
            if !keys.iter().any(|(own, _, _)| own == &key.inner)
                || signature.sighash_type != EcdsaSighashType::All
                || secp
                    .verify_ecdsa(&message, &signature.signature, &key.inner)
                    .is_err()
            {
                return Err(SendError::refused(
                    "Input %@ carries a signature that is not a valid one from the wallet's keys.",
                    [index.to_string()],
                ));
            }
        }
        inputs.push(ReviewedInput {
            outpoint: txin.previous_output,
            value: utxo.value.to_sat(),
            place,
            address: Address::from_script(&script_pubkey, network)
                .map_err(SendError::invalid)?
                .to_string(),
            signed_by: {
                signed_by.sort_unstable();
                signed_by
            },
        });
    }
    let mut outputs = Vec::with_capacity(tx.output.len());
    for (txout, output) in tx.output.iter().zip(&psbt.outputs) {
        let change = match place_named(policy, &output.bip32_derivation)? {
            Some(place) => {
                let (script_pubkey, witness) = script_at(policy, place)?;
                if txout.script_pubkey != script_pubkey
                    || output
                        .witness_script
                        .as_ref()
                        .is_some_and(|named| *named != witness)
                {
                    return Err(SendError::invalid(
                        "An output claims to be the wallet's change but pays another script.",
                    ));
                }
                Some(place)
            }
            None => None,
        };
        outputs.push(ReviewedOutput {
            address: Address::from_script(&txout.script_pubkey, network)
                .map(|address| address.to_string())
                .unwrap_or_else(|_| format!("script {}", txout.script_pubkey.to_hex_string())),
            value: txout.value.to_sat(),
            change,
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
        .ok_or_else(|| SendError::invalid("The PSBT pays out more than it spends."))?;
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
    Ok(Review {
        txid: tx.compute_txid().to_string(),
        digest,
        inputs,
        outputs,
        fee,
        complete,
    })
}

/// Sign every input with the cosigner's key at its place, after `psbt`
/// reviews as the account's; the review is the one returned.
pub(crate) fn sign(
    policy: &MultisigPolicy,
    chain: Chain,
    psbt: &mut Psbt,
    account: &Xpriv,
) -> Result<Review, SendError> {
    let reviewed = review(policy, chain, psbt)?;
    let secp = Secp256k1::new();
    let mut cache = SighashCache::new(psbt.unsigned_tx.clone());
    for (index, input) in reviewed.inputs.iter().enumerate() {
        let (_, witness) = script_at(policy, input.place)?;
        let child = account
            .derive_priv(
                &secp,
                &[
                    bitcoin::bip32::ChildNumber::from_normal_idx(input.place.0)
                        .map_err(SendError::invalid)?,
                    bitcoin::bip32::ChildNumber::from_normal_idx(input.place.1)
                        .map_err(SendError::invalid)?,
                ],
            )
            .map_err(SendError::invalid)?;
        let key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &child.private_key);
        if !policy
            .keys(input.place)?
            .iter()
            .any(|(own, _, _)| *own == key)
        {
            return Err(SendError::invalid(
                "The cosigner key is not one of the input's keys.",
            ));
        }
        let sighash = cache
            .p2wsh_signature_hash(
                index,
                &witness,
                Amount::from_sat(input.value),
                EcdsaSighashType::All,
            )
            .map_err(SendError::invalid)?;
        let signature = secp.sign_ecdsa(
            &Message::from_digest(bitcoin::hashes::Hash::to_byte_array(sighash)),
            &child.private_key,
        );
        let psbt_input = &mut psbt.inputs[index];
        psbt_input.witness_script = Some(witness);
        psbt_input.partial_sigs.insert(
            bitcoin::PublicKey::new(key),
            Signature {
                signature,
                sighash_type: EcdsaSighashType::All,
            },
        );
    }
    review(policy, chain, psbt)
}

/// Merge another cosigner's copy of the same transaction into `psbt`: its
/// unsigned transaction must be the one reviewed, and it must review as the
/// account's, signatures included.
pub(crate) fn combine(
    policy: &MultisigPolicy,
    chain: Chain,
    psbt: &mut Psbt,
    other: Psbt,
) -> Result<Review, SendError> {
    if other.unsigned_tx != psbt.unsigned_tx {
        return Err(SendError::invalid(
            "This PSBT is another transaction: its inputs or outputs changed.",
        ));
    }
    review(policy, chain, &other)?;
    psbt.combine(other).map_err(SendError::invalid)?;
    review(policy, chain, psbt)
}

/// The finished transaction: each input's witness the threshold's
/// signatures in the script's key order, then the script.
pub(crate) fn finalize(
    policy: &MultisigPolicy,
    chain: Chain,
    psbt: &Psbt,
) -> Result<Transaction, SendError> {
    let reviewed = review(policy, chain, psbt)?;
    if !reviewed.complete {
        return Err(SendError::invalid(
            "The PSBT does not yet carry enough signatures.",
        ));
    }
    let mut psbt = psbt.clone();
    for (input, reviewed) in psbt.inputs.iter_mut().zip(&reviewed.inputs) {
        let witness_script = policy.witness_script(reviewed.place)?;
        let mut keys: Vec<[u8; 33]> = policy
            .keys(reviewed.place)?
            .into_iter()
            .map(|(key, _, _)| key.serialize())
            .collect();
        keys.sort_unstable();
        let mut witness = Witness::new();
        witness.push([]);
        let mut signed = 0;
        for key in keys {
            if signed == policy.threshold {
                break;
            }
            if let Some(signature) = input
                .partial_sigs
                .iter()
                .find(|(own, _)| own.inner.serialize() == key)
                .map(|(_, signature)| signature)
            {
                witness.push(signature.to_vec());
                signed += 1;
            }
        }
        witness.push(witness_script.as_bytes());
        input.final_script_witness = Some(witness);
        input.partial_sigs.clear();
        input.sighash_type = None;
        input.witness_script = None;
        input.bip32_derivation.clear();
    }
    psbt.extract_tx().map_err(SendError::invalid)
}

/// The virtual size of a transaction spending `inputs` of the account's
/// P2WSH outputs to outputs of these script lengths, each signature at its
/// largest DER size.
pub(crate) fn estimate_vsize(
    policy: &MultisigPolicy,
    inputs: usize,
    output_scripts: impl IntoIterator<Item = usize>,
) -> u64 {
    let varint = |n: usize| match n {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        _ => 5,
    };
    let script = 3 + 34 * policy.cosigners.len();
    let witness_per_input =
        varint(policy.threshold + 2) + 1 + policy.threshold * 73 + varint(script) + script;
    let outputs: Vec<usize> = output_scripts.into_iter().collect();
    let base = 4
        + 4
        + varint(inputs)
        + varint(outputs.len())
        + inputs * 41
        + outputs
            .iter()
            .map(|len| 8 + varint(*len) + len)
            .sum::<usize>();
    let weight = base * 4 + 2 + inputs * witness_per_input;
    weight.div_ceil(4) as u64
}

#[cfg(test)]
#[path = "tests/psbt.rs"]
mod tests;

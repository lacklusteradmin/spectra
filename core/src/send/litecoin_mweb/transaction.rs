//! Building MWEB transactions: spending the wallet's outputs, paying
//! stealth addresses, pegging in from and out to canonical Litecoin.
//!
//! The commitments balance as Litecoin Core checks them: the outputs' less
//! the inputs' equals the kernel's excess, the kernel offset times `G`, and
//! the value the kernel adds (its peg-in, less its fee and peg-outs) times
//! `H`. The stealth keys balance the same way: the outputs' sender keys and
//! the inputs' keys, less the spent outputs' keys, equal the stealth offset
//! times `G` plus the kernel's stealth excess.

use secp256k1::{PublicKey, SecretKey};

use super::keys::StealthAddress;
use super::output::{self, OwnedOutput};
use super::primitives::{self, blake3};
use crate::api::litecoin_p2p::wire::{
    ExtendedTransaction, Input, Kernel, MwebTx, Output, PegOut, RangeProof, Reader, TxBody,
};
use crate::send::error::SendError;

/// The sender's signature, and with a full proof the range proof.
pub(crate) fn verify_output(output: &Output) -> bool {
    primitives::verify(
        &output.sender,
        &output.signature_message(),
        &output.signature,
    ) && match &output.range_proof {
        RangeProof::Full(proof) => {
            primitives::verify_range(&output.commitment, proof, &output.message.encode())
        }
        RangeProof::Hash(_) => true,
    }
}

/// The key an input signs with: `Ko·H(Ki ‖ Ko) + Ki` with a stealth input
/// key, else the output key.
pub(crate) fn input_signing_key(input: &Input) -> Result<PublicKey, SendError> {
    match &input.input_key {
        Some(input_key) => {
            let mut preimage = input_key.serialize().to_vec();
            preimage.extend(input.output_key.serialize());
            primitives::add(
                &primitives::mul(&input.output_key, &blake3(&preimage))?,
                input_key,
            )
        }
        None => Ok(input.output_key),
    }
}

pub(crate) fn verify_input(input: &Input) -> bool {
    input_signing_key(input)
        .is_ok_and(|key| primitives::verify(&key, &input.signature_message(), &input.signature))
}

/// The key a kernel signs with: `E·H(E ‖ E′) + E′` with a stealth excess
/// `E′`, both as public keys, else the excess.
pub(crate) fn kernel_signing_key(kernel: &Kernel) -> Result<PublicKey, SendError> {
    let excess = primitives::commitment_point(&kernel.excess)?;
    match &kernel.stealth_excess {
        Some(stealth) => {
            let mut preimage = excess.serialize().to_vec();
            preimage.extend(stealth.serialize());
            primitives::add(&primitives::mul(&excess, &blake3(&preimage))?, stealth)
        }
        None => Ok(excess),
    }
}

pub(crate) fn verify_kernel(kernel: &Kernel) -> bool {
    kernel_signing_key(kernel)
        .is_ok_and(|key| primitives::verify(&key, &kernel.signature_message(), &kernel.signature))
}

/// MWEB fees, as Litecoin Core weighs a transaction: each unit of weight
/// costs 100 litoshis.
pub(crate) const FEE_PER_WEIGHT: u64 = 100;
/// A kernel with a stealth excess.
pub(crate) const KERNEL_WEIGHT: u64 = 3;
/// An output with standard fields.
pub(crate) const OUTPUT_WEIGHT: u64 = 18;
/// Bytes of a peg-out script per unit of weight.
const BYTES_PER_WEIGHT: u64 = 42;
/// What a peg-out's canonical output pays per kilobyte: ten times Litecoin
/// Core's incremental relay fee, a few hundred litoshis for a typical one.
pub(crate) const PEGOUT_FEE_PER_KB: u64 = 10_000;
/// The most inputs Litecoin Core relays in one MWEB transaction.
pub(crate) const MAX_STANDARD_INPUTS: usize = 1_000;

/// The fee of a transaction with `outputs` MWEB outputs and `pegouts`
/// canonical outputs: its weight at 100 litoshis, and the peg-outs'
/// serialized size at `fee_per_kb`, rounded up, as ltcd's `EstimateFee`.
pub(crate) fn fee(outputs: u64, pegouts: &[PegOut], fee_per_kb: u64) -> Result<u64, SendError> {
    let overflow = || SendError::invalid("MWEB fee overflow");
    let mut weight = KERNEL_WEIGHT
        .checked_add(outputs.checked_mul(OUTPUT_WEIGHT).ok_or_else(overflow)?)
        .ok_or_else(overflow)?;
    let mut size: u64 = 0;
    for pegout in pegouts {
        let script = pegout.script.len() as u64;
        weight = weight
            .checked_add(script.div_ceil(BYTES_PER_WEIGHT))
            .ok_or_else(overflow)?;
        // value, CompactSize length, script
        let compact = match script {
            0..=0xfc => 1,
            0xfd..=0xffff => 3,
            _ => 5,
        };
        size = size
            .checked_add(8 + compact + script)
            .ok_or_else(overflow)?;
    }
    let canonical = size
        .checked_mul(fee_per_kb)
        .ok_or_else(overflow)?
        .div_ceil(1000);
    weight
        .checked_mul(FEE_PER_WEIGHT)
        .and_then(|fee| fee.checked_add(canonical))
        .ok_or_else(overflow)
}

/// An owned output to spend, with the secret of its output key.
pub(crate) struct Spend {
    pub coin: OwnedOutput,
    pub output_secret: SecretKey,
}

/// What a transaction moves.
pub(crate) struct Plan {
    pub spends: Vec<Spend>,
    /// Stealth addresses and what each receives; change is one of them.
    pub recipients: Vec<(StealthAddress, u64)>,
    pub fee: u64,
    /// What a peg-in brings in from canonical Litecoin.
    pub pegin: u64,
    pub pegouts: Vec<PegOut>,
}

fn random_secret() -> SecretKey {
    SecretKey::new(&mut rand::rngs::OsRng)
}

fn sum(keys: impl IntoIterator<Item = SecretKey>) -> Result<Option<SecretKey>, SendError> {
    let mut total: Option<SecretKey> = None;
    for key in keys {
        total = Some(match total {
            Some(total) => primitives::add_secrets(&total, &key)?,
            None => key,
        });
    }
    Ok(total)
}

/// Build and sign `plan`'s transaction.
pub(crate) fn build(plan: &Plan) -> Result<MwebTx, SendError> {
    let overflow = || SendError::invalid("MWEB value overflow");
    let spent = plan
        .spends
        .iter()
        .try_fold(0u64, |total, spend| total.checked_add(spend.coin.value))
        .ok_or_else(overflow)?;
    let paid = plan
        .recipients
        .iter()
        .try_fold(0u64, |total, (_, value)| total.checked_add(*value))
        .ok_or_else(overflow)?;
    let pegged_out = plan
        .pegouts
        .iter()
        .try_fold(0u64, |total, pegout| total.checked_add(pegout.amount))
        .ok_or_else(overflow)?;
    let incoming = spent.checked_add(plan.pegin).ok_or_else(overflow)?;
    let outgoing = paid
        .checked_add(pegged_out)
        .and_then(|total| total.checked_add(plan.fee))
        .ok_or_else(overflow)?;
    if incoming != outgoing {
        return Err(SendError::Internal("MWEB amounts do not balance".into()));
    }
    if plan.recipients.is_empty() && plan.pegouts.is_empty() {
        return Err(SendError::Internal(
            "an MWEB transaction paying no one".into(),
        ));
    }

    // Outputs, each under a sender key of its own.
    let mut outputs = Vec::new();
    let mut sender_keys = Vec::new();
    for (address, value) in &plan.recipients {
        let sender = random_secret();
        outputs.push(output::create_output(address, *value, &sender)?);
        sender_keys.push(sender);
    }

    // Blindings: Σ outputs − Σ inputs = kernel blind + kernel offset.
    let kernel_offset = random_secret();
    let mut negatives = vec![kernel_offset];
    for spend in &plan.spends {
        negatives.push(primitives::blind_switch(
            &spend.coin.mask_blind()?,
            spend.coin.value,
        )?);
    }
    let positives = sum(outputs.iter().map(|created| created.blind))?;
    let negatives = sum(negatives)?.expect("the kernel offset at least");
    let kernel_blind = match positives {
        Some(positives) => primitives::sub_secrets(&positives, &negatives)?,
        None => negatives.negate(),
    };

    // Inputs, each signed by its output key and a fresh input key.
    let mut inputs = Vec::new();
    let mut input_keys = Vec::new();
    for spend in &plan.spends {
        let input_secret = random_secret();
        let output_key = primitives::public(&spend.output_secret);
        if output_key.serialize() != spend.coin.output_key {
            return Err(SendError::invalid(
                "An MWEB output's key does not match the wallet's",
            ));
        }
        let commitment = primitives::switch_commit(&spend.coin.mask_blind()?, spend.coin.value)?;
        if commitment.0 != spend.coin.commitment {
            return Err(SendError::invalid(
                "An MWEB output's commitment does not match the wallet's",
            ));
        }
        let mut input = Input {
            output_id: spend.coin.output_id,
            commitment,
            input_key: Some(primitives::public(&input_secret)),
            output_key,
            extra: None,
            signature: [0; 64],
        };
        let mut preimage = primitives::public(&input_secret).serialize().to_vec();
        preimage.extend(output_key.serialize());
        let signing = primitives::add_secrets(
            &primitives::mul_secrets(&spend.output_secret, &blake3(&preimage))?,
            &input_secret,
        )?;
        input.signature = primitives::sign(&signing, &input.signature_message())?;
        inputs.push(input);
        input_keys.push(primitives::sub_secrets(
            &input_secret,
            &spend.output_secret,
        )?);
    }

    // The kernel, signed by its excess and a stealth excess.
    let stealth_blind = random_secret();
    let mut kernel = Kernel {
        fee: Some(plan.fee),
        pegin: (plan.pegin > 0).then_some(plan.pegin),
        pegouts: plan.pegouts.clone(),
        lock_height: None,
        stealth_excess: Some(primitives::public(&stealth_blind)),
        extra: None,
        excess: primitives::commit(&kernel_blind, 0)?,
        signature: [0; 64],
    };
    let mut preimage = primitives::public(&kernel_blind).serialize().to_vec();
    preimage.extend(primitives::public(&stealth_blind).serialize());
    let signing = primitives::add_secrets(
        &primitives::mul_secrets(&kernel_blind, &blake3(&preimage))?,
        &stealth_blind,
    )?;
    kernel.signature = primitives::sign(&signing, &kernel.signature_message())?;

    // Stealth offset: Σ sender keys + Σ (input key − output key) − stealth blind.
    let stealth_offset = match sum(sender_keys.into_iter().chain(input_keys))? {
        Some(total) => primitives::sub_secrets(&total, &stealth_blind)?,
        None => stealth_blind.negate(),
    };

    let mut body = TxBody {
        inputs,
        outputs: outputs
            .iter()
            .map(|created| created.output.clone())
            .collect(),
        kernels: vec![kernel],
    };
    body.sort();
    Ok(MwebTx {
        kernel_offset: kernel_offset.secret_bytes(),
        stealth_offset: stealth_offset.secret_bytes(),
        body,
    })
}

/// Whether `tx` is valid as Litecoin Core checks a transaction alone: every
/// signature and range proof, the order of its parts, and both balances.
pub(crate) fn verify(tx: &MwebTx) -> Result<(), SendError> {
    let refuse = |what: &str| {
        Err(SendError::invalid(format!(
            "Invalid MWEB transaction: {what}"
        )))
    };
    let body = &tx.body;
    if !body.inputs.iter().all(verify_input) {
        return refuse("an input signature");
    }
    if !body.outputs.iter().all(verify_output) {
        return refuse("an output signature or range proof");
    }
    if !body.kernels.iter().all(verify_kernel) {
        return refuse("a kernel signature");
    }
    let mut sorted = body.clone();
    sorted.sort();
    if sorted != *body {
        return refuse("the order of its parts");
    }
    // Commitments.
    let points = |commitments: &mut dyn Iterator<Item = &primitives::Commitment>| -> Result<Vec<PublicKey>, SendError> {
        commitments.map(primitives::commitment_point).collect()
    };
    let outputs = points(&mut body.outputs.iter().map(|o| &o.commitment))?;
    let inputs = points(&mut body.inputs.iter().map(|i| &i.commitment))?;
    let mut right = points(&mut body.kernels.iter().map(|k| &k.excess))?;
    right.push(primitives::public(&primitives::secret(tx.kernel_offset)?));
    let supply: i128 = body.kernels.iter().map(Kernel::supply_change).sum();
    let mut left = outputs;
    // Value the kernels carry in: positive on the right, negative on the left.
    let value_point = |value: u128| {
        primitives::value_point(
            u64::try_from(value).map_err(|_| SendError::invalid("MWEB value overflow"))?,
        )
    };
    if supply > 0 {
        right.push(value_point(supply.unsigned_abs())?);
    } else if supply < 0 {
        left.push(value_point(supply.unsigned_abs())?);
    }
    let combine = |points: &[PublicKey], negatives: &[PublicKey]| -> Option<PublicKey> {
        let negated: Vec<PublicKey> = negatives
            .iter()
            .map(|p| p.negate(secp256k1::SECP256K1))
            .collect();
        let all: Vec<&PublicKey> = points.iter().chain(negated.iter()).collect();
        PublicKey::combine_keys(&all).ok()
    };
    // Σ outputs (+ negative supply) − Σ inputs == Σ excess + offset·G (+ positive supply).
    let lhs = combine(&left, &inputs);
    let rhs = combine(&right, &[]);
    if lhs.is_none() || lhs != rhs {
        return refuse("its commitments do not balance");
    }
    // Stealth keys.
    let mut positive: Vec<PublicKey> = body.outputs.iter().map(|o| o.sender).collect();
    positive.extend(body.inputs.iter().filter_map(|i| i.input_key));
    let spent: Vec<PublicKey> = body.inputs.iter().map(|i| i.output_key).collect();
    let mut stealth: Vec<PublicKey> = body
        .kernels
        .iter()
        .filter_map(|k| k.stealth_excess)
        .collect();
    if let Ok(offset) = primitives::secret(tx.stealth_offset) {
        stealth.push(primitives::public(&offset));
    }
    if combine(&positive, &spent) != combine(&stealth, &[]) {
        return refuse("its stealth keys do not balance");
    }
    Ok(())
}

/// The Litecoin transaction an MWEB transaction travels in: `canonical`'s
/// inputs and outputs (none for a payment inside MWEB or a peg-out; a
/// peg-in's), marked by flag bit `0x08` and followed by the MWEB
/// transaction.
pub(crate) fn litecoin_transaction(canonical: &bitcoin::Transaction, mweb: &MwebTx) -> Vec<u8> {
    use bitcoin::consensus::Encodable;
    let mut out = Vec::new();
    canonical
        .version
        .consensus_encode(&mut out)
        .expect("writing to memory");
    let segwit = canonical
        .input
        .iter()
        .any(|input| !input.witness.is_empty());
    out.push(0x00);
    out.push(0x08 | if segwit { 0x01 } else { 0x00 });
    canonical
        .input
        .consensus_encode(&mut out)
        .expect("writing to memory");
    canonical
        .output
        .consensus_encode(&mut out)
        .expect("writing to memory");
    if segwit {
        for input in &canonical.input {
            input
                .witness
                .consensus_encode(&mut out)
                .expect("writing to memory");
        }
    }
    out.push(0x01);
    mweb.encode(&mut out);
    canonical
        .lock_time
        .consensus_encode(&mut out)
        .expect("writing to memory");
    out
}

/// A Litecoin transaction's id as explorers write it: for one with no
/// canonical inputs or outputs, its first kernel's id; otherwise the hash of
/// its canonical part, without witnesses or MWEB data.
pub(crate) fn litecoin_txid(canonical: &bitcoin::Transaction, mweb: &MwebTx) -> String {
    use bitcoin::hashes::Hash as _;
    let mut bytes = if canonical.input.is_empty() && canonical.output.is_empty() {
        mweb.body
            .kernels
            .first()
            .map(Kernel::id)
            .unwrap_or_default()
    } else {
        canonical.compute_txid().to_byte_array()
    };
    bytes.reverse();
    hex::encode(bytes)
}

/// The MWEB outputs of a transaction `litecoin_transaction` wrote.
pub(crate) fn mweb_outputs(raw: &[u8]) -> Result<Vec<Output>, SendError> {
    let mut reader = Reader(raw);
    let tx = ExtendedTransaction::decode(&mut reader)?;
    reader.finished()?;
    Ok(tx.mweb.map(|mweb| mweb.body.outputs).unwrap_or_default())
}

//! Peercoin ordinary transfers. Version 3 omits the historical timestamp in
//! both transaction serialization and legacy/BIP143 signature preimages.
//! Fees use the full serialized size, including undiscounted witness bytes.
//!
//! Protocol references:
//! https://github.com/peercoin/peercoin/blob/master/src/primitives/transaction.h
//! https://github.com/peercoin/peercoin/blob/master/src/script/interpreter.cpp
//! https://github.com/peercoin/peercoin/blob/master/src/consensus/tx_verify.cpp
//! https://github.com/peercoin/peercoin/blob/master/src/wallet/spend.cpp

use std::collections::HashSet;
use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::{Hash, hash160};
use bitcoin::key::TapTweak;
use bitcoin::script::{Builder, PushBytesBuf};
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::sighash::{EcdsaSighashType, Prevouts, SighashCache, TapSighashType};
use bitcoin::transaction::Version;
use bitcoin::{
    Amount, CompressedPublicKey, OutPoint, Script, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};

use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;
use crate::send::error::SendError;

type Input = (String, u32, u64, Vec<u8>);

#[cfg(test)]
#[path = "tests/peercoin.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    P2pk,
    P2pkh,
    P2wpkh,
    NestedP2wpkh,
    P2tr,
}

impl InputKind {
    fn from_script(script: &Script) -> Result<Self, SendError> {
        if script.is_p2pk() {
            Ok(Self::P2pk)
        } else if script.is_p2pkh() {
            Ok(Self::P2pkh)
        } else if script.is_p2wpkh() {
            Ok(Self::P2wpkh)
        } else if script.is_p2sh() {
            Ok(Self::NestedP2wpkh)
        } else if script.is_p2tr() {
            Ok(Self::P2tr)
        } else {
            Err(SendError::invalid("unsupported Peercoin input script"))
        }
    }

    fn has_witness(self) -> bool {
        matches!(self, Self::P2wpkh | Self::NestedP2wpkh | Self::P2tr)
    }

    fn validate_key(self, script: &Script, key: &CompressedPublicKey) -> Result<(), SendError> {
        let valid = match self {
            Self::P2pk => script
                .p2pk_public_key()
                .is_some_and(|public| public.inner == key.0),
            Self::P2pkh => *script == ScriptBuf::new_p2pkh(&key.pubkey_hash()),
            Self::P2wpkh => *script == ScriptBuf::new_p2wpkh(&key.wpubkey_hash()),
            Self::NestedP2wpkh => *script == ScriptBuf::new_p2wpkh(&key.wpubkey_hash()).to_p2sh(),
            Self::P2tr => {
                *script == ScriptBuf::new_p2tr(&Secp256k1::new(), key.0.x_only_public_key().0, None)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(SendError::invalid(
                "Peercoin source script does not match the signing key",
            ))
        }
    }
}

/// Blockbook maps generated P2PK outputs to their HASH160 address. Keep the
/// real prevout script while verifying that it belongs to the fetched sender.
pub(crate) fn peercoin_input_matches_sender(input: &[u8], sender: &[u8]) -> bool {
    if input == sender {
        return InputKind::from_script(Script::from_bytes(input)).is_ok();
    }
    let input = Script::from_bytes(input);
    if input.is_p2pk() && input.p2pk_public_key().is_some() {
        let public_bytes = &input.as_bytes()[1..input.len() - 1];
        let hash =
            bitcoin::PubkeyHash::from_byte_array(hash160::Hash::hash(public_bytes).to_byte_array());
        return ScriptBuf::new_p2pkh(&hash).as_bytes() == sender;
    }
    false
}

fn validate_inventory(chain: Chain, inputs: &[Input]) -> Result<u64, SendError> {
    let maximum = chain.peercoin_max_money()?;
    if inputs.is_empty() {
        return Err(SendError::invalid("Peercoin transaction must have inputs"));
    }
    let mut seen = HashSet::new();
    inputs
        .iter()
        .try_fold(0u64, |total, (txid, vout, value, script)| {
            let outpoint = OutPoint {
                txid: Txid::from_str(txid).map_err(SendError::invalid)?,
                vout: *vout,
            };
            if outpoint.is_null() || !seen.insert(outpoint) {
                return Err(SendError::invalid(
                    "invalid or duplicate Peercoin input outpoint",
                ));
            }
            InputKind::from_script(Script::from_bytes(script))?;
            if *value == 0 || *value > maximum {
                return Err(SendError::invalid(
                    "Peercoin input value is outside MoneyRange",
                ));
            }
            total
                .checked_add(*value)
                .ok_or_else(|| SendError::invalid("Peercoin wallet input total overflow"))
        })
}

fn validate_inputs(chain: Chain, inputs: &[Input]) -> Result<u64, SendError> {
    let total = validate_inventory(chain, inputs)?;
    if total > chain.peercoin_max_money()? {
        return Err(SendError::invalid(
            "Peercoin transaction input total exceeds MAX_MONEY",
        ));
    }
    Ok(total)
}

fn compact_size(value: u64) -> u64 {
    match value {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// Maximum serialized size with a low-S DER signature plus SIGHASH byte
/// (at most 72 bytes). Witness bytes receive no discount in Peercoin fees.
fn estimate_bytes(
    inputs: &[Input],
    recipient: &[u8],
    change: Option<&[u8]>,
) -> Result<u64, SendError> {
    let overflow = || SendError::invalid("Peercoin transaction size overflow");
    let mut base = 8u64 + compact_size(inputs.len() as u64) + 1;
    let mut witness = 0u64;
    let mut has_witness = false;
    for (_, _, _, script) in inputs {
        let kind = InputKind::from_script(Script::from_bytes(script))?;
        let (input_bytes, witness_bytes) = match kind {
            InputKind::P2pk => (114, 1),
            InputKind::P2pkh => (148, 1),
            InputKind::P2wpkh => (41, 108),
            InputKind::NestedP2wpkh => (64, 108),
            InputKind::P2tr => (41, 66),
        };
        has_witness |= kind.has_witness();
        base = base.checked_add(input_bytes).ok_or_else(overflow)?;
        witness = witness.checked_add(witness_bytes).ok_or_else(overflow)?;
    }
    for script in std::iter::once(recipient).chain(change) {
        let len = u64::try_from(script.len()).map_err(SendError::invalid)?;
        base = base
            .checked_add(8 + compact_size(len))
            .and_then(|n| n.checked_add(len))
            .ok_or_else(overflow)?;
    }
    let bytes = if has_witness {
        base.checked_add(witness)
            .and_then(|n| n.checked_add(2))
            .ok_or_else(overflow)?
    } else {
        base
    };
    Ok(bytes)
}

pub(crate) fn peercoin_minimum_fee(chain: Chain, bytes: u64) -> Result<u64, SendError> {
    let per_byte = chain.peercoin_fee_per_kb_units()? / 1_000;
    let minimum = chain.peercoin_min_fee_units()?;
    let maximum = chain.peercoin_max_money()?;
    bytes
        .checked_mul(per_byte)
        .map(|fee| fee.max(minimum))
        .filter(|fee| *fee <= maximum)
        .ok_or_else(|| SendError::invalid("Peercoin fee exceeds MAX_MONEY"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PeercoinFeeQuote {
    pub fee: u64,
    pub change: u64,
    pub estimated_bytes: u64,
    pub max_sendable: u64,
}

pub(crate) struct PeercoinSelection {
    /// Indices into the validated inventory, retained with their owned sources.
    pub indices: Vec<usize>,
    pub quote: PeercoinFeeQuote,
    pub spendable_balance: u64,
}

/// Select only the largest inputs needed for an ordinary spend. The capacity
/// uses the complete inventory, retaining a deterministic largest-first set
/// within the transaction's MoneyRange and size limits. Wallet totals can be
/// larger than a single transaction's consensus limit.
pub(crate) fn select_peercoin_inputs(
    chain: Chain,
    inputs: &[Input],
    amount: u64,
    recipient_script: &[u8],
    change_script: &[u8],
    requested_fee: Option<u64>,
) -> Result<PeercoinSelection, SendError> {
    let minimum_output = chain.peercoin_min_output_units()?;
    let maximum = chain.peercoin_max_money()?;
    if amount < minimum_output || amount > maximum {
        return Err(SendError::invalid(
            "Peercoin amount is outside the supported range",
        ));
    }
    let spendable_balance = validate_inventory(chain, inputs)?;
    let mut ordered: Vec<_> = (0..inputs.len()).collect();
    ordered.sort_by(|a, b| {
        inputs[*b]
            .2
            .cmp(&inputs[*a].2)
            .then_with(|| (&inputs[*a].0, inputs[*a].1).cmp(&(&inputs[*b].0, inputs[*b].1)))
    });
    let mut capacity_inputs = Vec::new();
    let mut capacity_indices = Vec::new();
    let mut capacity_total = 0u64;
    let mut max_sendable = 0u64;
    for index in ordered {
        let Some(total) = capacity_total
            .checked_add(inputs[index].2)
            .filter(|n| *n <= maximum)
        else {
            continue;
        };
        capacity_inputs.push(inputs[index].clone());
        let Ok(size) = estimate_bytes(&capacity_inputs, recipient_script, None) else {
            capacity_inputs.pop();
            continue;
        };
        // Weight never exceeds four times raw size, so this conservative
        // bound respects the standard 400 kWU transaction limit.
        if size > 100_000 {
            capacity_inputs.pop();
            continue;
        }
        let minimum_fee = peercoin_minimum_fee(chain, size)?;
        let fee = requested_fee.unwrap_or(minimum_fee);
        capacity_total = total;
        if fee >= minimum_fee {
            max_sendable = max_sendable.max(total.saturating_sub(fee));
        }
        capacity_indices.push(index);
    }
    if max_sendable < minimum_output {
        max_sendable = 0;
    }
    let mut selected = Vec::new();
    for (position, input) in capacity_inputs.iter().enumerate() {
        selected.push(input.clone());
        match quote_peercoin_fee(
            chain,
            &selected,
            amount,
            recipient_script,
            change_script,
            requested_fee,
        ) {
            Ok(mut quote) => {
                quote.max_sendable = max_sendable;
                return Ok(PeercoinSelection {
                    indices: capacity_indices[..=position].to_vec(),
                    quote,
                    spendable_balance,
                });
            }
            Err(SendError::InsufficientFunds(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Err(SendError::insufficient_funds())
}

/// Quote the same all-input layout the reviewed transaction signs. A maximum
/// spend uses one output. Smaller spends keep change >= 0.01 PPC; smaller
/// residuals join the reviewed fee, matching Peercoin Core's wallet policy.
pub(crate) fn quote_peercoin_fee(
    chain: Chain,
    inputs: &[Input],
    amount: u64,
    recipient_script: &[u8],
    sender_script: &[u8],
    requested_fee: Option<u64>,
) -> Result<PeercoinFeeQuote, SendError> {
    let minimum_output = chain.peercoin_min_output_units()?;
    if amount < minimum_output {
        return Err(SendError::invalid(
            "Peercoin amount must be at least 0.01 PPC",
        ));
    }
    let recipient = Script::from_bytes(recipient_script);
    if !(recipient.is_p2pkh()
        || recipient.is_p2sh()
        || recipient.is_p2wpkh()
        || recipient.is_p2wsh()
        || recipient.is_p2tr())
    {
        return Err(SendError::invalid("unsupported Peercoin recipient script"));
    }
    let total = validate_inputs(chain, inputs)?;
    let remaining = total
        .checked_sub(amount)
        .ok_or_else(SendError::insufficient_funds)?;
    let one_size = estimate_bytes(inputs, recipient_script, None)?;
    let one_minimum = peercoin_minimum_fee(chain, one_size)?;
    let two_size = estimate_bytes(inputs, recipient_script, Some(sender_script))?;
    let two_minimum = peercoin_minimum_fee(chain, two_size)?;
    let fee = requested_fee.unwrap_or(two_minimum);
    let with_change = remaining
        .checked_sub(fee)
        .filter(|change| *change >= minimum_output);
    let (fee, change, estimated_bytes) = match with_change {
        Some(change) => {
            if fee < two_minimum {
                return Err(SendError::invalid(
                    "Peercoin fee is below the protocol minimum",
                ));
            }
            (fee, change, two_size)
        }
        None => {
            if remaining < one_minimum {
                return Err(SendError::insufficient_funds());
            }
            if let Some(fee) = requested_fee {
                if fee > remaining {
                    return Err(SendError::insufficient_funds());
                }
                if fee < one_minimum {
                    return Err(SendError::invalid(
                        "Peercoin fee is below the protocol minimum",
                    ));
                }
            }
            (remaining, 0, one_size)
        }
    };
    let max_sendable = total
        .checked_sub(requested_fee.unwrap_or(one_minimum))
        .filter(|amount| *amount >= minimum_output)
        .unwrap_or(0);
    if estimated_bytes > 100_000 {
        return Err(SendError::invalid("Peercoin transaction is too large"));
    }
    Ok(PeercoinFeeQuote {
        fee,
        change,
        estimated_bytes,
        max_sendable,
    })
}

/// Validate the sender address against its wallet-owned key before a spend.
pub(crate) fn validate_peercoin_sender(
    chain: Chain,
    sender: &str,
    private_key: &[u8],
) -> Result<(), SendError> {
    chain.peercoin_max_money()?;
    let script = ScriptBuf::from_bytes(parse_utxo_address(chain, sender)?.script_pubkey());
    let secret = SecretKey::from_slice(private_key).map_err(SendError::invalid)?;
    let public = CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
        &Secp256k1::new(),
        &secret,
    ));
    InputKind::from_script(&script)?.validate_key(&script, &public)
}

/// Each reviewed prevout retains its own wallet-derived signing key.
pub(crate) struct PeercoinSigningInput<'a> {
    pub utxo: &'a Input,
    pub private_key: &'a [u8],
}

/// Sign Peercoin version-3 P2PK/P2PKH/native or nested P2WPKH and P2TR prevouts.
/// Signing refuses any difference from the fee the user reviewed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign_peercoin_inputs_with_output_script(
    chain: Chain,
    inputs: &[PeercoinSigningInput<'_>],
    recipient_script: &[u8],
    amount: u64,
    fee: u64,
    change_address: &str,
    change_private_key: &[u8],
) -> Result<Vec<u8>, SendError> {
    let change_script =
        ScriptBuf::from_bytes(parse_utxo_address(chain, change_address)?.script_pubkey());
    validate_peercoin_sender(chain, change_address, change_private_key)?;
    let utxos: Vec<_> = inputs.iter().map(|input| input.utxo.clone()).collect();
    let quote = quote_peercoin_fee(
        chain,
        &utxos,
        amount,
        recipient_script,
        change_script.as_bytes(),
        Some(fee),
    )?;
    if quote.fee != fee {
        return Err(SendError::invalid(
            "Peercoin fee changed; build and review again",
        ));
    }
    let secp = Secp256k1::new();
    let mut identities = Vec::with_capacity(inputs.len());
    let mut tx_inputs = Vec::with_capacity(inputs.len());
    for input in inputs {
        let (txid, vout, _, script) = input.utxo;
        let secret = SecretKey::from_slice(input.private_key).map_err(SendError::invalid)?;
        let public = CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp, &secret,
        ));
        let kind = InputKind::from_script(Script::from_bytes(script))?;
        kind.validate_key(Script::from_bytes(script), &public)?;
        identities.push((kind, secret, public));
        tx_inputs.push(TxIn {
            previous_output: OutPoint {
                txid: Txid::from_str(txid).map_err(SendError::invalid)?,
                vout: *vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        });
    }
    let mut tx = Transaction {
        version: Version(3),
        lock_time: LockTime::ZERO,
        input: tx_inputs,
        output: vec![TxOut {
            value: Amount::from_sat(amount),
            script_pubkey: ScriptBuf::from_bytes(recipient_script.to_vec()),
        }],
    };
    if quote.change > 0 {
        tx.output.push(TxOut {
            value: Amount::from_sat(quote.change),
            script_pubkey: change_script,
        });
    }
    let prevouts: Vec<_> = inputs
        .iter()
        .map(|input| TxOut {
            value: Amount::from_sat(input.utxo.2),
            script_pubkey: ScriptBuf::from_bytes(input.utxo.3.clone()),
        })
        .collect();
    let mut cache = SighashCache::new(&tx);
    let mut signatures = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let (kind, secret, public) = &identities[i];
        if *kind == InputKind::P2tr {
            let digest = cache
                .taproot_key_spend_signature_hash(
                    i,
                    &Prevouts::All(&prevouts),
                    TapSighashType::Default,
                )
                .map_err(SendError::invalid)?
                .to_byte_array();
            let tweaked =
                bitcoin::key::Keypair::from_secret_key(&secp, secret).tap_tweak(&secp, None);
            signatures.push(
                secp.sign_schnorr(&Message::from_digest(digest), &tweaked.to_keypair())
                    .serialize()
                    .to_vec(),
            );
            continue;
        }
        let digest = if kind.has_witness() {
            // BIP143 commits to the actual six-decimal atomic prevout value.
            let program = ScriptBuf::new_p2wpkh(&public.wpubkey_hash());
            cache
                .p2wpkh_signature_hash(
                    i,
                    &program,
                    Amount::from_sat(input.utxo.2),
                    EcdsaSighashType::All,
                )
                .map_err(SendError::invalid)?
                .to_byte_array()
        } else {
            cache
                .legacy_signature_hash(
                    i,
                    Script::from_bytes(&input.utxo.3),
                    EcdsaSighashType::All as u32,
                )
                .map_err(SendError::invalid)?
                .to_byte_array()
        };
        let mut signature = secp
            .sign_ecdsa(&Message::from_digest(digest), secret)
            .serialize_der()
            .to_vec();
        signature.push(EcdsaSighashType::All as u8);
        signatures.push(signature);
    }
    for (i, signature) in signatures.iter().enumerate() {
        let (kind, _, public) = &identities[i];
        match kind {
            InputKind::P2pk => {
                let push = PushBytesBuf::try_from(signature.clone()).map_err(SendError::invalid)?;
                tx.input[i].script_sig = Builder::new().push_slice(push).into_script();
            }
            InputKind::P2pkh => {
                tx.input[i].script_sig = ScriptBuf::from_bytes(
                    super::bitcoin_wire::p2pkh_script_sig(signature, &public.to_bytes()),
                );
            }
            InputKind::P2wpkh | InputKind::NestedP2wpkh => {
                if *kind == InputKind::NestedP2wpkh {
                    let program = ScriptBuf::new_p2wpkh(&public.wpubkey_hash());
                    let push = PushBytesBuf::try_from(program.as_bytes().to_vec())
                        .map_err(SendError::invalid)?;
                    tx.input[i].script_sig = Builder::new().push_slice(push).into_script();
                }
                tx.input[i].witness =
                    Witness::from_slice(&[signature.as_slice(), &public.to_bytes()]);
            }
            InputKind::P2tr => tx.input[i].witness = Witness::from_slice(&[signature.as_slice()]),
        }
    }
    let raw = bitcoin::consensus::serialize(&tx);
    if fee < peercoin_minimum_fee(chain, raw.len() as u64)? {
        return Err(SendError::invalid(
            "Peercoin signed transaction fee is below the protocol minimum",
        ));
    }
    Ok(raw)
}

/// Single-key wrapper for an imported key or a standalone protocol check.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign_peercoin_tx(
    chain: Chain,
    inputs: &[Input],
    to_address: &str,
    amount: u64,
    fee: u64,
    sender: &str,
    private_key: &[u8],
) -> Result<Vec<u8>, SendError> {
    let recipient = parse_utxo_address(chain, to_address)?.script_pubkey();
    let signing: Vec<_> = inputs
        .iter()
        .map(|utxo| PeercoinSigningInput { utxo, private_key })
        .collect();
    sign_peercoin_inputs_with_output_script(
        chain,
        &signing,
        &recipient,
        amount,
        fee,
        sender,
        private_key,
    )
}

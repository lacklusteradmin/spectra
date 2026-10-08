//! Litecoin transparent transactions: P2PKH and native/nested P2WPKH inputs.

use std::collections::HashSet;
use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::script::{Builder, PushBytesBuf};
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::transaction::Version;
use bitcoin::{
    Amount, CompressedPublicKey, OutPoint, Script, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};

use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;
use crate::send::error::SendError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    P2pkh,
    P2wpkh,
    NestedP2wpkh,
}

impl InputKind {
    fn from_script(script: &Script) -> Result<Self, SendError> {
        if script.is_p2pkh() {
            Ok(Self::P2pkh)
        } else if script.is_p2wpkh() {
            Ok(Self::P2wpkh)
        } else if script.is_p2sh() {
            Ok(Self::NestedP2wpkh)
        } else {
            Err(SendError::Invalid(
                "Litecoin sender must be P2PKH, P2WPKH or P2SH-P2WPKH".into(),
            ))
        }
    }

    fn script(self, key: &CompressedPublicKey) -> ScriptBuf {
        match self {
            Self::P2pkh => ScriptBuf::new_p2pkh(&key.pubkey_hash()),
            Self::P2wpkh => ScriptBuf::new_p2wpkh(&key.wpubkey_hash()),
            Self::NestedP2wpkh => ScriptBuf::new_p2wpkh(&key.wpubkey_hash()).to_p2sh(),
        }
    }

    fn validate_key(self, script: &Script, key: &CompressedPublicKey) -> Result<(), SendError> {
        if script != self.script(key).as_script() {
            return Err(SendError::Invalid(
                "Litecoin source script does not match the supplied private key".into(),
            ));
        }
        Ok(())
    }
}

fn sender_script(chain: Chain, sender: &str) -> Result<(InputKind, ScriptBuf), SendError> {
    if !matches!(chain, Chain::Litecoin | Chain::LitecoinTestnet) {
        return Err(SendError::Invalid("expected a Litecoin network".into()));
    }
    let script = ScriptBuf::from_bytes(parse_utxo_address(chain, sender)?.script_pubkey());
    Ok((InputKind::from_script(&script)?, script))
}

/// Validate the supported sender script and its ownership before network access.
pub(crate) fn validate_ltc_sender(
    chain: Chain,
    sender: &str,
    private_key_bytes: &[u8],
) -> Result<(), SendError> {
    let (kind, script) = sender_script(chain, sender)?;
    let secret = SecretKey::from_slice(private_key_bytes).map_err(SendError::invalid)?;
    let key = CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
        &Secp256k1::new(),
        &secret,
    ));
    kind.validate_key(&script, &key)
}

/// Validate provider amounts against Litecoin Core's consensus MoneyRange
/// before preparing, quoting or signing, and return the checked input total.
pub(crate) fn validate_ltc_values(
    chain: Chain,
    values: impl IntoIterator<Item = u64>,
) -> Result<u64, SendError> {
    let maximum = chain.litecoin_max_money()?;
    values.into_iter().try_fold(0u64, |total, value| {
        if value == 0 {
            return Err(SendError::Invalid(
                "Litecoin input value must be positive".into(),
            ));
        }
        total
            .checked_add(value)
            .filter(|total| *total <= maximum)
            .ok_or_else(|| SendError::Invalid("Litecoin input total exceeds MAX_MONEY".into()))
    })
}

/// Litecoin Core's GetDustThreshold policy uses its dust relay rate and an
/// estimated cost to spend the output: 148 bytes for legacy, 67 for a
/// witness program. Only the standard outputs a wallet pays are priced: an
/// MWEB peg-in's version 9 program among them, any other script refused.
/// https://github.com/litecoin-project/litecoin/blob/master/src/policy/policy.cpp
pub(crate) fn litecoin_dust_threshold(chain: Chain, script: &[u8]) -> Result<u64, SendError> {
    let relay_fee = chain.litecoin_dust_relay_fee_per_kvb()?;
    let pegin = script.len() == 34 && script[..2] == [0x59, 0x20];
    let script = Script::from_bytes(script);
    let spend_size = if script.is_p2pkh() || script.is_p2sh() {
        148
    } else if script.is_p2wpkh() || script.is_p2wsh() || script.is_p2tr() || pegin {
        67
    } else {
        return Err(SendError::Invalid(
            "unsupported Litecoin output script".into(),
        ));
    };
    // Supported standard scripts are shorter than CompactSize's 253-byte boundary.
    let output_size = 8 + 1 + script.len() as u64;
    (output_size + spend_size)
        .checked_mul(relay_fee)
        .map(|fee| fee / 1_000)
        .ok_or_else(|| SendError::Invalid("Litecoin dust threshold overflow".into()))
}

/// Upper bound for a signed transparent transaction's virtual size, including
/// mixed legacy/witness inputs and CompactSize count boundaries. Litecoin Core
/// uses BIP141 weight and BIP143 signatures for transparent witness-v0 inputs.
/// A low-S DER signature plus its SIGHASH byte uses at most 72 bytes.
pub(crate) fn estimate_ltc_vsize<'a>(
    input_scripts: impl IntoIterator<Item = &'a [u8]>,
    to_script_len: usize,
    change_script: Option<&[u8]>,
) -> Result<u64, SendError> {
    let compact_size = |value: u64| -> u64 {
        match value {
            0..=0xfc => 1,
            0xfd..=0xffff => 3,
            0x1_0000..=0xffff_ffff => 5,
            _ => 9,
        }
    };
    let overflow = || SendError::Invalid("Litecoin transaction size overflow".into());
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
        };
        has_witness |= kind != InputKind::P2pkh;
        input_count = input_count.checked_add(1).ok_or_else(overflow)?;
        inputs_size = inputs_size.checked_add(base).ok_or_else(overflow)?;
        witness_size = witness_size.checked_add(witness).ok_or_else(overflow)?;
    }
    if input_count == 0 {
        return Err(SendError::Invalid("transaction must have inputs".into()));
    }
    let output_size = |script_len: usize| -> Result<u64, SendError> {
        let len = u64::try_from(script_len).map_err(SendError::invalid)?;
        8u64.checked_add(compact_size(len))
            .and_then(|size| size.checked_add(len))
            .ok_or_else(overflow)
    };
    let primary_size = output_size(to_script_len)?;
    let change_size = change_script.map_or(Ok(0), |script| output_size(script.len()))?;
    let base_size = 8u64
        .checked_add(compact_size(input_count))
        .and_then(|size| size.checked_add(1)) // one or two outputs
        .and_then(|size| size.checked_add(inputs_size))
        .and_then(|size| size.checked_add(primary_size))
        .and_then(|size| size.checked_add(change_size))
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

/// One discovered or imported input paired with its own signing key.
pub(crate) struct LtcSigningInput<'a> {
    pub utxo: &'a (String, u32, u64, Vec<u8>),
    pub private_key: &'a [u8],
}

/// Sign Litecoin key-owned transparent inputs with a validated change key.
///
/// `to_script` is the complete primary transparent recipient output script.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign_ltc_inputs_with_output_script(
    chain: Chain,
    inputs: &[LtcSigningInput<'_>],
    to_script: &[u8],
    amount_sat: u64,
    fee_sat: u64,
    change_address: &str,
    change_private_key: &[u8],
) -> Result<Vec<u8>, SendError> {
    let recipient_dust = litecoin_dust_threshold(chain, to_script)?;
    if amount_sat < recipient_dust {
        return Err(SendError::Invalid(
            "Litecoin recipient amount is below the dust threshold".into(),
        ));
    }
    let total = validate_ltc_values(chain, inputs.iter().map(|input| input.utxo.2))?;
    let change = super::accounting::checked_change([total], amount_sat, fee_sat)?;
    let (change_kind, change_script) = sender_script(chain, change_address)?;
    let change_dust = litecoin_dust_threshold(chain, change_script.as_bytes())?;
    let secp = Secp256k1::new();
    let change_secret = SecretKey::from_slice(change_private_key).map_err(SendError::invalid)?;
    let change_key = CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp,
        &change_secret,
    ));
    change_kind.validate_key(&change_script, &change_key)?;

    let mut seen = HashSet::new();
    let mut identities = Vec::with_capacity(inputs.len());
    let mut tx_inputs = Vec::with_capacity(inputs.len());
    for input in inputs {
        let (txid, vout, _, script) = input.utxo;
        let script = Script::from_bytes(script);
        let kind = InputKind::from_script(script)?;
        let secret = SecretKey::from_slice(input.private_key).map_err(SendError::invalid)?;
        let key = CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp, &secret,
        ));
        kind.validate_key(script, &key)?;
        let outpoint = OutPoint {
            txid: Txid::from_str(txid).map_err(SendError::invalid)?,
            vout: *vout,
        };
        if outpoint.is_null() {
            return Err(SendError::Invalid("invalid Litecoin input outpoint".into()));
        }
        if !seen.insert(outpoint) {
            return Err(SendError::Invalid("duplicate Litecoin input".into()));
        }
        identities.push((kind, secret, key));
        tx_inputs.push(TxIn {
            previous_output: outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        });
    }

    let mut outputs = vec![TxOut {
        value: Amount::from_sat(amount_sat),
        script_pubkey: ScriptBuf::from_bytes(to_script.to_vec()),
    }];
    if change > 0 && change >= change_dust {
        outputs.push(TxOut {
            value: Amount::from_sat(change),
            script_pubkey: change_script,
        });
    }
    let mut tx = Transaction {
        version: Version::ONE,
        lock_time: LockTime::ZERO,
        input: tx_inputs,
        output: outputs,
    };

    // Litecoin Core's SignatureHash uses the standard legacy and BIP143
    // encodings. See litecoin-project/litecoin src/script/interpreter.cpp.
    let mut cache = SighashCache::new(&mut tx);
    let mut signatures = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let (kind, secret, key) = &identities[index];
        let digest = match kind {
            InputKind::P2pkh => cache
                .legacy_signature_hash(
                    index,
                    Script::from_bytes(&input.utxo.3),
                    EcdsaSighashType::All as u32,
                )
                .map_err(SendError::invalid)?
                .to_byte_array(),
            InputKind::P2wpkh | InputKind::NestedP2wpkh => cache
                .p2wpkh_signature_hash(
                    index,
                    &InputKind::P2wpkh.script(key),
                    Amount::from_sat(input.utxo.2),
                    EcdsaSighashType::All,
                )
                .map_err(SendError::invalid)?
                .to_byte_array(),
        };
        let mut signature = secp
            .sign_ecdsa(&Message::from_digest(digest), secret)
            .serialize_der()
            .to_vec();
        signature.push(EcdsaSighashType::All as u8);
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
        }
    }
    Ok(bitcoin::consensus::serialize(tx))
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign_ltc_with_output_script(
    chain: Chain,
    utxos: &[(String, u32, u64, Vec<u8>)],
    to_script: &[u8],
    amount_sat: u64,
    fee_sat: u64,
    change_address: &str,
    private_key_bytes: &[u8],
) -> Result<Vec<u8>, SendError> {
    let inputs: Vec<_> = utxos
        .iter()
        .map(|utxo| LtcSigningInput {
            utxo,
            private_key: private_key_bytes,
        })
        .collect();
    sign_ltc_inputs_with_output_script(
        chain,
        &inputs,
        to_script,
        amount_sat,
        fee_sat,
        change_address,
        private_key_bytes,
    )
}

#[cfg(test)]
#[path = "tests/litecoin.rs"]
mod tests;

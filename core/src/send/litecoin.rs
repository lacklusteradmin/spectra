//! Litecoin transparent transactions: P2PKH and native/nested P2WPKH inputs.

use bitcoin::script::Script;
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::transaction::Version;
use bitcoin::{CompressedPublicKey, ScriptBuf, Sequence};

pub(crate) use super::bitcoin::InputKind;
use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;
use crate::send::error::SendError;

/// A Litecoin input's kind: a key's P2PKH or P2WPKH, nested or native.
/// Litecoin wallets hold no Taproot key.
fn litecoin_kind(script: &Script) -> Result<InputKind, SendError> {
    match InputKind::from_script(script)? {
        InputKind::P2tr => Err(SendError::Invalid(
            "Litecoin sender must be P2PKH, P2WPKH or P2SH-P2WPKH".into(),
        )),
        kind => Ok(kind),
    }
}

fn sender_script(chain: Chain, sender: &str) -> Result<(InputKind, ScriptBuf), SendError> {
    if !matches!(chain, Chain::Litecoin | Chain::LitecoinTestnet) {
        return Err(SendError::Invalid("expected a Litecoin network".into()));
    }
    let script = ScriptBuf::from_bytes(parse_utxo_address(chain, sender)?.script_pubkey());
    Ok((litecoin_kind(&script)?, script))
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
    let input_scripts: Vec<&[u8]> = input_scripts.into_iter().collect();
    for script in &input_scripts {
        litecoin_kind(Script::from_bytes(script))?;
    }
    super::bitcoin::estimate_vsize(
        input_scripts,
        std::iter::once(to_script_len).chain(change_script.map(<[u8]>::len)),
    )
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

    for input in inputs {
        litecoin_kind(Script::from_bytes(&input.utxo.3))?;
    }
    let mut outputs = vec![(to_script.to_vec(), amount_sat)];
    if change > 0 && change >= change_dust {
        outputs.push((change_script.into_bytes(), change));
    }
    // Litecoin Core's SignatureHash uses the standard legacy and BIP143
    // encodings. See litecoin-project/litecoin src/script/interpreter.cpp.
    super::bitcoin::sign_inputs(
        &inputs
            .iter()
            .map(|input| super::bitcoin::SigningInput {
                utxo: input.utxo,
                private_key: input.private_key,
            })
            .collect::<Vec<_>>(),
        &outputs,
        Version::ONE,
        Sequence::MAX,
    )
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

//! Cardano send: ADA and native assets from the wallet's address, as one
//! transaction model. Coin selection spends any unspent output, assets
//! included; the change returns every asset the inputs carried that the
//! transfer does not send; every output holds the protocol's minimum ADA for
//! its size; the fee is the protocol's linear fee for the signed size. A
//! minimal CBOR encoder and the extended-key witness.

use crate::api::cardano_asset::CardanoAssetId;
use crate::api::koios::CardanoProtocolParams;
use crate::send::error::SendError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Some quantity of one native asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CardanoAssetAmount {
    /// `policy.name`, as `CardanoAssetId::identifier` writes it.
    pub asset: String,
    pub quantity: u64,
}

/// An unspent output the transaction spends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CardanoInput {
    pub tx_hash: String,
    pub tx_index: u32,
    pub lovelace: u64,
    pub assets: Vec<CardanoAssetAmount>,
}

/// An output the transaction creates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CardanoOutput {
    /// Bech32.
    pub address: String,
    pub lovelace: u64,
    pub assets: Vec<CardanoAssetAmount>,
}

/// A transfer as reviewed: its inputs, the recipient's output and then the
/// change, if any, its fee and TTL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedCardanoTransaction {
    pub inputs: Vec<CardanoInput>,
    pub outputs: Vec<CardanoOutput>,
    pub fee: u64,
    pub ttl: u64,
}

/// What a transfer delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CardanoTransfer {
    Ada(u64),
    Asset {
        asset: CardanoAssetId,
        quantity: u64,
    },
}

fn refused(message: &'static str) -> SendError {
    SendError::Invalid(message.into())
}

/// Every asset in `amounts`, summed by asset, in a stable order.
fn tally(
    amounts: impl IntoIterator<Item = CardanoAssetAmount>,
) -> Result<BTreeMap<String, u64>, SendError> {
    let mut sums = BTreeMap::new();
    for amount in amounts {
        let sum: &mut u64 = sums.entry(amount.asset).or_default();
        *sum = sum
            .checked_add(amount.quantity)
            .ok_or_else(|| refused("Cardano native token quantity overflow"))?;
    }
    Ok(sums)
}

fn amounts(sums: &BTreeMap<String, u64>) -> Vec<CardanoAssetAmount> {
    sums.iter()
        .filter(|(_, quantity)| **quantity != 0)
        .map(|(asset, quantity)| CardanoAssetAmount {
            asset: asset.clone(),
            quantity: *quantity,
        })
        .collect()
}

/// An output's value: the coin alone, or `[coin, multiasset]` with policies
/// in byte order and each policy's names shortest first, as canonical CBOR
/// and the Cardano Serialization Library order them.
fn encode_value(lovelace: u64, assets: &[CardanoAssetAmount]) -> Result<Vec<u8>, SendError> {
    if assets.is_empty() {
        return Ok(cbor_uint(lovelace));
    }
    let mut policies: BTreeMap<[u8; 28], BTreeMap<(usize, Vec<u8>), u64>> = BTreeMap::new();
    for amount in assets {
        let id = CardanoAssetId::parse(&amount.asset)?;
        let names = policies.entry(id.policy).or_default();
        if names
            .insert((id.name.len(), id.name), amount.quantity)
            .is_some()
        {
            return Err(refused("A Cardano output names an asset twice"));
        }
    }
    let encoded = policies
        .into_iter()
        .map(|(policy, names)| {
            let names = names
                .into_iter()
                .map(|((_, name), quantity)| (cbor_bytes(&name), cbor_uint(quantity)))
                .collect::<Vec<_>>();
            (cbor_bytes(&policy), cbor_map(&names))
        })
        .collect::<Vec<_>>();
    Ok(cbor_array(&[cbor_uint(lovelace), cbor_map(&encoded)]))
}

/// `[address, value]`: the output form without a datum or script.
fn encode_output(output: &CardanoOutput) -> Result<Vec<u8>, SendError> {
    let address = crate::derivation::cardano::decode_cardano_addr_bytes(&output.address)?;
    Ok(cbor_array(&[
        cbor_bytes(&address),
        encode_value(output.lovelace, &output.assets)?,
    ]))
}

/// The ADA an output must hold: 160 bytes of overhead and the output's own
/// size, at the protocol's price per byte, for the output as it stands.
fn required_ada(output: &CardanoOutput, params: &CardanoProtocolParams) -> Result<u64, SendError> {
    let size = encode_output(output)?.len() as u64;
    (160 + size)
        .checked_mul(params.coins_per_utxo_byte)
        .ok_or_else(|| refused("Cardano minimum ADA overflow"))
}

/// The least ADA `address` can receive `assets` with: the requirement of an
/// output holding exactly that much, found as the Cardano Serialization
/// Library's `min_ada_for_output` finds it.
pub(crate) fn minimum_ada(
    address: &str,
    assets: &[CardanoAssetAmount],
    params: &CardanoProtocolParams,
) -> Result<u64, SendError> {
    let mut output = CardanoOutput {
        address: address.to_string(),
        lovelace: 0,
        assets: assets.to_vec(),
    };
    for _ in 0..3 {
        let required = required_ada(&output, params)?;
        if output.lovelace >= required {
            return Ok(required);
        }
        output.lovelace = required;
    }
    output.lovelace = u64::MAX;
    required_ada(&output, params)
}

impl PreparedCardanoTransaction {
    /// Choose inputs from `utxos` for `transfer` to `recipient`, returning
    /// the change to `sender`, under `params`.
    ///
    /// Inputs that hold the sent asset come first, largest first; then
    /// outputs of ADA alone, then outputs holding other assets, each largest
    /// first, until the outputs, their minimum ADA and the fee are covered.
    /// Change too small to stand as an output joins the fee.
    pub(crate) fn plan(
        utxos: &[CardanoInput],
        params: &CardanoProtocolParams,
        sender: &str,
        recipient: &str,
        transfer: &CardanoTransfer,
        ttl: u64,
    ) -> Result<Self, SendError> {
        let recipient_output = match transfer {
            CardanoTransfer::Ada(lovelace) => {
                let output = CardanoOutput {
                    address: recipient.to_string(),
                    lovelace: *lovelace,
                    assets: Vec::new(),
                };
                let minimum = required_ada(&output, params)?;
                if *lovelace < minimum {
                    return Err(SendError::Invalid(crate::LocalizableMessage::new(
                        "A Cardano output holds at least %@ ADA",
                        [crate::decimal::from_units(u128::from(minimum), 6)],
                    )));
                }
                output
            }
            CardanoTransfer::Asset { asset, quantity } => {
                if *quantity == 0 {
                    return Err(refused("Amount must be greater than zero"));
                }
                let assets = vec![CardanoAssetAmount {
                    asset: asset.identifier(),
                    quantity: *quantity,
                }];
                CardanoOutput {
                    address: recipient.to_string(),
                    lovelace: minimum_ada(recipient, &assets, params)?,
                    assets,
                }
            }
        };
        let holds = |input: &CardanoInput, asset: &str| {
            input
                .assets
                .iter()
                .filter(|amount| amount.asset == asset)
                .map(|amount| amount.quantity)
                .sum::<u64>()
        };
        let sent_asset = match transfer {
            CardanoTransfer::Asset { asset, .. } => Some(asset.identifier()),
            CardanoTransfer::Ada(_) => None,
        };
        let mut candidates: Vec<&CardanoInput> = utxos.iter().collect();
        candidates.sort_by_key(|input| {
            let tier = match &sent_asset {
                Some(asset) if holds(input, asset) > 0 => 0,
                _ if input.assets.is_empty() => 1,
                _ => 2,
            };
            let size = match &sent_asset {
                Some(asset) if tier == 0 => holds(input, asset),
                _ => input.lovelace,
            };
            (
                tier,
                std::cmp::Reverse(size),
                input.tx_hash.clone(),
                input.tx_index,
            )
        });
        let mut selected: Vec<CardanoInput> = Vec::new();
        let mut last_shortfall = None;
        for candidate in candidates {
            selected.push(candidate.clone());
            match Self::balance(&selected, &recipient_output, sender, params, ttl)? {
                Ok(transaction) => return transaction.within_limits(params),
                Err(shortfall) => last_shortfall = Some(shortfall),
            }
        }
        Err(match last_shortfall {
            Some(Shortfall::Asset) | None if sent_asset.is_some() => {
                SendError::InsufficientFunds("Insufficient token balance".into())
            }
            _ => SendError::insufficient_funds(),
        })
    }

    /// The transaction these inputs make, or what they lack.
    fn balance(
        inputs: &[CardanoInput],
        recipient: &CardanoOutput,
        sender: &str,
        params: &CardanoProtocolParams,
        ttl: u64,
    ) -> Result<Result<Self, Shortfall>, SendError> {
        let held = tally(inputs.iter().flat_map(|input| input.assets.clone()))?;
        let mut change_assets = held.clone();
        for amount in &recipient.assets {
            let left = change_assets.entry(amount.asset.clone()).or_default();
            match left.checked_sub(amount.quantity) {
                Some(rest) => *left = rest,
                None => return Ok(Err(Shortfall::Asset)),
            }
        }
        let change_assets = amounts(&change_assets);
        let lovelace = inputs
            .iter()
            .try_fold(0u64, |sum, input| sum.checked_add(input.lovelace))
            .ok_or_else(|| refused("Cardano input value overflow"))?;
        let Some(available) = lovelace.checked_sub(recipient.lovelace) else {
            return Ok(Err(Shortfall::Ada));
        };
        let mut fee = 0u64;
        for _ in 0..16 {
            let Some(change) = available.checked_sub(fee) else {
                return Ok(Err(Shortfall::Ada));
            };
            let change_output = CardanoOutput {
                address: sender.to_string(),
                lovelace: change,
                assets: change_assets.clone(),
            };
            let stands = change > 0 && change >= required_ada(&change_output, params)?;
            let transaction = if change_assets.is_empty() && !stands {
                // Too little to stand as an output: it joins the fee.
                Self {
                    inputs: inputs.to_vec(),
                    outputs: vec![recipient.clone()],
                    fee: available,
                    ttl,
                }
            } else if stands {
                Self {
                    inputs: inputs.to_vec(),
                    outputs: vec![recipient.clone(), change_output],
                    fee,
                    ttl,
                }
            } else {
                // The change holds assets but not the ADA to carry them.
                return Ok(Err(Shortfall::Ada));
            };
            let required = transaction.minimum_fee(params)?;
            if transaction.fee >= required {
                return Ok(Ok(transaction));
            }
            if transaction.outputs.len() == 1 {
                return Ok(Err(Shortfall::Ada));
            }
            fee = required;
        }
        Err(refused("Cardano fee did not settle"))
    }

    /// The protocol's fee for this transaction signed by one key.
    pub(crate) fn minimum_fee(&self, params: &CardanoProtocolParams) -> Result<u64, SendError> {
        let size = self.encode_signed(&[0; 32], &[0; 64])?.len() as u64;
        params
            .fee_per_byte
            .checked_mul(size)
            .and_then(|fee| fee.checked_add(params.fee_fixed))
            .ok_or_else(|| refused("Cardano fee overflow"))
    }

    /// Refuse what the ledger would: an output short of its minimum ADA or
    /// whose value is too large, a transaction past the size limit, or
    /// value and assets that do not balance.
    pub(crate) fn within_limits(self, params: &CardanoProtocolParams) -> Result<Self, SendError> {
        for output in &self.outputs {
            if output.lovelace < required_ada(output, params)? {
                return Err(refused("A Cardano output holds less than its minimum ADA"));
            }
            if encode_value(output.lovelace, &output.assets)?.len() as u64 > params.max_value_size {
                return Err(refused(
                    "A Cardano output would hold more assets than one output can",
                ));
            }
        }
        if self.encode_signed(&[0; 32], &[0; 64])?.len() as u64 > params.max_tx_size {
            return Err(refused(
                "The Cardano transaction would be too large; send from fewer outputs",
            ));
        }
        if self.fee < self.minimum_fee(params)? {
            return Err(refused("The Cardano fee is below the protocol's minimum"));
        }
        self.conserves()?;
        Ok(self)
    }

    /// Inputs equal outputs and the fee, in ADA and in every asset.
    pub(crate) fn conserves(&self) -> Result<(), SendError> {
        let lovelace_in = self
            .inputs
            .iter()
            .try_fold(0u64, |sum, input| sum.checked_add(input.lovelace));
        let lovelace_out = self
            .outputs
            .iter()
            .try_fold(self.fee, |sum, output| sum.checked_add(output.lovelace));
        let assets_in = tally(self.inputs.iter().flat_map(|input| input.assets.clone()))?;
        let assets_out = tally(self.outputs.iter().flat_map(|output| output.assets.clone()))?;
        if lovelace_in.is_none()
            || lovelace_in != lovelace_out
            || amounts(&assets_in) != amounts(&assets_out)
        {
            return Err(refused(
                "The Cardano transaction does not return every input's value",
            ));
        }
        Ok(())
    }

    fn encode_body(&self) -> Result<Vec<u8>, SendError> {
        let inputs = self
            .inputs
            .iter()
            .map(|input| {
                let hash = hex::decode(&input.tx_hash)
                    .map_err(|e| SendError::Invalid(format!("input txid: {e}").into()))?;
                if hash.len() != 32 {
                    return Err(SendError::Invalid(
                        "input txid must contain exactly 32 bytes".into(),
                    ));
                }
                Ok(cbor_array(&[
                    cbor_bytes(&hash),
                    cbor_uint(u64::from(input.tx_index)),
                ]))
            })
            .collect::<Result<Vec<_>, SendError>>()?;
        let outputs = self
            .outputs
            .iter()
            .map(encode_output)
            .collect::<Result<Vec<_>, SendError>>()?;
        // {0: inputs, 1: outputs, 2: fee, 3: ttl}
        Ok(cbor_map(&[
            (cbor_uint(0), cbor_tagged_set(&inputs)),
            (cbor_uint(1), cbor_array(&outputs)),
            (cbor_uint(2), cbor_uint(self.fee)),
            (cbor_uint(3), cbor_uint(self.ttl)),
        ]))
    }

    /// `[body, {0: [[vkey, signature]]}, true, null]`.
    fn encode_signed(&self, public: &[u8; 32], signature: &[u8; 64]) -> Result<Vec<u8>, SendError> {
        Ok(cbor_array(&[
            self.encode_body()?,
            encode_witness_set(public, signature),
            cbor_bool(true),
            cbor_null(),
        ]))
    }

    /// The transaction's id: the hash of its body.
    pub(crate) fn transaction_hash(&self) -> Result<[u8; 32], SendError> {
        Ok(blake2b_256(&self.encode_body()?))
    }

    /// The signed transaction, as CBOR hex.
    pub(crate) fn sign(
        &self,
        signing_key: &[u8; 64],
        verification_key: &[u8; 32],
    ) -> Result<String, SendError> {
        self.conserves()?;
        let signature = sign_extended(signing_key, verification_key, &self.transaction_hash()?)?;
        Ok(hex::encode(
            self.encode_signed(verification_key, &signature)?,
        ))
    }

    /// What a native-asset transfer carries beyond the asset: the minimum
    /// ADA its output holds, which the recipient keeps.
    pub(crate) fn terms(&self, decimals: u32) -> Option<crate::send::stages::AssetTransferTerms> {
        let recipient = self.outputs.first()?;
        let sent = recipient.assets.first()?;
        let amount = crate::decimal::from_units(u128::from(sent.quantity), decimals);
        Some(crate::send::stages::AssetTransferTerms {
            debited: amount.clone(),
            received: amount,
            fee: "0".into(),
            hook_program: None,
            carried_native: Some(crate::decimal::from_units(
                u128::from(recipient.lovelace),
                6,
            )),
            recipient_registration: None,
        })
    }
}

/// The fee and the most ADA a send from `utxos` can deliver: everything but
/// the fee and, where the outputs hold native assets, the minimum ADA their
/// change keeps. The fee is priced for a base-address recipient, the longest
/// a Shelley recipient is, so the build never needs more.
pub(crate) fn ada_preview(
    utxos: &[CardanoInput],
    params: &CardanoProtocolParams,
    sender: &str,
) -> Result<(u64, u64), SendError> {
    let total = utxos
        .iter()
        .try_fold(0u64, |sum, input| sum.checked_add(input.lovelace))
        .ok_or_else(|| refused("Cardano input value overflow"))?;
    let held = amounts(&tally(utxos.iter().flat_map(|input| input.assets.clone()))?);
    let recipient = longest_recipient(sender)?;
    let keep = if held.is_empty() {
        0
    } else {
        minimum_ada(sender, &held, params)?
    };
    let mut fee = 0u64;
    for _ in 0..16 {
        let max = total.saturating_sub(keep).saturating_sub(fee);
        let mut outputs = vec![CardanoOutput {
            address: recipient.clone(),
            lovelace: max,
            assets: Vec::new(),
        }];
        if !held.is_empty() {
            outputs.push(CardanoOutput {
                address: sender.to_string(),
                lovelace: keep,
                assets: held.clone(),
            });
        }
        let required = PreparedCardanoTransaction {
            inputs: utxos.to_vec(),
            outputs,
            fee,
            ttl: u64::from(u32::MAX),
        }
        .minimum_fee(params)?;
        if required <= fee {
            return Ok((fee, max));
        }
        fee = required;
    }
    Err(refused("Cardano fee did not settle"))
}

/// A base address on `sender`'s network: the longest a Shelley recipient's
/// is, so a quote for it never falls short.
fn longest_recipient(sender: &str) -> Result<String, SendError> {
    let hrp = if sender.starts_with("addr_test") {
        "addr_test"
    } else {
        "addr"
    };
    bech32::encode::<bech32::Bech32>(
        bech32::Hrp::parse(hrp).expect("valid hrp"),
        &[[0x01].as_slice(), &[0xff; 56]].concat(),
    )
    .map_err(|_| refused("Cardano address encoding"))
}

/// The most ADA a send of `asset` carries to its recipient: the minimum for
/// the largest quantity at the longest address.
pub(crate) fn token_send_ada(
    sender: &str,
    asset: &str,
    params: &CardanoProtocolParams,
) -> Result<u64, SendError> {
    minimum_ada(
        &longest_recipient(sender)?,
        &[CardanoAssetAmount {
            asset: asset.to_string(),
            quantity: u64::MAX,
        }],
        params,
    )
}

/// What a selection of inputs lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortfall {
    Asset,
    Ada,
}

/// Cardano's extended Ed25519 key is kL || kR, not a seed. kL is the
/// derived scalar and kR is the nonce prefix; hashing kL as a seed changes
/// the public key and produces an invalid payment witness.
pub(crate) fn sign_extended(
    key: &[u8; 64],
    public: &[u8; 32],
    message: &[u8],
) -> Result<[u8; 64], SendError> {
    use curve25519_dalek::{constants::ED25519_BASEPOINT_POINT, scalar::Scalar};
    use sha2::{Digest, Sha512};
    use zeroize::Zeroizing;
    let scalar_bytes =
        Zeroizing::new(<[u8; 32]>::try_from(&key[..32]).expect("fixed extended key"));
    if crate::derivation::cardano::public_from_extended_key(key)? != *public {
        return Err(SendError::invalid(
            "Cardano extended key does not match the witness public key",
        ));
    }
    let scalar = Zeroizing::new(Scalar::from_bytes_mod_order(*scalar_bytes));
    let mut nonce_hash = Sha512::new();
    nonce_hash.update(&key[32..]);
    nonce_hash.update(message);
    let nonce_digest = Zeroizing::new(<[u8; 64]>::from(nonce_hash.finalize()));
    let nonce = Zeroizing::new(Scalar::from_bytes_mod_order_wide(&nonce_digest));
    let r = (*nonce * ED25519_BASEPOINT_POINT).compress().to_bytes();
    let mut challenge = Sha512::new();
    challenge.update(r);
    challenge.update(public);
    challenge.update(message);
    let h = Scalar::from_bytes_mod_order_wide(&challenge.finalize().into());
    let s = Zeroizing::new(*nonce + h * *scalar);
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(&r);
    signature[32..].copy_from_slice(&s.to_bytes());
    Ok(signature)
}

fn encode_witness_set(vkey: &[u8], sig: &[u8]) -> Vec<u8> {
    // {0: [[vkey_bytes, sig_bytes]]}
    let vkey_sig = cbor_array(&[cbor_bytes(vkey), cbor_bytes(sig)]);
    cbor_map(&[(cbor_uint(0), cbor_tagged_set(&[vkey_sig]))])
}

// ── Minimal CBOR encoder

fn cbor_uint(n: u64) -> Vec<u8> {
    if n <= 23 {
        vec![n as u8]
    } else if n <= 0xff {
        vec![0x18, n as u8]
    } else if n <= 0xffff {
        let mut v = vec![0x19];
        v.extend_from_slice(&(n as u16).to_be_bytes());
        v
    } else if n <= 0xffff_ffff {
        let mut v = vec![0x1a];
        v.extend_from_slice(&(n as u32).to_be_bytes());
        v
    } else {
        let mut v = vec![0x1b];
        v.extend_from_slice(&n.to_be_bytes());
        v
    }
}

fn cbor_bytes(data: &[u8]) -> Vec<u8> {
    let mut out = cbor_len_prefix(2, data.len());
    out.extend_from_slice(data);
    out
}

fn cbor_array(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = cbor_len_prefix(4, items.len());
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn cbor_tagged_set(items: &[Vec<u8>]) -> Vec<u8> {
    // Tag 258 = finite set
    let mut out = vec![0xd9, 0x01, 0x02];
    out.extend_from_slice(&cbor_array(items));
    out
}

fn cbor_map(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = cbor_len_prefix(5, entries.len());
    for (k, v) in entries {
        out.extend_from_slice(k);
        out.extend_from_slice(v);
    }
    out
}

fn cbor_bool(b: bool) -> Vec<u8> {
    vec![if b { 0xf5 } else { 0xf4 }]
}

fn cbor_null() -> Vec<u8> {
    vec![0xf6]
}

fn cbor_len_prefix(major: u8, len: usize) -> Vec<u8> {
    let major = major << 5;
    if len <= 23 {
        vec![major | len as u8]
    } else if len <= 0xff {
        vec![major | 24, len as u8]
    } else if len <= 0xffff {
        let mut v = vec![major | 25];
        v.extend_from_slice(&(len as u16).to_be_bytes());
        v
    } else {
        let mut v = vec![major | 26];
        v.extend_from_slice(&(len as u32).to_be_bytes());
        v
    }
}

fn blake2b_256(data: &[u8]) -> [u8; 32] {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut h = Blake2b::<U32>::new();
    h.update(data);
    h.finalize().into()
}

#[cfg(test)]
#[path = "tests/cardano.rs"]
mod tests;

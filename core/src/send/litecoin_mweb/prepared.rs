//! What an MWEB transaction is reviewed as, and signing it.
//!
//! A payment out of MWEB funds is reviewed as the outputs it spends and what
//! it pays — an MWEB address, or any other Litecoin address by a peg-out —
//! with its change and fee; a peg-in as the canonical inputs it spends and
//! what arrives in MWEB. Signing makes the whole transaction from that: fresh
//! sender, input and stealth keys, outputs with their proofs, the kernel.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::keys::{self, StealthAddress, ViewKeys};
use super::output::OwnedOutput;
use super::transaction::{self, MAX_STANDARD_INPUTS, PEGOUT_FEE_PER_KB, Plan, Spend};
use crate::api::litecoin_p2p::wire::{MwebTx, PegOut, pegin_script};
use crate::registry::Chain;
use crate::send::error::SendError;

/// An owned output a payment spends, as its review names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CoinRef {
    #[serde(with = "hex::serde")]
    pub output_id: [u8; 32],
    pub value: u64,
}

/// Where a payment out of MWEB funds goes: an MWEB address, or any other
/// Litecoin address, which a peg-out pays from the block's HogEx.
pub(crate) enum Destination {
    Stealth(StealthAddress),
    PegOut(Vec<u8>),
}

impl Destination {
    /// The destination `recipient` names on `chain`'s network.
    pub(crate) fn parse(chain: Chain, recipient: &str) -> Result<Self, SendError> {
        if let Some(address) = StealthAddress::decode(chain, recipient) {
            return Ok(Self::Stealth(address));
        }
        Ok(Self::PegOut(
            crate::derivation::utxo_address::parse_utxo_address(chain, recipient)?.script_pubkey(),
        ))
    }

    /// The MWEB outputs and peg-outs paying `amount` here takes.
    fn parts(&self, amount: u64) -> (Vec<(StealthAddress, u64)>, Vec<PegOut>) {
        match self {
            Self::Stealth(address) => (vec![(*address, amount)], Vec::new()),
            Self::PegOut(script) => (
                Vec::new(),
                vec![PegOut {
                    amount,
                    script: script.clone(),
                }],
            ),
        }
    }
}

/// A payment out of MWEB funds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedMwebSpend {
    pub inputs: Vec<CoinRef>,
    /// The recipient, as entered: an MWEB address, or another Litecoin
    /// address for a peg-out.
    pub recipient: String,
    /// What the recipient receives, in litoshis.
    pub amount: u64,
    /// Returned to the wallet's change address.
    pub change: u64,
    pub fee: u64,
}

impl PreparedMwebSpend {
    /// Whether this is a payment Litecoin Core relays and that moves exactly
    /// what it says: distinct inputs no more than it relays, paying the
    /// amount, the change and a fee no less than the transaction's weight
    /// costs; and the recipient `chain`'s.
    pub(crate) fn check(&self, chain: Chain) -> Result<Destination, SendError> {
        let destination = Destination::parse(chain, &self.recipient)?;
        let mut seen = HashSet::new();
        if self.inputs.is_empty()
            || self.inputs.len() > MAX_STANDARD_INPUTS
            || !self.inputs.iter().all(|coin| seen.insert(coin.output_id))
        {
            return Err(SendError::invalid(
                "The MWEB payment's inputs are not valid",
            ));
        }
        let spent = self
            .inputs
            .iter()
            .try_fold(0u64, |total, coin| total.checked_add(coin.value));
        let paid = self
            .amount
            .checked_add(self.change)
            .and_then(|total| total.checked_add(self.fee));
        let (recipients, pegouts) = destination.parts(self.amount);
        let outputs = recipients.len() as u64 + u64::from(self.change > 0);
        if self.amount == 0
            || spent.is_none()
            || spent != paid
            || self.fee < transaction::fee(outputs, &pegouts, PEGOUT_FEE_PER_KB)?
        {
            return Err(SendError::invalid("The MWEB payment does not balance"));
        }
        if let Destination::PegOut(script) = &destination
            && self.amount < crate::send::litecoin::litecoin_dust_threshold(chain, script)?
        {
            return Err(SendError::invalid(
                "Litecoin recipient amount is below the dust threshold",
            ));
        }
        Ok(destination)
    }
}

/// The coins to spend for `amount` to `recipient`, largest first, and the
/// fee: the transaction's weight's, with a change output when what is left
/// over pays for one; less than that is left to the fee.
pub(crate) fn plan_spend(
    chain: Chain,
    coins: &[OwnedOutput],
    recipient: &str,
    amount: u64,
) -> Result<PreparedMwebSpend, SendError> {
    if amount == 0 {
        return Err(SendError::invalid("The amount must be positive."));
    }
    let destination = Destination::parse(chain, recipient)?;
    if let Destination::PegOut(script) = &destination
        && amount < crate::send::litecoin::litecoin_dust_threshold(chain, script)?
    {
        return Err(SendError::invalid(
            "Litecoin recipient amount is below the dust threshold",
        ));
    }
    let (recipients, pegouts) = destination.parts(amount);
    let paid_outputs = recipients.len() as u64;
    let fee_without_change = transaction::fee(paid_outputs, &pegouts, PEGOUT_FEE_PER_KB)?;
    let fee_with_change = transaction::fee(paid_outputs + 1, &pegouts, PEGOUT_FEE_PER_KB)?;
    let mut sorted: Vec<&OwnedOutput> = coins.iter().collect();
    sorted.sort_by(|a, b| b.value.cmp(&a.value).then(a.output_id.cmp(&b.output_id)));
    let mut chosen = Vec::new();
    let mut total: u64 = 0;
    for coin in sorted.into_iter().take(MAX_STANDARD_INPUTS) {
        chosen.push(coin);
        total = total
            .checked_add(coin.value)
            .ok_or_else(|| SendError::invalid("MWEB value overflow"))?;
        let Some(left) = total.checked_sub(amount) else {
            continue;
        };
        let (change, fee) = if left > fee_with_change {
            (left - fee_with_change, fee_with_change)
        } else if left >= fee_without_change {
            // Change too small to pay for its own output goes to the fee.
            (0, left)
        } else {
            continue;
        };
        return Ok(PreparedMwebSpend {
            inputs: chosen
                .iter()
                .map(|coin| CoinRef {
                    output_id: coin.output_id,
                    value: coin.value,
                })
                .collect(),
            recipient: recipient.to_string(),
            amount,
            change,
            fee,
        });
    }
    Err(SendError::InsufficientFunds(
        "The MWEB balance cannot cover the amount and the fee.".into(),
    ))
}

/// A signed transaction: its bytes, and its id as explorers write it.
pub(crate) struct Signed {
    pub raw: Vec<u8>,
    pub txid: String,
}

/// The transaction a reviewed payment out of MWEB funds makes. `coins`
/// holds the wallet's unspent outputs by id; each input must be one of
/// them, with the value it was reviewed with.
pub(crate) fn sign_spend(
    chain: Chain,
    prepared: &PreparedMwebSpend,
    coins: &HashMap<[u8; 32], OwnedOutput>,
    view: &ViewKeys,
    spend_secret: &[u8; 32],
) -> Result<Signed, SendError> {
    let destination = prepared.check(chain)?;
    let mut spends = Vec::new();
    for input in &prepared.inputs {
        let coin = coins
            .get(&input.output_id)
            .filter(|coin| coin.value == input.value)
            .ok_or_else(|| {
                SendError::invalid(
                    "An MWEB output this payment spends is gone; sync and build again",
                )
            })?;
        let address_secret = keys::address_spend_secret(view, spend_secret, coin.address_index)?;
        spends.push(Spend {
            output_secret: coin.output_secret(&address_secret)?,
            coin: coin.clone(),
        });
    }
    let (mut recipients, pegouts) = destination.parts(prepared.amount);
    if prepared.change > 0 {
        recipients.push((view.address(keys::CHANGE_INDEX)?, prepared.change));
    }
    let tx = transaction::build(&Plan {
        spends,
        recipients,
        fee: prepared.fee,
        pegin: 0,
        pegouts,
    })?;
    transaction::verify(&tx)?;
    Ok(envelope(empty_canonical(), &tx))
}

fn empty_canonical() -> bitcoin::Transaction {
    bitcoin::Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: Vec::new(),
        output: Vec::new(),
    }
}

fn envelope(canonical: bitcoin::Transaction, mweb: &MwebTx) -> Signed {
    Signed {
        raw: transaction::litecoin_transaction(&canonical, mweb),
        txid: transaction::litecoin_txid(&canonical, mweb),
    }
}

/// Whom a transaction from transparent funds pays: a transparent address's
/// script, or an MWEB address, which a peg-in pays.
pub(crate) enum CanonicalRecipient {
    Script(Vec<u8>),
    PegIn,
}

impl CanonicalRecipient {
    pub(crate) fn parse(chain: Chain, recipient: &str) -> Result<Self, SendError> {
        if StealthAddress::decode(chain, recipient).is_some() {
            return Ok(Self::PegIn);
        }
        Ok(Self::Script(
            crate::derivation::utxo_address::parse_utxo_address(chain, recipient)?.script_pubkey(),
        ))
    }

    /// The length of the script the canonical output pays: a peg-in's
    /// names its kernel, 34 bytes.
    pub(crate) fn script_len(&self) -> usize {
        match self {
            Self::Script(script) => script.len(),
            Self::PegIn => pegin_script(&[0; 32]).len(),
        }
    }

    /// The MWEB fee the canonical output carries beside the amount: a
    /// peg-in kernel's.
    pub(crate) fn mweb_fee(&self) -> Result<u64, SendError> {
        match self {
            Self::Script(_) => Ok(0),
            Self::PegIn => PreparedPegIn::kernel_fee(),
        }
    }

    /// The least the recipient can be paid: what keeps the canonical output
    /// at Litecoin Core's dust threshold.
    pub(crate) fn minimum_amount(&self, chain: Chain) -> Result<u64, SendError> {
        let dust = |script: &[u8]| crate::send::litecoin::litecoin_dust_threshold(chain, script);
        Ok(match self {
            Self::Script(script) => dust(script)?,
            Self::PegIn => dust(&pegin_script(&[0; 32]))?
                .saturating_sub(self.mweb_fee()?)
                .max(1),
        })
    }
}

/// A peg-in: canonical inputs and change, and the MWEB output it pays.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedPegIn {
    pub inputs: Vec<crate::send::stages::UtxoPreparedInput>,
    /// The MWEB address that receives it, as entered.
    pub recipient: String,
    /// What arrives there, in litoshis.
    pub amount: u64,
    /// The kernel's fee, paid out of what the canonical output pegs in.
    pub mweb_fee: u64,
    /// The canonical transaction's fee.
    pub canonical_fee: u64,
}

impl PreparedPegIn {
    /// The MWEB fee of a peg-in: one output and its kernel's weight.
    pub(crate) fn kernel_fee() -> Result<u64, SendError> {
        transaction::fee(1, &[], 0)
    }

    /// What the canonical peg-in output carries: the amount and the
    /// kernel's fee.
    pub(crate) fn pegin_value(&self) -> Result<u64, SendError> {
        self.amount
            .checked_add(self.mweb_fee)
            .ok_or_else(|| SendError::invalid("MWEB value overflow"))
    }

    /// The MWEB address it pays on `chain`, after checking the kernel pays
    /// at least its weight's fee.
    pub(crate) fn check(&self, chain: Chain) -> Result<StealthAddress, SendError> {
        let address = StealthAddress::decode(chain, &self.recipient)
            .ok_or_else(|| SendError::invalid("Not an MWEB address on this network"))?;
        if self.inputs.is_empty()
            || self.amount == 0
            || self.mweb_fee < Self::kernel_fee()?
            || self.canonical_fee == 0
        {
            return Err(SendError::invalid("The peg-in does not balance"));
        }
        self.pegin_value()?;
        Ok(address)
    }
}

/// A peg-in's MWEB half, and the canonical output script its kernel makes:
/// `canonical` signs the transaction paying that script the peg-in value,
/// and the two travel together.
pub(crate) fn sign_pegin(
    chain: Chain,
    prepared: &PreparedPegIn,
    canonical: impl FnOnce(&[u8], u64) -> Result<Vec<u8>, SendError>,
) -> Result<Signed, SendError> {
    let address = prepared.check(chain)?;
    let pegin = prepared.pegin_value()?;
    let tx = transaction::build(&Plan {
        spends: Vec::new(),
        recipients: vec![(address, prepared.amount)],
        fee: prepared.mweb_fee,
        pegin,
        pegouts: Vec::new(),
    })?;
    transaction::verify(&tx)?;
    let kernel = tx
        .body
        .kernels
        .first()
        .ok_or_else(|| SendError::Internal("a peg-in with no kernel".into()))?;
    let raw = canonical(&pegin_script(&kernel.id()), pegin)?;
    let canonical_tx: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&raw).map_err(SendError::invalid)?;
    Ok(envelope(canonical_tx, &tx))
}

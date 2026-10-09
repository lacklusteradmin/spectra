//! TON's multisig v2 contract (ton-blockchain/multisig-contract-v2): its
//! signers propose an order, a dictionary of actions, which the multisig
//! deploys as an Order contract at an address derived from its number; the
//! signers approve the order there, and the approval that meets the
//! threshold sends it back to the multisig, which executes it. Approvals
//! are internal messages from each signer's own wallet.
//!
//! The orders Spectra writes and reads hold one action: a transfer from the
//! multisig, sent in mode 3. An order read from the network is decoded and
//! written again; one that does not come out the same is refused.

use crate::derivation::ton_cell::{Cell, dictionary, single_entry_dictionary};
use crate::send::error::SendError;

/// The multisig contract's code hash (v2, as deployed by its UI).
pub(crate) const MULTISIG_CODE_HASH: &str =
    "d3d14da9a627f0ec3533341829762af92b9540b21bf03665fac09c2b46eabbac";
/// The Order contract's code, which its state names as a library.
const ORDER_CODE_HASH: &str = "6305a8061c856c2ccf05dcb0df5815c71475870567cab5f049e340bcf59251f3";

const OP_NEW_ORDER: u64 = 0xf718_510f;
const OP_APPROVE: u64 = 0xa762_230f;
const ACTION_SEND_MESSAGE: u64 = 0xf138_1e5b;
/// The mode an order's transfer is sent in: pay fees apart, ignore errors.
const SEND_MODE: u64 = 3;

/// A standard address: workchain and account.
pub(crate) type TonAccount = (i8, [u8; 32]);

fn unread() -> SendError {
    SendError::invalid("The order is not a transfer as Spectra writes one.")
}

/// A multisig's data: the next order's number, the threshold, its signers
/// by index, how many proposers it has, and whether orders may take any
/// number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultisigData {
    pub next_order_seqno: u64,
    pub threshold: u8,
    pub signers: Vec<TonAccount>,
    pub proposers: usize,
    pub allow_arbitrary_seqno: bool,
}

/// An index-keyed dictionary of addresses, its keys 0, 1, ….
fn addresses(dict: &Cell) -> Result<Vec<TonAccount>, SendError> {
    let mut entries = dictionary(dict, 8)?;
    entries.sort_by_key(|(key, _)| *key);
    entries
        .into_iter()
        .enumerate()
        .map(|(index, (key, mut value))| {
            if key != index as u64 {
                return Err(SendError::invalid("A signer index is missing."));
            }
            let address = value.address()?;
            value.end()?;
            Ok(address)
        })
        .collect()
}

impl MultisigData {
    pub(crate) fn parse(data: &Cell) -> Result<Self, SendError> {
        let mut reader = data.reader();
        let next_order_seqno = reader.uint256_as_u64()?;
        let threshold = reader.uint(8)? as u8;
        let signers = addresses(reader.reference()?)?;
        let signers_num = reader.uint(8)? as usize;
        let proposers = reader
            .maybe_reference()?
            .map(addresses)
            .transpose()?
            .map_or(0, |proposers| proposers.len());
        let allow_arbitrary_seqno = reader.bit()?;
        reader.end()?;
        if signers_num != signers.len() || threshold == 0 || usize::from(threshold) > signers.len()
        {
            return Err(SendError::invalid(
                "The multisig's signers are inconsistent.",
            ));
        }
        Ok(Self {
            next_order_seqno,
            threshold,
            signers,
            proposers,
            allow_arbitrary_seqno,
        })
    }
}

/// An order's state once its multisig deployed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderState {
    pub threshold: u8,
    pub sent_for_execution: bool,
    pub signers: Vec<TonAccount>,
    /// The signers who approved, by index.
    pub approvals: Vec<u8>,
    pub expiration: u64,
    pub order: Cell,
}

impl OrderState {
    /// An order's data, refused unless it is `multisig`'s order `seqno`;
    /// `None` while the multisig has not initialized it.
    pub(crate) fn parse(
        data: &Cell,
        multisig: &TonAccount,
        seqno: u64,
    ) -> Result<Option<Self>, SendError> {
        let mut reader = data.reader();
        if reader.address()? != *multisig || reader.uint256_as_u64()? != seqno {
            return Err(SendError::invalid(
                "The order belongs to another multisig or number.",
            ));
        }
        if reader.remaining_bits() == 0 {
            reader.end()?;
            return Ok(None);
        }
        let threshold = reader.uint(8)? as u8;
        let sent_for_execution = reader.bit()?;
        let signers = addresses(reader.reference()?)?;
        let mask: [u8; 32] = reader.bytes()?;
        let approvals_num = reader.uint(8)?;
        let expiration = reader.uint(48)?;
        let order = reader.reference()?.clone();
        reader.end()?;
        let approvals: Vec<u8> = (0..=255u8)
            .filter(|index| mask[31 - usize::from(*index) / 8] >> (index % 8) & 1 == 1)
            .collect();
        if approvals.len() as u64 != approvals_num
            || approvals
                .iter()
                .any(|index| usize::from(*index) >= signers.len())
        {
            return Err(SendError::invalid(
                "The order's approvals are inconsistent.",
            ));
        }
        Ok(Some(Self {
            threshold,
            sent_for_execution,
            signers,
            approvals,
            expiration,
            order,
        }))
    }
}

/// The one transfer an order makes from the multisig.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TonTransferOrder {
    pub destination: TonAccount,
    pub bounce: bool,
    pub nanotons: u64,
    pub comment: Option<String>,
}

impl TonTransferOrder {
    /// The transfer as a relaxed internal message: no source, no fees, the
    /// body inline where it fits.
    fn message(&self) -> Result<Cell, SendError> {
        let mut message = Cell::default();
        message
            .uint(0, 1)?
            .uint(1, 1)?
            .uint(u64::from(self.bounce), 1)?
            .uint(0, 1)?
            .uint(0, 2)?
            .address(self.destination.0, &self.destination.1)?
            .coins(self.nanotons)?
            .uint(0, 1)?
            .coins(0)?
            .coins(0)?
            .uint(0, 64)?
            .uint(0, 32)?
            .uint(0, 1)?;
        match self.comment.as_deref().filter(|text| !text.is_empty()) {
            Some(text) => {
                message.body(crate::send::ton::comment_cell(text)?)?;
            }
            None => {
                message.uint(0, 1)?;
            }
        }
        Ok(message)
    }

    /// The order: action 0, `send_message` in mode 3.
    pub(crate) fn order(&self) -> Result<Cell, SendError> {
        if self.nanotons == 0 {
            return Err(SendError::invalid("TON: amount must be positive"));
        }
        let mut action = Cell::default();
        action
            .uint(ACTION_SEND_MESSAGE, 32)?
            .uint(SEND_MODE, 8)?
            .reference(self.message()?)?;
        let mut value = Cell::default();
        value.reference(action)?;
        Ok(single_entry_dictionary(8, 0, value)?)
    }

    /// `order` as the one transfer it makes; refused unless it encodes back
    /// to the same cell.
    pub(crate) fn decode(order: &Cell) -> Result<Self, SendError> {
        let entries = dictionary(order, 8).map_err(|_| unread())?;
        let [(0, value)] = entries.as_slice() else {
            return Err(unread());
        };
        let mut value = value.clone();
        let mut action = value.reference()?.reader();
        value.end()?;
        if action.uint(32)? != ACTION_SEND_MESSAGE || action.uint(8)? != SEND_MODE {
            return Err(unread());
        }
        let mut message = action.reference()?.reader();
        action.end()?;
        if message.bit()? || !message.bit()? {
            return Err(unread());
        }
        let bounce = message.bit()?;
        if message.bit()? || message.uint(2)? != 0 {
            return Err(unread());
        }
        let destination = message.address()?;
        let nanotons = u64::try_from(message.coins()?).map_err(|_| unread())?;
        message.bit()?;
        message.coins()?;
        message.coins()?;
        message.uint(64)?;
        message.uint(32)?;
        message.bit()?;
        let mut body = if message.bit()? {
            let body = message.reference()?.reader();
            message.end()?;
            body
        } else {
            message
        };
        let comment = if body.remaining_bits() == 0 {
            None
        } else {
            if body.uint(32)? != 0 {
                return Err(unread());
            }
            let mut text = Vec::new();
            loop {
                if body.remaining_bits() % 8 != 0 {
                    return Err(unread());
                }
                while body.remaining_bits() > 0 {
                    text.push(body.uint(8)? as u8);
                }
                match body.reference() {
                    Ok(next) => body = next.reader(),
                    Err(_) => break,
                }
            }
            Some(String::from_utf8(text).map_err(|_| unread())?)
        };
        let decoded = Self {
            destination,
            bounce,
            nanotons,
            comment,
        };
        if decoded.order()?.hash_depth().0 != order.hash_depth().0 {
            return Err(unread());
        }
        Ok(decoded)
    }
}

/// The address of `multisig`'s order `seqno`: the hash of its state, the
/// Order code by library and the multisig and number as its data, on the
/// basic workchain.
pub(crate) fn order_address(multisig: &TonAccount, seqno: u64) -> Result<TonAccount, SendError> {
    let code_hash: [u8; 32] = hex::decode(ORDER_CODE_HASH)
        .expect("constant")
        .try_into()
        .expect("32 bytes");
    let mut data = Cell::default();
    data.address(multisig.0, &multisig.1)?
        .uint(0, 64)?
        .uint(0, 64)?
        .uint(0, 64)?
        .uint(seqno, 64)?;
    let mut state = Cell::default();
    state
        .uint(0, 2)?
        .uint(1, 1)?
        .reference(Cell::library(&code_hash)?)?
        .uint(1, 1)?
        .reference(data)?
        .uint(0, 1)?;
    Ok((0, state.hash_depth().0))
}

/// A signer's proposal of `order` as number `seqno`, which counts as its
/// approval.
pub(crate) fn new_order(
    query_id: u64,
    seqno: u64,
    signer_index: u8,
    expiration: u64,
    order: Cell,
) -> Result<Cell, SendError> {
    let mut body = Cell::default();
    body.uint(OP_NEW_ORDER, 32)?
        .uint(query_id, 64)?
        .uint(0, 64)?
        .uint(0, 64)?
        .uint(0, 64)?
        .uint(seqno, 64)?
        .uint(1, 1)?
        .uint(u64::from(signer_index), 8)?
        .uint(expiration, 48)?
        .reference(order)?;
    Ok(body)
}

/// A signer's approval of the order it is sent to.
pub(crate) fn approve(query_id: u64, signer_index: u8) -> Result<Cell, SendError> {
    let mut body = Cell::default();
    body.uint(OP_APPROVE, 32)?
        .uint(query_id, 64)?
        .uint(u64::from(signer_index), 8)?;
    Ok(body)
}

#[cfg(test)]
#[path = "tests/ton_multisig.rs"]
mod tests;

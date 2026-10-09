//! A Substrate multisig account's calls (`pallet-multisig`): the transfer
//! the account makes, and the approvals its signatories submit for it, each
//! from its own account. Every approval but the one that meets the
//! threshold names the transfer by its hash (`approve_as_multi`); that one
//! carries the call itself, and the weight it may dispatch with
//! (`as_multi`), and the runtime executes it. All but the first name the
//! block and extrinsic index of the first, the operation's timepoint.

use parity_scale_codec::{Compact, Decode, Encode};

use crate::api::substrate_json_rpc::multisig::MultisigPallet;
use crate::send::error::SendError;
use crate::send::polkadot::transfer_call_at;

/// Where an operation's first approval landed: block height and extrinsic
/// index.
pub(crate) type Timepoint = (u32, u32);

/// The transfer a call makes: `Balances.transfer_keep_alive` as the
/// runtime's pallet and call indices lay it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MultisigTransfer {
    pub recipient: [u8; 32],
    pub amount: u128,
}

/// `call` as a transfer under `(pallet, call)` indices, refused unless it
/// encodes back byte for byte.
pub(crate) fn decode_transfer(
    call: &[u8],
    indices: (u8, u8),
) -> Result<MultisigTransfer, SendError> {
    let unread = || SendError::invalid("The call is not a transfer as Spectra writes one.");
    let [pallet, index, 0, rest @ ..] = call else {
        return Err(unread());
    };
    if (*pallet, *index) != indices || rest.len() < 32 {
        return Err(unread());
    }
    let recipient: [u8; 32] = rest[..32].try_into().map_err(|_| unread())?;
    let mut input = &rest[32..];
    let amount = Compact::<u128>::decode(&mut input).map_err(|_| unread())?.0;
    let transfer = MultisigTransfer { recipient, amount };
    if transfer_call(indices, &transfer) != call || amount == 0 {
        return Err(unread());
    }
    Ok(transfer)
}

/// The call making `transfer`.
pub(crate) fn transfer_call(indices: (u8, u8), transfer: &MultisigTransfer) -> Vec<u8> {
    transfer_call_at(indices, &transfer.recipient, transfer.amount)
}

/// A call's hash, which names its operation: `blake2_256(call)`.
pub(crate) fn call_hash(call: &[u8]) -> [u8; 32] {
    super::substrate::blake2b_256(call)
}

fn approval_head(
    pallet: &MultisigPallet,
    call: u8,
    threshold: u16,
    others: &[[u8; 32]],
    timepoint: Option<Timepoint>,
) -> Vec<u8> {
    let mut out = vec![pallet.index, call];
    out.extend(threshold.encode());
    out.extend(others.to_vec().encode());
    match timepoint {
        Some((height, index)) => {
            out.push(1);
            out.extend(height.encode());
            out.extend(index.encode());
        }
        None => out.push(0),
    }
    out
}

/// An approval naming the operation by its call's hash. It dispatches
/// nothing, so it caps no weight.
pub(crate) fn approve_as_multi(
    pallet: &MultisigPallet,
    threshold: u16,
    others: &[[u8; 32]],
    timepoint: Option<Timepoint>,
    hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = approval_head(
        pallet,
        pallet.approve_as_multi,
        threshold,
        others,
        timepoint,
    );
    out.extend(hash);
    out.extend(Compact(0u64).encode());
    out.extend(Compact(0u64).encode());
    out
}

/// The approval carrying `call`, which the runtime dispatches once it meets
/// the threshold, within `max_weight`.
pub(crate) fn as_multi(
    pallet: &MultisigPallet,
    threshold: u16,
    others: &[[u8; 32]],
    timepoint: Option<Timepoint>,
    call: &[u8],
    max_weight: (u64, u64),
) -> Vec<u8> {
    let mut out = approval_head(pallet, pallet.as_multi, threshold, others, timepoint);
    out.extend(call);
    out.extend(Compact(max_weight.0).encode());
    out.extend(Compact(max_weight.1).encode());
    out
}

#[cfg(test)]
#[path = "tests/substrate_multisig.rs"]
mod tests;

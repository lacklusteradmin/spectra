//! Zcash transactions that touch the shielded pools: a payment from the
//! wallet's shielded notes, and shielding its transparent funds.
//!
//! librustzcash chooses the inputs, the change and the ZIP-317 fee as a
//! proposal over the wallet's own shielded database. The proposal is what is
//! reviewed: its protobuf encoding and what it pays are bound into the review
//! digest. Signing decodes it against the database again, refuses it when
//! what it would pay has changed, then proves and signs it with the spending
//! key ZIP-32 derives from the seed — Orchard proofs need no parameters,
//! Sapling ones the published Groth16 parameters (`zcash_params`).

use prost::Message;
use serde::{Deserialize, Serialize};
use zcash_client_backend::data_api::WalletRead;
use zcash_client_backend::data_api::error::Error as ProposalError;
use zcash_client_backend::data_api::wallet::{
    ConfirmationsPolicy, SpendingKeys, create_proposed_transactions,
    input_selection::GreedyInputSelector, propose_shielding, propose_standard_transfer_to_address,
};
use zcash_client_backend::fees::{
    ChangeError, DustOutputPolicy, StandardFeeRule, standard::SingleOutputChangeStrategy,
};
use zcash_client_backend::proposal::Proposal;
use zcash_client_backend::wallet::OvkPolicy;
use zcash_keys::address::Address;
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_protocol::consensus::Network;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zcash_protocol::{PoolType, ShieldedPool};

use crate::send::error::SendError;
use crate::wallet_db::zcash::ZcashDb;

/// One payment a proposal makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ZcashPayment {
    /// The recipient, as it was entered and as ZIP-321 encodes it.
    pub address: String,
    pub zatoshis: u64,
    /// The memo, for a shielded recipient.
    pub memo: Option<String>,
}

/// A reviewed proposal: what it spends and pays, and the proposal itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedZcashShielded {
    /// zcash_client_backend's protobuf encoding of the proposal, in hex.
    pub proposal_hex: String,
    /// What it pays to others, in order.
    pub payments: Vec<ZcashPayment>,
    /// The ZIP-317 fee of every transaction it makes, together.
    pub fee_zat: u64,
    /// Transparent value it spends: what shielding moves in.
    pub transparent_in_zat: u64,
    /// Shielded value it spends.
    pub shielded_in_zat: u64,
    /// Value returned to the wallet's shielded pools.
    pub change_zat: u64,
    /// It spends Sapling notes.
    pub spends_sapling: bool,
    /// It spends or creates Sapling notes, so proving it needs the Sapling
    /// parameters.
    pub uses_sapling: bool,
}

/// The shielded account a wallet's database holds; there is one.
pub(crate) fn account_id(db: &ZcashDb) -> Result<zcash_client_sqlite::AccountUuid, SendError> {
    db.get_account_ids()
        .map_err(failure)?
        .into_iter()
        .next()
        .ok_or_else(|| SendError::invalid("Sync the shielded wallet before sending from it."))
}

fn failure(error: impl std::fmt::Display) -> SendError {
    SendError::Internal(error.to_string())
}

/// What a proposal pays and spends, read from the proposal itself.
fn review<N>(
    network: &Network,
    proposal: &Proposal<StandardFeeRule, N>,
) -> Result<PreparedZcashShielded, SendError> {
    let mut payments = Vec::new();
    let mut fee = Zatoshis::ZERO;
    let mut transparent_in = Zatoshis::ZERO;
    let mut shielded_in = Zatoshis::ZERO;
    let mut change = Zatoshis::ZERO;
    let mut spends_sapling = false;
    let mut uses_sapling = false;
    let add = |a: Zatoshis, b: Zatoshis| {
        (a + b).ok_or_else(|| SendError::invalid("Zcash value overflow"))
    };
    // One transaction: a proposal of several (a TEX payment's two legs) is
    // refused before it is made.
    if proposal.steps().len() != 1 {
        return Err(SendError::Internal(
            "a shielded proposal of more than one transaction".into(),
        ));
    }
    for step in proposal.steps() {
        fee = add(fee, step.balance().fee_required())?;
        for input in step.transparent_inputs() {
            transparent_in = add(transparent_in, input.txout().value())?;
        }
        if let Some(inputs) = step.shielded_inputs() {
            for note in inputs.notes() {
                shielded_in = add(shielded_in, note.note().value())?;
                if note.note().pool() == ShieldedPool::Sapling {
                    spends_sapling = true;
                }
            }
        }
        for (index, payment) in step.transaction_request().payments() {
            let pool = step.payment_pools().get(index).copied();
            if pool == Some(PoolType::SAPLING) {
                uses_sapling = true;
            }
            payments.push(ZcashPayment {
                address: payment.recipient_address().encode(),
                zatoshis: payment.amount().map_or(0, u64::from),
                memo: payment
                    .memo()
                    .and_then(|memo| zcash_protocol::memo::Memo::try_from(memo.clone()).ok())
                    .and_then(|memo| match memo {
                        zcash_protocol::memo::Memo::Text(text) => Some(text.to_string()),
                        _ => None,
                    }),
            });
        }
        for value in step.balance().proposed_change() {
            if value.is_ephemeral() {
                return Err(SendError::Internal(
                    "a shielded proposal with an ephemeral output".into(),
                ));
            }
            change = add(change, value.value())?;
            if value.output_pool() == PoolType::SAPLING {
                uses_sapling = true;
            }
        }
    }
    let _ = network;
    Ok(PreparedZcashShielded {
        proposal_hex: hex::encode(
            zcash_client_backend::proto::proposal::Proposal::from_standard_proposal(proposal)
                .encode_to_vec(),
        ),
        payments,
        fee_zat: fee.into(),
        transparent_in_zat: transparent_in.into(),
        shielded_in_zat: shielded_in.into(),
        change_zat: change.into(),
        spends_sapling,
        uses_sapling: uses_sapling || spends_sapling,
    })
}

/// What a proposal that failed means to the person sending.
fn propose_error<DE, TE, SE, FE, CE, N>(error: ProposalError<DE, TE, SE, FE, CE, N>) -> SendError
where
    DE: std::fmt::Display,
    TE: std::fmt::Display,
    SE: std::fmt::Display,
    FE: std::fmt::Display,
    CE: std::fmt::Display,
    N: std::fmt::Display,
{
    match error {
        ProposalError::InsufficientFunds { .. }
        | ProposalError::Change(ChangeError::InsufficientFunds { .. }) => {
            SendError::InsufficientFunds(
                "The shielded balance cannot cover the amount and the fee.".into(),
            )
        }
        ProposalError::Change(ChangeError::DustInputs { .. }) => {
            SendError::InsufficientFunds("The funds are too small to spend after the fee.".into())
        }
        ProposalError::ScanRequired => {
            SendError::invalid("Sync the shielded wallet before sending from it.")
        }
        other => SendError::Internal(other.to_string()),
    }
}

/// Propose paying `amount` zatoshis to `recipient` from the wallet's
/// shielded notes. A memo goes only to a shielded recipient.
pub(crate) fn propose_payment(
    db: &mut ZcashDb,
    network: &Network,
    recipient: &str,
    amount: u64,
    memo: Option<&str>,
) -> Result<PreparedZcashShielded, SendError> {
    let address = Address::decode(network, recipient.trim())
        .ok_or_else(|| SendError::invalid("Not a Zcash address on this network"))?;
    // ZIP-320: a TEX address takes only transparent funds, which shielded
    // ones reach through a transparent hop of their own; the transparent
    // balance pays it directly.
    if matches!(address, Address::Tex(_)) {
        return Err(SendError::invalid(
            "A TEX address is paid from the wallet's transparent balance.",
        ));
    }
    let memo = match memo.map(str::trim).filter(|memo| !memo.is_empty()) {
        None => None,
        Some(_) if matches!(address, Address::Transparent(_)) => {
            return Err(SendError::invalid(
                "A memo reaches only a shielded recipient.",
            ));
        }
        Some(text) => Some(
            MemoBytes::from_bytes(text.as_bytes())
                .map_err(|_| SendError::invalid("A memo is at most 512 bytes."))?,
        ),
    };
    let amount =
        Zatoshis::from_u64(amount).map_err(|_| SendError::invalid("Invalid Zcash amount"))?;
    if amount == Zatoshis::ZERO {
        return Err(SendError::invalid("The amount must be positive."));
    }
    let account = account_id(db)?;
    let proposal = propose_standard_transfer_to_address::<_, _, std::convert::Infallible>(
        db,
        network,
        StandardFeeRule::Zip317,
        account,
        ConfirmationsPolicy::default(),
        &address,
        amount,
        memo,
        None,
        ShieldedPool::Orchard,
        None,
        None,
    )
    .map_err(propose_error)?;
    review(network, &proposal)
}

/// Propose moving every spendable transparent output of the wallet into its
/// Orchard pool.
pub(crate) fn propose_shielding_all(
    db: &mut ZcashDb,
    network: &Network,
) -> Result<PreparedZcashShielded, SendError> {
    let account = account_id(db)?;
    let receivers: Vec<_> = db
        .get_transparent_receivers(account, true, false)
        .map_err(failure)?
        .into_keys()
        .collect();
    if receivers.is_empty() {
        return Err(SendError::invalid("Nothing to shield."));
    }
    let input_selector = GreedyInputSelector::new();
    let change_strategy = SingleOutputChangeStrategy::new(
        StandardFeeRule::Zip317,
        None,
        ShieldedPool::Orchard,
        DustOutputPolicy::default(),
    );
    let proposal = propose_shielding::<_, _, _, _, std::convert::Infallible>(
        db,
        network,
        &input_selector,
        &change_strategy,
        Zatoshis::const_from_u64(10_000),
        &receivers,
        account,
        ConfirmationsPolicy::default(),
        zcash_client_backend::data_api::CoinbaseFilter::AllTransparentOutputs,
        None,
    )
    .map_err(|error| match error {
        ProposalError::InsufficientFunds { .. }
        | ProposalError::Change(
            ChangeError::InsufficientFunds { .. } | ChangeError::DustInputs { .. },
        ) => SendError::InsufficientFunds(
            "The transparent balance is too small to shield after the fee.".into(),
        ),
        other => propose_error(other),
    })?;
    review(network, &proposal)
}

/// The Groth16 parameters a Sapling spend or output is proved with.
pub(crate) struct SaplingParameters {
    pub spend: Vec<u8>,
    pub output: Vec<u8>,
}

/// Where the published Sapling parameters are, and what each must hash to
/// (BLAKE2b-512), as zcash_proofs pins them.
pub(crate) const SAPLING_PARAMETERS_URL: &str = "https://download.z.cash/downloads";
pub(crate) const SAPLING_PARAMETER_FILES: [(&str, &str, usize); 2] = [
    (
        "sapling-spend.params",
        "8270785a1a0d0bc77196f000ee6d221c9c9894f55307bd9357c3f0105d31ca63991ab91324160d8f53e2bbd3c2633a6eb8bdf5205d822e7f3f73edac51b2b70c",
        47_958_396,
    ),
    (
        "sapling-output.params",
        "657e3d38dbb5cb5e7dd2970e8b03d69b4787dd907285b5a7f0790dcc8072f60bf593b32cc2d1c030e00ff5ae64bf84c5c3beb84ddc841d48264b4a171744d028",
        3_592_860,
    ),
];

/// Whether `bytes` are the parameter file `name`, by its pinned hash.
pub(crate) fn sapling_parameter_is_genuine(name: &str, bytes: &[u8]) -> bool {
    SAPLING_PARAMETER_FILES
        .iter()
        .find(|(file, _, _)| *file == name)
        .is_some_and(|(_, hash, size)| {
            bytes.len() == *size
                && hex::encode(
                    blake2b_simd::Params::new()
                        .hash_length(64)
                        .hash(bytes)
                        .as_bytes(),
                ) == *hash
        })
}

/// Prove and sign `prepared` with the account's spending key, after its
/// proposal decodes against the database to exactly what was reviewed.
/// Returns each transaction's id and bytes, in the order they are sent.
pub(crate) fn sign(
    db: &mut ZcashDb,
    network: &Network,
    seed: &[u8],
    account_index: u32,
    prepared: &PreparedZcashShielded,
    sapling: Option<&SaplingParameters>,
) -> Result<Vec<([u8; 32], Vec<u8>)>, SendError> {
    let bytes = hex::decode(&prepared.proposal_hex).map_err(SendError::invalid)?;
    let encoded = zcash_client_backend::proto::proposal::Proposal::decode(&bytes[..])
        .map_err(|_| SendError::invalid("The reviewed proposal no longer decodes"))?;
    let proposal = encoded
        .try_into_standard_proposal(network, db)
        .map_err(|error| {
            SendError::Invalid(crate::LocalizableMessage::new(
                "The reviewed notes changed; build and review again (%@)",
                [error],
            ))
        })?;
    // The proposal says exactly what was reviewed, or nothing is signed.
    let mut decoded = review(network, &proposal)?;
    decoded.proposal_hex = prepared.proposal_hex.clone();
    if decoded != *prepared {
        return Err(SendError::invalid(
            "The shielded transaction changed; build and review again",
        ));
    }
    let account = zip32::AccountId::try_from(account_index)
        .map_err(|_| SendError::invalid("Invalid Zcash account"))?;
    let usk = UnifiedSpendingKey::from_seed(network, seed, account)
        .map_err(|error| SendError::Internal(format!("Zcash key derivation: {error:?}")))?;
    let keys = SpendingKeys::from_unified_spending_key(usk);
    let txids = match sapling {
        Some(parameters) => {
            let prover = zcash_proofs::prover::LocalTxProver::from_bytes(
                &parameters.spend,
                &parameters.output,
            );
            create_proposed_transactions::<
                _,
                _,
                std::convert::Infallible,
                _,
                std::convert::Infallible,
                _,
            >(
                db,
                network,
                &prover,
                &prover,
                &keys,
                OvkPolicy::Sender,
                &proposal,
                None,
            )
        }
        None => {
            if prepared.uses_sapling {
                return Err(SendError::invalid(
                    "A Sapling transaction needs the Sapling parameters",
                ));
            }
            create_proposed_transactions::<
                _,
                _,
                std::convert::Infallible,
                _,
                std::convert::Infallible,
                _,
            >(
                db,
                network,
                &NoSapling,
                &NoSapling,
                &keys,
                OvkPolicy::Sender,
                &proposal,
                None,
            )
        }
    }
    .map_err(|error| SendError::Internal(format!("Zcash transaction: {error}")))?;
    let mut transactions = Vec::new();
    for txid in txids {
        let tx = db
            .get_transaction(txid)
            .map_err(failure)?
            .ok_or_else(|| SendError::Internal("signed transaction not stored".into()))?;
        let mut raw = Vec::new();
        tx.write(&mut raw).map_err(failure)?;
        transactions.push((*txid.as_ref(), raw));
    }
    Ok(transactions)
}

/// The prover for a transaction with no Sapling part. It is never asked:
/// a proposal touching Sapling is proved with the parameters.
struct NoSapling;

impl sapling_crypto::prover::SpendProver for NoSapling {
    type Proof = ();
    fn prepare_circuit(
        _: sapling_crypto::ProofGenerationKey,
        _: sapling_crypto::Diversifier,
        _: sapling_crypto::Rseed,
        _: sapling_crypto::value::NoteValue,
        _: jubjub::Fr,
        _: sapling_crypto::value::ValueCommitTrapdoor,
        _: bls12_381::Scalar,
        _: sapling_crypto::MerklePath,
    ) -> Option<sapling_crypto::circuit::Spend> {
        None
    }
    fn create_proof<R: rand::RngCore>(&self, _: sapling_crypto::circuit::Spend, _: &mut R) {
        unreachable!("a Sapling spend is proved with the Sapling parameters")
    }
    fn encode_proof(_: ()) -> sapling_crypto::bundle::GrothProofBytes {
        unreachable!("a Sapling spend is proved with the Sapling parameters")
    }
}

impl sapling_crypto::prover::OutputProver for NoSapling {
    type Proof = ();
    fn prepare_circuit(
        _: &sapling_crypto::keys::EphemeralSecretKey,
        _: sapling_crypto::PaymentAddress,
        _: jubjub::Fr,
        _: sapling_crypto::value::NoteValue,
        _: sapling_crypto::value::ValueCommitTrapdoor,
    ) -> sapling_crypto::circuit::Output {
        unreachable!("a Sapling output is proved with the Sapling parameters")
    }
    fn create_proof<R: rand::RngCore>(&self, _: sapling_crypto::circuit::Output, _: &mut R) {
        unreachable!("a Sapling output is proved with the Sapling parameters")
    }
    fn encode_proof(_: ()) -> sapling_crypto::bundle::GrothProofBytes {
        unreachable!("a Sapling output is proved with the Sapling parameters")
    }
}

#[cfg(test)]
#[path = "tests/zcash_shielded.rs"]
mod tests;

//! Closing an XRP or Stellar account to recover its reserve: XRP's
//! `AccountDelete` and Stellar's `AccountMerge` remove the account and send
//! everything it holds, less the fee, to another existing account.
//!
//! Both are irreversible and both fail on the network, fee charged, when a
//! prerequisite is missing. Core reads every prerequisite the network
//! enforces and refuses before building, and reads them again before
//! signing; the artifact names the destination, the reserve freed, the
//! objects deleted with the account and the fee, bound into the review
//! digest as its [`WalletOperation::CloseAccount`]. The transaction then
//! goes through the ordinary send stages.

use super::*;
use crate::api::horizon::StellarMergeState;
use crate::api::xrpl_json_rpc::XrpDeletionState;
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};

/// An XRP account whose `Sequence` is within this many ledgers of the
/// current one cannot be deleted, so a recreated account cannot replay.
const XRP_DELETION_LEDGER_GAP: u64 = 256;
/// The most objects XRP deletes with an account; more fails `tefTOO_BIG`.
const XRP_DELETABLE_OBJECTS: u64 = 1000;
/// `lsfRequireDestTag`: payments to the account need a destination tag.
const XRP_REQUIRE_DEST_TAG: u32 = 0x0002_0000;
/// `lsfDepositAuth`: the account takes only payments it has authorized.
const XRP_DEPOSIT_AUTH: u32 = 0x0100_0000;

/// Whether core closes accounts on `chain`.
pub(crate) fn closes_accounts(chain: Chain) -> bool {
    matches!(chain.mainnet_counterpart(), Chain::Xrp | Chain::Stellar)
}

/// The deletion an XRP account allows, or the network's reason to refuse
/// it: `(balance, sequence, objects deleted with it, total reserve, fee)`,
/// in drops.
fn xrp_deletion(
    state: &XrpDeletionState,
    destination: &str,
) -> Result<(u64, u32, u64, u128, u64), SpectraBridgeError> {
    let Some(source) = &state.source else {
        return Err(SpectraBridgeError::invalid(
            "This account is not on the network; there is nothing to close.",
        ));
    };
    if source.blockers > 0 {
        return Err(SpectraBridgeError::invalid(
            "The account still has trust lines, escrows, payment channels or checks. Remove them first.",
        ));
    }
    if source.owner_count > XRP_DELETABLE_OBJECTS {
        return Err(SpectraBridgeError::refused(
            "The account owns %@ objects; the network deletes at most 1000 with it. Remove some first.",
            [source.owner_count],
        ));
    }
    let ledger = u64::from(state.ledger_index);
    let ready_at = (u64::from(source.sequence) + XRP_DELETION_LEDGER_GAP)
        .max(source.minted_nft_sequence.unwrap_or(0) + XRP_DELETION_LEDGER_GAP);
    if ready_at > ledger {
        return Err(SpectraBridgeError::refused(
            "The account is too recent to delete. Try again after ledger %@.",
            [ready_at],
        ));
    }
    match state.destination_flags {
        None => {
            return Err(SpectraBridgeError::refused(
                "%@ is not on the network. Close into an account that exists.",
                [destination],
            ));
        }
        Some(flags) if flags & XRP_REQUIRE_DEST_TAG != 0 => {
            return Err(SpectraBridgeError::refused(
                "%@ requires a destination tag, which Spectra does not send. Close into another account.",
                [destination],
            ));
        }
        Some(flags) if flags & XRP_DEPOSIT_AUTH != 0 => {
            return Err(SpectraBridgeError::refused(
                "%@ accepts only payments it has authorized. Close into another account.",
                [destination],
            ));
        }
        Some(_) => {}
    }
    // The network charges one owner reserve to delete an account.
    let fee = state.reserve_increment;
    if source.balance_drops <= fee {
        return Err(SpectraBridgeError::invalid(
            "The account's balance does not cover the fee to close it.",
        ));
    }
    let reserve = u128::from(state.reserve_base)
        + u128::from(source.owner_count) * u128::from(state.reserve_increment);
    Ok((
        source.balance_drops,
        source.sequence,
        source.owner_count,
        reserve,
        fee,
    ))
}

/// The merge a Stellar account allows, or the network's reason to refuse
/// it: `(balance, the merge's sequence, total reserve)`, in stroops.
fn stellar_merge(
    state: &StellarMergeState,
    destination: &str,
) -> Result<(u64, u64, u128), SpectraBridgeError> {
    let Some(source) = &state.source else {
        return Err(SpectraBridgeError::invalid(
            "This account is not on the network; there is nothing to close.",
        ));
    };
    if source.subentries > 0 {
        return Err(SpectraBridgeError::invalid(
            "The account still has trust lines, offers, data entries or extra signers. Remove them first.",
        ));
    }
    if source.sponsoring > 0 {
        return Err(SpectraBridgeError::invalid(
            "The account sponsors reserves for other accounts. End those sponsorships first.",
        ));
    }
    if source.auth_immutable {
        return Err(SpectraBridgeError::invalid(
            "The account's authorization flags are immutable, so the network never lets it merge.",
        ));
    }
    let sequence = source
        .sequence
        .checked_add(1)
        .ok_or_else(|| SpectraBridgeError::failure("Sequence exhausted"))?;
    // The merge's sequence must stay below the next ledger's starting
    // sequence, so a recreated account cannot replay it.
    if sequence >= u64::from(state.ledger) << 32 {
        return Err(SpectraBridgeError::invalid(
            "The account's sequence number is ahead of the ledger; it can merge only in a later ledger.",
        ));
    }
    match state.destination_memo_required {
        None => {
            return Err(SpectraBridgeError::refused(
                "%@ is not on the network. Close into an account that exists.",
                [destination],
            ));
        }
        Some(true) => {
            return Err(SpectraBridgeError::refused(
                "%@ requires a memo, which Spectra does not send. Close into another account.",
                [destination],
            ));
        }
        Some(false) => {}
    }
    let reserve = (2 + u128::from(source.subentries) + u128::from(source.sponsoring))
        .saturating_sub(u128::from(source.sponsored))
        * u128::from(state.base_reserve);
    Ok((source.balance_stroops, sequence, reserve))
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Build the transaction that closes the wallet's XRP or Stellar account
    /// into `destination`, an existing account: XRP `AccountDelete` or
    /// Stellar `AccountMerge`, prepared and stored like any send, to be
    /// signed and broadcast through the same stages. Refuses whatever the
    /// network would refuse, before anything is built.
    pub async fn build_account_closing(
        &self,
        wallet_id: String,
        destination: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let wallet = this.stored_wallet(&wallet_id).await?;
            let chain = wallet.chain_id;
            if !closes_accounts(chain) {
                return Err(SpectraBridgeError::refused(
                    "A %@ account cannot be closed.",
                    [chain.chain_display_name()],
                ));
            }
            if wallet.is_watch_only() {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
            let sender = wallet
                .address_on(chain)
                .ok_or_else(|| {
                    SpectraBridgeError::failure("Wallet has no address on this network")
                })?
                .to_string();
            let destination = destination.trim().to_string();
            if !crate::send::flow::is_valid_send_address(chain, destination.clone()) {
                return Err(SpectraBridgeError::refused(
                    "Not an address on %@: %@",
                    [chain.chain_display_name(), destination.as_str()],
                ));
            }
            if destination == sender {
                return Err(SpectraBridgeError::invalid(
                    "An account cannot be closed into itself.",
                ));
            }
            let decimals = u32::from(chain.native_decimals());
            let coin = |units: u128| crate::decimal::from_units(units, decimals);
            let (prepared, balance, fee, reserve, removed_objects) =
                this.account_closing(chain, &sender, &destination).await?;
            let network_fee = coin(u128::from(fee));
            let operation = WalletOperation::CloseAccount {
                destination: destination.clone(),
                reserve: coin(reserve),
                removed_objects,
                network_fee: network_fee.clone(),
            };
            let amount = coin(u128::from(balance - fee));
            let request = crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: wallet_id.clone(),
                password: None,
                to_address: destination.clone(),
                amount_str: amount.clone(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: Some(network_fee),
                evm_overrides: None,
                sign_only: false,
            };
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id,
                    chain_id: chain,
                    sender,
                    recipient: destination,
                    amount,
                    asset: chain.coin_symbol().into(),
                    symbol: chain.coin_symbol().into(),
                    staking: None,
                    operation: Some(operation),
                    created_at: crate::store::now_unix().floor(),
                    review_digest: String::new(),
                    review: SendArtifactReview::default(),
                    prepared_details: serde_json::to_string_pretty(&prepared)?,
                    signing_payload_hex: String::new(),
                    signed_payload: None,
                    transaction_hash: None,
                    attempts: Vec::new(),
                    selected_endpoints: Vec::new(),
                },
                request,
                prepared,
                submission: None,
                signed_digest: None,
                substrate_verified_through: None,
                icp_staking_receipts: vec![],
            };
            stored.view.review_digest = stored.digest()?;
            this.save_send_artifact(&stored, Vec::new()).await?;
            Ok(stored.view)
        })
        .await
    }
}

impl WalletService {
    /// Read and check what closing `sender` into `destination` depends on:
    /// the prepared transaction, the balance, the fee and the reserve in the
    /// coin's smallest unit, and the objects deleted with the account.
    pub(super) async fn account_closing(
        &self,
        chain: Chain,
        sender: &str,
        destination: &str,
    ) -> Result<(PreparedPayload, u64, u64, u128, u64), SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(match chain.mainnet_counterpart() {
            Chain::Xrp => {
                let state = XrplClient::new(endpoints)
                    .fetch_deletion_state(chain, sender, destination)
                    .await?;
                let (balance, sequence, objects, reserve, fee_drops) =
                    xrp_deletion(&state, destination)?;
                crate::send::xrp::validate_drops(u128::from(fee_drops))?;
                (
                    PreparedPayload::XrpAccountDelete {
                        sequence,
                        fee_drops,
                    },
                    balance,
                    fee_drops,
                    reserve,
                    objects,
                )
            }
            Chain::Stellar => {
                let state = HorizonClient::new(endpoints.clone())
                    .fetch_merge_state(chain, sender, destination)
                    .await?;
                let (balance, sequence, reserve) = stellar_merge(&state, destination)?;
                let fee_stroops =
                    HorizonClient::new(self.endpoints_for(chain, &[EndpointCapability::Fee]).await)
                        .fetch_base_fee()
                        .await?;
                if balance <= fee_stroops {
                    return Err(SpectraBridgeError::invalid(
                        "The account's balance does not cover the fee to close it.",
                    ));
                }
                (
                    PreparedPayload::StellarAccountMerge {
                        sequence,
                        fee_stroops,
                    },
                    balance,
                    fee_stroops,
                    reserve,
                    0,
                )
            }
            _ => {
                return Err(SpectraBridgeError::refused(
                    "A %@ account cannot be closed.",
                    [chain.chain_display_name()],
                ));
            }
        })
    }
}

#[cfg(test)]
#[path = "tests/wallet_closing.rs"]
mod tests;

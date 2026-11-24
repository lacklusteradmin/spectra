//! Algorand: building, revalidating, signing and expiring a payment of
//! ALGO, its send preview and its status.
use std::sync::Arc;

use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::api::algod::{AlgodClient, AlgorandAccount};
use crate::api::algorand_indexer::AlgorandIndexerClient;
use crate::api::transaction_status::TransactionStatus;
use crate::send::algorand::{AlgorandKey, AlgorandPaymentTerms};
use crate::send::payload::SignedPayload;
use crate::send::preview_types::SendPreview;
use crate::send::stages::*;

/// Refuse an account that has been rekeyed: the network takes only its
/// authorized address's signature, which the wallet's key is not.
fn require_own_signer(chain: Chain, account: &AlgorandAccount) -> Result<(), SpectraBridgeError> {
    match &account.auth_address {
        Some(auth) => Err(SpectraBridgeError::failed(
            "This %@ account is rekeyed to %@, whose key signs for it instead of this wallet's.",
            [chain.chain_display_name(), auth.as_str()],
        )),
        None => Ok(()),
    }
}

/// Refuse a payment that leaves its recipient below its minimum balance,
/// which the network refuses: an address holding nothing must receive at
/// least 0.1 ALGO to exist.
fn require_receiver_minimum(
    chain: Chain,
    receiver: &AlgorandAccount,
    amount: u128,
) -> Result<(), SpectraBridgeError> {
    if u128::from(receiver.amount).saturating_add(amount) < u128::from(receiver.min_balance) {
        let units = |value: u64| {
            crate::amount::decimal::from_units(
                u128::from(value),
                u32::from(chain.native_decimals()),
            )
        };
        return Err(SpectraBridgeError::failed(
            "The recipient must hold at least %@ %@ after this payment; it holds %@ %@.",
            [
                units(receiver.min_balance).as_str(),
                chain.coin_symbol(),
                units(receiver.amount).as_str(),
                chain.coin_symbol(),
            ],
        ));
    }
    Ok(())
}

/// The amount plus the fee, refused unless the sender's ALGO above its
/// minimum balance covers it.
fn require_funds(
    chain: Chain,
    sender: &AlgorandAccount,
    amount: u128,
    fee: u64,
) -> Result<(), SpectraBridgeError> {
    let required = amount
        .checked_add(u128::from(fee))
        .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
    if u128::from(sender.spendable()) < required {
        let decimals = u32::from(chain.native_decimals());
        let symbol = chain.coin_symbol();
        return Err(SpectraBridgeError::failed(
            "Insufficient %@ for the amount plus the network fee, above the %@ %@ the account must keep (requires %@ %@).",
            [
                symbol,
                crate::amount::decimal::from_units(u128::from(sender.min_balance), decimals)
                    .as_str(),
                symbol,
                crate::amount::decimal::from_units(required, decimals).as_str(),
                symbol,
            ],
        ));
    }
    Ok(())
}

impl WalletService {
    async fn algod_client(&self, chain: Chain, needs: &[EndpointCapability]) -> AlgodClient {
        AlgodClient::new(self.endpoints_for(chain, needs).await)
    }

    pub(super) async fn prepare_algorand(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        if request.contract_address.is_some() {
            return Err(SpectraBridgeError::failure(
                "Algorand sends carry ALGO only; Algorand Standard Assets are not supported",
            ));
        }
        if request.memo.is_some() {
            return Err(SpectraBridgeError::failure(
                "An Algorand payment carries no memo",
            ));
        }
        let client = AlgodClient::new(Arc::clone(&staged.eps));
        let params = client.fetch_params(chain).await?;
        let account = client.fetch_account(sender).await?;
        require_own_signer(chain, &account)?;
        let to = request.to_address.clone();
        let terms = |fee: u64| AlgorandPaymentTerms {
            sender,
            receiver: &to,
            amount: staged.amount,
            fee,
            first_valid: params.last_round,
            last_valid: params.last_round + crate::registry::ALGORAND_MAX_TXN_LIFE,
        };
        let mut fee =
            crate::send::algorand::fee_for(chain, terms(1), params.fee_per_byte, params.min_fee)?;
        // A reviewed fee higher than the network asks is kept: it is what
        // the person agreed to pay.
        if let Some(reviewed) = request.fee_amount.as_deref() {
            let reviewed =
                crate::send::payload::fee_units(reviewed, u32::from(chain.native_decimals()))?;
            fee = fee.max(reviewed);
        }
        let prepared = crate::send::algorand::prepare_payment(chain, terms(fee))?;
        if prepared.receiver != prepared.sender {
            require_receiver_minimum(
                chain,
                &client.fetch_account(&prepared.receiver).await?,
                prepared.amount,
            )?;
        }
        require_funds(chain, &account, prepared.amount, fee)?;
        request.fee_amount = Some(crate::amount::decimal::from_units(
            u128::from(fee),
            u32::from(chain.native_decimals()),
        ));
        Ok(PreparedPayload::Algorand(prepared))
    }

    /// Recheck a prepared payment against the chain: the same network, a
    /// last valid round still ahead, a sender its key signs for with the
    /// ALGO to pay, and a recipient the payment leaves above its minimum.
    pub(super) async fn validate_algorand_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Algorand(prepared) = &stored.prepared else {
            return Ok(());
        };
        let network = chain.algorand_network()?;
        if prepared.genesis_id != network.genesis_id
            || prepared.genesis_hash != network.genesis_hash
        {
            return Err(SpectraBridgeError::failure(
                "Algorand transaction was prepared for another network",
            ));
        }
        let client = self
            .algod_client(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await;
        if client.fetch_params(chain).await?.last_round >= prepared.last_valid {
            return Err(SpectraBridgeError::failure(
                "Algorand transaction expired; build and review again",
            ));
        }
        let account = client.fetch_account(&prepared.sender).await?;
        require_own_signer(chain, &account)?;
        require_funds(chain, &account, prepared.amount, prepared.fee)?;
        if prepared.receiver != prepared.sender {
            require_receiver_minimum(
                chain,
                &client.fetch_account(&prepared.receiver).await?,
                prepared.amount,
            )?;
        }
        Ok(())
    }

    pub(super) async fn sign_staged_algorand(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let PreparedPayload::Algorand(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::failure("Not an Algorand transaction"));
        };
        let key = AlgorandKey::from_bytes(&signer.private_key()?)?;
        let (signed, txid) = prepared.sign(chain, &key)?;
        Ok((SignedPayload::Hex(hex::encode(signed)), Some(txid)))
    }

    /// Whether a signed Algorand payment can no longer be committed: the
    /// network is at or past its last valid round.
    pub(super) async fn algorand_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let PreparedPayload::Algorand(prepared) = &stored.prepared else {
            return Ok(false);
        };
        Ok(self
            .algod_client(chain, &[EndpointCapability::Verification])
            .await
            .fetch_params(chain)
            .await?
            .last_round
            >= prepared.last_valid)
    }

    /// Whether `txid` has been committed: from a verified node, which
    /// remembers its pool and last 1,000 rounds, then from an indexer for
    /// an older one. Unknown to both, it is still pending.
    pub(super) async fn algorand_transaction_status(
        &self,
        chain: Chain,
        txid: &str,
    ) -> Result<TransactionStatus, SpectraBridgeError> {
        let client = self
            .algod_client(chain, &[EndpointCapability::Verification])
            .await;
        client.verify_network(chain).await?;
        if let Some(status) = client.fetch_transaction_status(txid).await? {
            return Ok(status);
        }
        let indexers = self
            .api_endpoints(
                chain,
                crate::api::endpoint::EndpointApi::AlgorandIndexer,
                &[EndpointCapability::History],
            )
            .await?;
        Ok(
            match AlgorandIndexerClient::new(Arc::new(indexers))
                .fetch_confirmed_round(chain, txid)
                .await?
            {
                Some(round) => TransactionStatus::Confirmed {
                    succeeded: true,
                    block: Some(round),
                },
                None => TransactionStatus::Pending,
            },
        )
    }

    /// An Algorand send's quote: the fee a payment from `address` pays now,
    /// and what it can send above the minimum balance it must keep.
    pub(super) async fn preview_algorand_send(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let client = self
            .algod_client(
                chain,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        let params = client.fetch_params(chain).await?;
        let account = client.fetch_account(address).await?;
        let fee = crate::send::algorand::fee_for(
            chain,
            AlgorandPaymentTerms {
                sender: address,
                receiver: address,
                amount: u128::from(u64::MAX),
                fee: 1,
                first_valid: params.last_round,
                last_valid: params.last_round + crate::registry::ALGORAND_MAX_TXN_LIFE,
            },
            params.fee_per_byte,
            params.min_fee,
        )?;
        let units = |value: u64| {
            crate::amount::decimal::from_units(
                u128::from(value),
                u32::from(chain.native_decimals()),
            )
        };
        let spendable = account.spendable();
        Ok(SendPreview {
            estimated_network_fee: units(fee),
            spendable_balance: units(spendable),
            max_sendable: units(spendable.saturating_sub(fee)),
            fee_rate_description: Some(
                "Network minimum fee; the account's minimum balance is kept".into(),
            ),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "tests/send_stage_algorand.rs"]
mod tests;

//! Tron: building, revalidating, signing and expiring its staged
//! transactions.
use super::protocols::{StagedAmount, protocol_u64};
use super::*;
use crate::send::payload::SignedPayload;

impl WalletService {
    /// Recheck a prepared Tron transaction against the chain.
    pub(super) async fn validate_tron_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        match &stored.prepared {
            PreparedPayload::Tron(prepared)
                if stored
                    .request
                    .contract_address
                    .as_deref()
                    .is_some_and(|id| {
                        chain.token_standard_for_identifier(id)
                            == Some(crate::registry::TokenStandard::Trc10)
                    }) =>
            {
                self.validate_tron_permission(chain, prepared, &stored.view.sender)
                    .await?;
                let fee = self
                    .validate_trc10_funds(chain, prepared, &stored.request, &stored.view.sender)
                    .await?;
                let reviewed = stored
                    .request
                    .fee_amount
                    .as_deref()
                    .ok_or_else(|| SpectraBridgeError::invalid("Missing TRC-10 fee budget"))?;
                if crate::send::payload::fee_units(reviewed, u32::from(chain.native_decimals()))?
                    < fee
                {
                    return Err(SpectraBridgeError::failure(
                        "TRC-10 fee budget changed; review again",
                    ));
                }
            }
            // The account's permissions again: a key taken out of them since
            // the review would be refused only at broadcast.
            PreparedPayload::Tron(prepared) => {
                self.validate_tron_permission(chain, prepared, &stored.view.sender)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn prepare_tron(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount { eps, amount, .. } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Tron => {
                use crate::send::tron::{Transfer, prepare_transfer_under};
                let trc10 = request.contract_address.as_deref().is_some_and(|id| {
                    chain.token_standard_for_identifier(id)
                        == Some(crate::registry::TokenStandard::Trc10)
                });
                let transfer = match request.contract_address.as_deref() {
                    Some(asset_id)
                        if chain.token_standard_for_identifier(asset_id)
                            == Some(crate::registry::TokenStandard::Trc10) =>
                    {
                        Transfer::Trc10 {
                            asset_id,
                            to,
                            amount: protocol_u64(amount)?,
                        }
                    }
                    Some(contract) => Transfer::Token {
                        contract,
                        to,
                        amount,
                        fee_limit: crate::send::tron::TRC20_FEE_LIMIT_SUN,
                    },
                    None => Transfer::Native {
                        to,
                        amount: protocol_u64(amount)?,
                    },
                };
                let client = TronHttpClient::new(eps);
                let reference = if trc10 {
                    client.transfer_reference_for(chain).await?
                } else {
                    client.transfer_reference().await?
                };
                let contract_type = match &transfer {
                    Transfer::Native { .. } => crate::send::tron_multisig::TRANSFER_CONTRACT,
                    Transfer::Trc10 { .. } => crate::send::tron_multisig::TRANSFER_ASSET_CONTRACT,
                    Transfer::Token { .. } => crate::send::tron_multisig::TRIGGER_SMART_CONTRACT,
                };
                let permission = self
                    .tron_permission_alone(chain, sender, contract_type)
                    .await?;
                let prepared =
                    prepare_transfer_under(sender, transfer, reference, permission, 60_000)?;
                if trc10 {
                    let fee = self
                        .validate_trc10_funds(chain, &prepared, request, sender)
                        .await?;
                    request.fee_amount = Some(crate::amount::decimal::from_units(
                        u128::from(fee),
                        u32::from(chain.native_decimals()),
                    ));
                }
                PreparedPayload::Tron(prepared)
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Tron network"));
            }
        })
    }

    pub(super) async fn sign_staged_tron(
        &self,
        stored: &StoredSend,
        signer: &crate::service::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        Ok(match &stored.prepared {
            PreparedPayload::Tron(p) => {
                if p.expiration() <= (crate::store::now_unix() * 1000.0) as u64 {
                    return Err(SpectraBridgeError::failure(
                        "Tron transaction expired; build and review again",
                    ));
                }
                let key = signer.private_key()?;
                (
                    SignedPayload::Hex(hex::encode(p.sign(&key)?)),
                    Some(p.txid()),
                )
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Tron transaction"));
            }
        })
    }

    /// Whether a signed Tron transaction can no longer land.
    pub(super) async fn tron_signature_expired(
        &self,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let now = crate::store::now_unix();
        Ok(match &stored.prepared {
            PreparedPayload::Tron(p) => now * 1000.0 >= p.expiration() as f64,
            _ => false,
        })
    }

    async fn validate_trc10_funds(
        &self,
        chain: Chain,
        prepared: &crate::send::tron::PreparedTronTransfer,
        request: &crate::send::SendExecutionRequest,
        sender: &str,
    ) -> Result<u64, SpectraBridgeError> {
        let id = request
            .contract_address
            .as_deref()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing TRC-10 ID"))?;
        let decimals = request
            .token_decimals
            .ok_or_else(|| SpectraBridgeError::invalid("Missing TRC-10 precision"))?;
        let amount = crate::amount::input::parse_raw_amount(&request.amount_str, decimals)?;
        let amount = u64::try_from(amount)
            .ok()
            .filter(|n| *n > 0 && *n <= i64::MAX as u64)
            .ok_or_else(|| {
                SpectraBridgeError::invalid("TRC-10 amount exceeds positive int64 protocol range")
            })?;
        // The reviewed transfer, encoded under the same reference, is the
        // transaction's bytes or another transaction.
        let reviewed = crate::send::tron::encode(
            sender,
            crate::send::tron::Transfer::Trc10 {
                asset_id: id,
                to: &request.to_address,
                amount,
            },
            &prepared.reference,
        )?;
        if reviewed.raw != prepared.raw {
            return Err(SpectraBridgeError::invalid(
                "TRC-10 transaction differs from the reviewed transfer",
            ));
        }
        if prepared.expiration() <= (crate::store::now_unix() * 1000.0) as u64 {
            return Err(SpectraBridgeError::failure(
                "Tron transaction expired; build and review again",
            ));
        }
        let state = TronHttpClient::new(
            self.endpoints_for(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::TokenBalance,
                    EndpointCapability::Fee,
                ],
            )
            .await,
        )
        .fetch_trc10_transfer_state(
            chain,
            id,
            sender,
            Some(&request.to_address),
            prepared.bandwidth_bytes()?,
        )
        .await?;
        if u32::from(state.token.metadata.decimals) != decimals {
            return Err(SpectraBridgeError::failure(
                "TRC-10 precision changed; review again",
            ));
        }
        if state.token.balance_raw < amount {
            return Err(SpectraBridgeError::failure(
                "Insufficient TRC-10 token balance",
            ));
        }
        if state
            .recipient_balance
            .checked_add(amount)
            .is_none_or(|sum| sum > i64::MAX as u64)
        {
            return Err(SpectraBridgeError::failure(
                "TRC-10 recipient balance would exceed the protocol range",
            ));
        }
        if state.native_balance < state.fee_budget_sun {
            return Err(SpectraBridgeError::failure(
                "Insufficient TRX for the TRC-10 bandwidth and account activation budget",
            ));
        }
        Ok(state.fee_budget_sun)
    }
}

#[cfg(test)]
#[path = "../tests/send_stage_tron.rs"]
mod tests;

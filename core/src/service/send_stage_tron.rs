//! Tron: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;

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
                    .is_some_and(|id| chain.token_standard_for_identifier(id) == "TRC-10") =>
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
        let StagedAmount {
            eps,
            amount,
            amount_u64,
            ..
        } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Tron => {
                use crate::send::tron::{Transfer, prepare_transfer_under};
                let trc10 = request
                    .contract_address
                    .as_deref()
                    .is_some_and(|id| chain.token_standard_for_identifier(id) == "TRC-10");
                let transfer = match request.contract_address.as_deref() {
                    Some(asset_id) if chain.token_standard_for_identifier(asset_id) == "TRC-10" => {
                        Transfer::Trc10 {
                            asset_id,
                            to,
                            amount: amount_u64,
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
                        amount: amount_u64,
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
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        Ok(match &stored.prepared {
            PreparedPayload::Tron(p) => {
                let expiry = p.body["raw_data"]["expiration"]
                    .as_u64()
                    .ok_or_else(|| SpectraBridgeError::failure("Invalid Tron expiration"))?;
                if expiry <= (crate::store::now_unix() * 1000.0) as u64 {
                    return Err(SpectraBridgeError::failure(
                        "Tron transaction expired; build and review again",
                    ));
                }
                let key = decode_private_key(&signer.private_key_hex)?;
                (
                    SignedPayload::Json(p.clone().sign(&key)?),
                    p.body["txID"].as_str().map(str::to_string),
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
            PreparedPayload::Tron(p) => p.body["raw_data"]["expiration"]
                .as_u64()
                .is_none_or(|expiry| now * 1000.0 >= expiry as f64),
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
        let contract = &prepared.body["raw_data"]["contract"][0];
        let value = &contract["parameter"]["value"];
        let owner = format!(
            "41{}",
            crate::derivation::tron::tron_base58_to_evm_hex(sender)?
        );
        let receiver = format!(
            "41{}",
            crate::derivation::tron::tron_base58_to_evm_hex(&request.to_address)?
        );
        if contract["type"].as_str() != Some("TransferAssetContract")
            || value["asset_name"].as_str() != Some(hex::encode(id).as_str())
            || value["owner_address"].as_str() != Some(owner.as_str())
            || value["to_address"].as_str() != Some(receiver.as_str())
            || value["amount"].as_u64() != Some(amount)
        {
            return Err(SpectraBridgeError::invalid(
                "TRC-10 transaction differs from the reviewed transfer",
            ));
        }
        let expiry = prepared.body["raw_data"]["expiration"]
            .as_u64()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing Tron expiration"))?;
        if expiry <= (crate::store::now_unix() * 1000.0) as u64 {
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

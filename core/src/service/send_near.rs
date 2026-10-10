//! NEAR native, NEP-141 and staking share protocol admission-fee accounting.
use super::*;
use crate::send::stages::{PreparedPayload, StoredSend};

pub(super) struct NearSendQuote {
    pub budget: u128,
    pub spendable: u128,
    pub token_balance: Option<u128>,
    /// The deposit that registers the recipient with the token first, when
    /// the token has not registered it (NEP-145).
    pub registration: Option<u128>,
}

impl NearSendQuote {
    /// The NEAR a send of `amount` (NEAR when `native`) takes from the
    /// account: the fee budget, the recipient's registration and the amount.
    pub(super) fn required(&self, amount: u128, native: bool) -> Result<u128, SpectraBridgeError> {
        self.budget
            .checked_add(self.registration.unwrap_or(0))
            .and_then(|sum| sum.checked_add(if native { amount } else { 0 }))
            .ok_or_else(|| SpectraBridgeError::invalid("NEAR amount and fee overflow"))
    }
}

impl WalletService {
    pub(super) async fn near_send_quote(
        &self,
        chain: Chain,
        owner: &str,
        destination: &str,
        amount: u128,
        contract: Option<&str>,
    ) -> Result<NearSendQuote, SpectraBridgeError> {
        let endpoints = self
            .api_endpoints(
                chain,
                crate::EndpointApi::NearJsonRpc,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await?;
        crate::api::http::race(&endpoints, |endpoint| async move {
            let client = NearClient::new(Arc::new(vec![endpoint]));
            client.verify_network(chain).await?;
            let destination = if destination.is_empty() {
                "0000000000000000000000000000000000000000000000000000000000000000"
            } else {
                destination
            };
            let (budget, token_balance, registration) = if let Some(contract) = contract {
                let registration = client
                    .fetch_storage_registration(contract, destination)
                    .await?;
                let calls = crate::send::near::nep141_transfer_calls(
                    destination,
                    amount,
                    registration,
                    chain.near_token_gas_limit().unwrap(),
                )?;
                let fee = client
                    .function_calls_fee_budget(
                        owner,
                        contract,
                        &calls
                            .iter()
                            .map(|call| (call.method.len() + call.args.len(), call.gas))
                            .collect::<Vec<_>>(),
                    )
                    .await?;
                (
                    // ft_transfer's one attached yoctoNEAR.
                    fee.checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::invalid("NEAR fee overflow"))?,
                    Some(client.fetch_ft_balance_of(contract, owner).await?),
                    registration,
                )
            } else {
                (
                    client.transfer_fee_budget(owner, destination).await?,
                    None,
                    None,
                )
            };
            Ok(NearSendQuote {
                budget,
                spendable: client.fetch_spendable_balance(owner).await?,
                token_balance,
                registration,
            })
        })
        .await
    }

    pub(super) async fn preview_near_send(
        &self,
        chain: Chain,
        owner: &str,
        destination: &str,
        amount: u128,
        contract: Option<&str>,
    ) -> Result<crate::send::preview_types::NearSendPreview, SpectraBridgeError> {
        let quote = self
            .near_send_quote(chain, owner, destination, amount, contract)
            .await?;
        let decimal =
            |amount| crate::decimal::from_units(amount, u32::from(chain.native_decimals()));
        Ok(crate::send::preview_types::NearSendPreview {
            estimatedNetworkFee: decimal(quote.budget),
            feeBudgetYoctoNear: quote.budget.to_string(),
            spendableBalance: decimal(quote.spendable),
            maxSendable: decimal(quote.spendable.saturating_sub(quote.required(0, false)?)),
            feeRateDescription: Some("Protocol prepayment budget; storage stake retained".into()),
        })
    }

    pub(super) async fn validate_near_transfer_state(
        &self,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Near {
            amount,
            token_contract,
            fee_budget,
            registration_deposit,
            public_key,
            nonce,
            ..
        } = &stored.prepared
        else {
            return Err(SpectraBridgeError::invalid(
                "Missing prepared NEAR transfer",
            ));
        };
        let chain = stored.view.chain_id;
        if token_contract != &stored.request.contract_address {
            return Err(SpectraBridgeError::invalid(
                "NEAR token differs from reviewed transfer",
            ));
        }
        let decimals = stored
            .request
            .token_decimals
            .unwrap_or(u32::from(chain.native_decimals()));
        if crate::send::amount_input::parse_raw_amount(&stored.request.amount_str, decimals)?
            != *amount
        {
            return Err(SpectraBridgeError::invalid(
                "NEAR amount differs from reviewed transfer",
            ));
        }
        if let Some(contract) = token_contract {
            let actual = self.token_contract_decimals(chain, contract).await?;
            if actual != stored.request.token_decimals {
                return Err(SpectraBridgeError::invalid("NEAR token precision changed"));
            }
        }
        let quote = self
            .near_send_quote(
                chain,
                &stored.view.sender,
                &stored.view.recipient,
                *amount,
                token_contract.as_deref(),
            )
            .await?;
        let reviewed = fee_budget
            .parse::<u128>()
            .map_err(SpectraBridgeError::invalid)?;
        if stored
            .request
            .fee_amount
            .as_deref()
            .map(|fee| {
                crate::send::amount_input::parse_raw_amount(fee, u32::from(chain.native_decimals()))
            })
            .transpose()?
            != Some(reviewed)
        {
            return Err(SpectraBridgeError::invalid(
                "NEAR fee differs from reviewed transfer",
            ));
        }
        let client = NearClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        client.verify_network(chain).await?;
        if client
            .fetch_full_access_key_nonce(
                &stored.view.sender,
                &bs58::encode(public_key).into_string(),
            )
            .await?
            .checked_add(1)
            != Some(*nonce)
        {
            return Err(SpectraBridgeError::invalid(
                "NEAR access-key nonce changed; review again",
            ));
        }
        if quote.budget > reviewed {
            return Err(SpectraBridgeError::invalid(
                "NEAR fee exceeds reviewed budget; review again",
            ));
        }
        if quote.token_balance.is_some_and(|balance| balance < *amount) {
            return Err(SpectraBridgeError::invalid("Insufficient NEP-141 balance"));
        }
        // The recipient's registration, read again: registered or not, and
        // at what deposit, as reviewed.
        if quote.registration.map(|deposit| deposit.to_string()) != *registration_deposit {
            return Err(SpectraBridgeError::invalid(
                "The recipient's registration with the token changed; build and review again",
            ));
        }
        let required = quote.required(*amount, token_contract.is_none())?;
        if quote.spendable < required {
            return Err(SpectraBridgeError::invalid(
                "Insufficient spendable NEAR for amount, protocol fee and storage stake",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/send_near.rs"]
mod send_near_tests;

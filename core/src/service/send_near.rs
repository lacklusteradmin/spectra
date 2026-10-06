//! NEAR native, NEP-141 and staking share protocol admission-fee accounting.
use super::*;
use crate::send::stages::{PreparedPayload, StoredSend};

pub(super) struct NearSendQuote {
    pub budget: u128,
    pub spendable: u128,
    pub token_balance: Option<u128>,
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
            let (budget, token_balance) = if let Some(contract) = contract {
                let args = crate::send::near::nep141_transfer_args(destination, amount)?;
                let fee = client
                    .function_call_fee_budget(
                        owner,
                        contract,
                        "ft_transfer",
                        args.len(),
                        chain.near_token_gas_limit().unwrap(),
                    )
                    .await?;
                (
                    fee.checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::invalid("NEAR fee overflow"))?,
                    Some(client.fetch_ft_balance_of(contract, owner).await?),
                )
            } else {
                (client.transfer_fee_budget(owner, destination).await?, None)
            };
            Ok(NearSendQuote {
                budget,
                spendable: client.fetch_spendable_balance(owner).await?,
                token_balance,
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
            maxSendable: decimal(quote.spendable.saturating_sub(quote.budget)),
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
        let required = quote
            .budget
            .checked_add(if token_contract.is_none() { *amount } else { 0 })
            .ok_or_else(|| SpectraBridgeError::invalid("NEAR amount and fee overflow"))?;
        if quote.spendable < required {
            return Err(SpectraBridgeError::invalid(
                "Insufficient spendable NEAR for amount, protocol fee and storage stake",
            ));
        }
        Ok(())
    }
}

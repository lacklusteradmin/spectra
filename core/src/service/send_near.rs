//! NEAR: native, NEP-141 and staking sends share protocol admission-fee
//! accounting, and their staged transactions are built, revalidated, signed
//! and expired here.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::api::endpoint::EndpointApi;
use crate::send::payload::SignedPayload;
use crate::send::stages::{PreparedPayload, StoredSend};
use base64::{Engine, engine::general_purpose::STANDARD};

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
                EndpointApi::NearJsonRpc,
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
    ) -> Result<crate::send::preview_types::SendPreview, SpectraBridgeError> {
        let quote = self
            .near_send_quote(chain, owner, destination, amount, contract)
            .await?;
        let decimal =
            |amount| crate::amount::decimal::from_units(amount, u32::from(chain.native_decimals()));
        Ok(crate::send::preview_types::SendPreview {
            estimated_network_fee: decimal(quote.budget),
            spendable_balance: decimal(quote.spendable),
            max_sendable: decimal(quote.spendable.saturating_sub(quote.required(0, false)?)),
            fee_rate_description: Some("Protocol prepayment budget; storage stake retained".into()),
            ..Default::default()
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
        if crate::amount::input::parse_raw_amount(&stored.request.amount_str, decimals)? != *amount
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
                crate::amount::input::parse_raw_amount(fee, u32::from(chain.native_decimals()))
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

impl WalletService {
    /// Recheck a prepared NEAR transaction against the chain.
    pub(super) async fn validate_near_state(
        &self,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        match &stored.prepared {
            PreparedPayload::Near { .. } => self.validate_near_transfer_state(stored).await?,
            // Staking is validated apart; a NEAR call that is not staking is
            // a storage refund.
            PreparedPayload::NearFunctionCall(prepared) => {
                self.validate_token_storage_refund(stored, prepared).await?
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn prepare_near(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount { eps, amount, .. } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Near => {
                let client = NearClient::new(eps);
                client.verify_network(chain).await?;
                let quote = self
                    .near_send_quote(
                        chain,
                        sender,
                        to,
                        amount,
                        request.contract_address.as_deref(),
                    )
                    .await?;
                if let Some(reviewed) = &request.fee_amount
                    && crate::amount::input::parse_raw_amount(
                        reviewed,
                        u32::from(chain.native_decimals()),
                    )? < quote.budget
                {
                    return Err(SpectraBridgeError::invalid(
                        "NEAR fee exceeds reviewed budget; review again",
                    ));
                }
                let required = quote.required(amount, request.contract_address.is_none())?;
                if quote.spendable < required {
                    return Err(SpectraBridgeError::invalid(
                        "Insufficient spendable NEAR for amount, protocol fee and storage stake",
                    ));
                }
                if quote.token_balance.is_some_and(|balance| balance < amount) {
                    return Err(SpectraBridgeError::invalid("Insufficient NEP-141 balance"));
                }
                request.fee_amount = Some(crate::amount::decimal::from_units(
                    quote.budget,
                    u32::from(chain.native_decimals()),
                ));
                let public_key: [u8; 32] =
                    if sender.len() == 64 && sender.bytes().all(|b| b.is_ascii_hexdigit()) {
                        hex::decode(sender)?
                            .try_into()
                            .map_err(|_| SpectraBridgeError::failure("Invalid NEAR public key"))?
                    } else {
                        // A named account signs with the key recorded at import,
                        // whichever other full-access keys it has.
                        super::wallet_near_keys::near_signing_key(
                            &self.stored_wallet(&request.wallet_id).await?,
                            sender,
                        )?
                    };
                let nonce = client
                    .fetch_full_access_key_nonce(sender, &bs58::encode(public_key).into_string())
                    .await?
                    .checked_add(1)
                    .ok_or_else(|| SpectraBridgeError::failure("Nonce exhausted"))?;
                let block_hash = crate::derivation::solana::decode_b58_32(
                    &client.fetch_latest_block_hash().await?,
                )?;
                PreparedPayload::Near {
                    public_key,
                    nonce,
                    block_hash,
                    amount,
                    token_contract: request.contract_address.clone(),
                    fee_budget: quote.budget.to_string(),
                    registration_deposit: quote.registration.map(|deposit| deposit.to_string()),
                }
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a NEAR network"));
            }
        })
    }

    pub(super) async fn sign_staged_near(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let seed = || crate::send::keys::Ed25519Seed::from_hex(&signer.private_key_hex);
        Ok(match &stored.prepared {
            PreparedPayload::NearDeleteKey(p) => {
                if !NearClient::new(eps)
                    .transaction_block_is_valid(chain, &p.block_hash)
                    .await?
                {
                    return Err(SpectraBridgeError::invalid(
                        "NEAR block reference is stale; review again",
                    ));
                }
                let (raw, hash) = p.sign(&seed()?)?;
                (SignedPayload::Base64(STANDARD.encode(raw)), Some(hash))
            }
            PreparedPayload::NearFunctionCall(p) => {
                if !NearClient::new(eps)
                    .transaction_block_is_valid(chain, &p.block_hash)
                    .await?
                {
                    return Err(SpectraBridgeError::invalid(
                        "NEAR block reference is stale; review again",
                    ));
                }
                let (raw, hash) = p.sign(&seed()?)?;
                (SignedPayload::Base64(STANDARD.encode(raw)), Some(hash))
            }
            PreparedPayload::Near {
                public_key,
                nonce,
                block_hash,
                amount,
                token_contract,
                registration_deposit,
                ..
            } => {
                let client = NearClient::new(eps);
                if seed()?.public_key() != *public_key
                    || client
                        .fetch_full_access_key_nonce(
                            &stored.view.sender,
                            &bs58::encode(public_key).into_string(),
                        )
                        .await?
                        .checked_add(1)
                        != Some(*nonce)
                    || !client.transaction_block_is_valid(chain, block_hash).await?
                {
                    return Err(crate::error::SpectraBridgeError::failure(
                        "NEAR signer, nonce or block reference is stale; build and review again",
                    ));
                }
                let bytes = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                let key: &[u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid NEAR seed"))?;
                let raw = if let Some(contract) = token_contract {
                    crate::send::near::build_near_function_call_tx(
                        &stored.view.sender,
                        public_key,
                        *nonce,
                        contract,
                        &crate::send::near::nep141_transfer_calls(
                            &stored.view.recipient,
                            *amount,
                            registration_deposit
                                .as_deref()
                                .map(str::parse)
                                .transpose()
                                .map_err(SpectraBridgeError::invalid)?,
                            chain.near_token_gas_limit().unwrap(),
                        )?,
                        block_hash,
                        key,
                    )?
                } else {
                    crate::send::near::build_near_transfer_tx(
                        &stored.view.sender,
                        public_key,
                        *nonce,
                        &stored.view.recipient,
                        *amount,
                        block_hash,
                        key,
                    )?
                };
                let hash = crate::send::near::signed_transaction_hash(&raw)?;
                (SignedPayload::Base64(STANDARD.encode(raw)), Some(hash))
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a NEAR transaction"));
            }
        })
    }

    /// Whether a signed NEAR transaction can no longer land.
    pub(super) async fn near_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let Some(block_hash) = near_block_hash(&stored.prepared) else {
            return Ok(false);
        };
        Ok(!NearClient::new(endpoints)
            .transaction_block_is_valid(chain, block_hash)
            .await?)
    }
}

/// The block a NEAR transaction references, which expires it.
fn near_block_hash(prepared: &PreparedPayload) -> Option<&[u8; 32]> {
    match prepared {
        PreparedPayload::Near { block_hash, .. } => Some(block_hash),
        PreparedPayload::NearFunctionCall(p) => Some(&p.block_hash),
        PreparedPayload::NearDeleteKey(p) => Some(&p.block_hash),
        _ => None,
    }
}

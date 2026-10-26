//! TON: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;
use base64::{Engine, engine::general_purpose::STANDARD};

impl WalletService {
    /// Recheck a prepared TON transaction against the chain.
    pub(super) async fn validate_ton_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let now = crate::store::now_unix() as u64;
        let PreparedPayload::Ton {
            seqno,
            amount,
            valid_until,
            jetton,
        } = &stored.prepared
        else {
            return Ok(());
        };
        let client = ToncenterV2Client::new(
            self.endpoints_for(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await,
        );
        client.verify_network(chain).await?;
        if u64::from(*valid_until) <= now
            || client.fetch_seqno(&stored.view.sender).await? != *seqno
        {
            return Err(SpectraBridgeError::failure(
                "TON sequence or expiration changed; build and review again",
            ));
        }
        let value = if let Some(plan) = jetton {
            if stored.request.contract_address.as_deref() != Some(plan.master.as_str()) {
                return Err(SpectraBridgeError::invalid(
                    "Jetton master differs from the reviewed asset",
                ));
            }
            if self.token_contract_decimals(chain, &plan.master).await?
                != stored.request.token_decimals
            {
                return Err(SpectraBridgeError::failure(
                    "Jetton precision changed; review again",
                ));
            }
            if self
                .ton_transfer_wallet(chain, &stored.view.sender, &plan.master, *amount)
                .await?
                != plan.source_wallet
            {
                return Err(SpectraBridgeError::failure(
                    "Jetton source wallet changed; review again",
                ));
            }
            u128::from(plan.attached_nanotons)
        } else {
            *amount
        };
        let fee = chain
            .static_fee_units()
            .ok_or_else(|| SpectraBridgeError::failure("TON fee unavailable"))?;
        if u128::from(client.fetch_balance(&stored.view.sender).await?.nanotons) < value + fee {
            return Err(SpectraBridgeError::failure(
                "Insufficient TON for the reviewed transfer and network fee",
            ));
        }
        Ok(())
    }

    pub(super) async fn prepare_ton(
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
        Ok(match chain.mainnet_counterpart() {
            Chain::Ton => {
                if amount >= 1u128 << 120 {
                    return Err(SpectraBridgeError::invalid(
                        "Jetton amount exceeds 120-bit protocol range",
                    ));
                }
                let client = ToncenterV2Client::new(eps);
                client.verify_network(chain).await?;
                let jetton = if let Some(master) = &request.contract_address {
                    let source_wallet = self
                        .ton_transfer_wallet(chain, sender, master, amount)
                        .await?;
                    Some(crate::send::ton::PreparedJettonTransfer {
                        master: master.clone(),
                        source_wallet,
                        attached_nanotons: 100_000_000,
                    })
                } else {
                    None
                };
                let value = jetton.as_ref().map_or(amount_u64, |j| j.attached_nanotons);
                let network_fee = chain
                    .static_fee_units()
                    .ok_or_else(|| SpectraBridgeError::failure("TON fee unavailable"))?;
                let required = u128::from(value) + network_fee;
                if u128::from(client.fetch_balance(sender).await?.nanotons) < required {
                    return Err(SpectraBridgeError::failure(
                        "Insufficient TON for transfer and network fee",
                    ));
                }
                request.fee_amount = Some(crate::amount::decimal::from_units(
                    network_fee
                        + jetton
                            .as_ref()
                            .map_or(0, |j| u128::from(j.attached_nanotons)),
                    9,
                ));
                let seqno = client.fetch_seqno(sender).await?;
                PreparedPayload::Ton {
                    seqno,
                    amount,
                    // Deployment messages sign the all-ones expiry value.
                    valid_until: if seqno == 0 {
                        u32::MAX
                    } else {
                        u32::try_from(crate::store::now_unix() as u64 + 60)
                            .map_err(|_| SpectraBridgeError::failure("TON expiry overflow"))?
                    },
                    jetton,
                }
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a TON network"));
            }
        })
    }

    pub(super) async fn sign_staged_ton(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let seed = || crate::send::keys::Ed25519Seed::from_hex(&signer.private_key_hex);
        Ok(match &stored.prepared {
            PreparedPayload::Ton {
                seqno,
                amount,
                valid_until,
                jetton,
            } => {
                let key = decode_secret_array::<32>(&signer.private_key_hex)?;
                let public = seed()?.public_key();
                // The stored sender names the wallet version this key signs as.
                let wallet = crate::send::ton::TonSigner::for_sender(
                    chain,
                    &stored.view.sender,
                    &key,
                    &public,
                )?;
                let recipient = crate::derivation::ton::parse_ton_address(&stored.view.recipient)?
                    .for_network(chain.is_testnet())?;
                let raw = if let Some(plan) = jetton {
                    crate::send::ton::build_jetton_transfer(
                        &wallet,
                        plan,
                        recipient,
                        crate::derivation::ton::parse_ton_address(&stored.view.sender)?
                            .for_network(chain.is_testnet())?,
                        *amount,
                        *seqno,
                        *valid_until,
                    )?
                } else {
                    crate::send::ton::build_transfer_for_address(
                        &wallet,
                        recipient,
                        u64::try_from(*amount).map_err(|_| {
                            SpectraBridgeError::invalid("TON native amount exceeds protocol range")
                        })?,
                        *seqno,
                        None,
                        *valid_until,
                        3,
                    )?
                };
                let message_hash = STANDARD.encode(crate::derivation::ton::boc_root_hash(&raw)?);
                (
                    SignedPayload::Base64(STANDARD.encode(raw)),
                    Some(message_hash),
                )
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a TON transaction"));
            }
        })
    }

    /// Whether a signed TON transaction can no longer land.
    pub(super) async fn ton_signature_expired(
        &self,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let now = crate::store::now_unix();
        Ok(match &stored.prepared {
            PreparedPayload::Ton { valid_until, .. } => now >= f64::from(*valid_until),
            _ => false,
        })
    }

    async fn ton_transfer_wallet(
        &self,
        chain: Chain,
        owner: &str,
        master: &str,
        amount: u128,
    ) -> Result<String, SpectraBridgeError> {
        let canonical = |value: &str| {
            crate::catalog::tokens::normalize_token_identifier(Some(value.to_string()), chain)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid TON account identity"))
        };
        let master = canonical(master)?;
        let owner = canonical(owner)?;
        let client = crate::api::toncenter_v3::ToncenterV3Client::new(Arc::new(
            self.api_endpoints(
                chain,
                EndpointApi::ToncenterV3,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::TokenBalance,
                ],
            )
            .await?,
        ));
        let wallet = client.fetch_transfer_wallet(chain, &owner, &master).await?;
        if canonical(&wallet.owner)? != owner || canonical(&wallet.jetton)? != master {
            return Err(SpectraBridgeError::invalid(
                "Jetton source wallet owner or master mismatch",
            ));
        }
        let balance: u128 = wallet
            .balance
            .parse()
            .map_err(|_| SpectraBridgeError::invalid("Invalid jetton balance"))?;
        if balance < amount {
            return Err(SpectraBridgeError::failure("Insufficient jetton balance"));
        }
        canonical(&wallet.address)
    }
}

//! Substrate: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;

impl WalletService {
    pub(super) async fn prepare_substrate(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount { eps, amount, .. } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Polkadot | Chain::Bittensor => {
                let prepared = crate::api::http::race(&eps, |endpoint| async move {
                    crate::send::polkadot::prepare_transfer(
                        &SubstrateClient::new(Arc::new(vec![endpoint])),
                        chain,
                        sender,
                        to,
                        amount,
                    )
                    .await
                })
                .await?;
                if let Some(reviewed) = &request.fee_amount {
                    let budget = crate::amount::input::parse_raw_amount(
                        reviewed,
                        u32::from(chain.native_decimals()),
                    )?;
                    if prepared.fee > budget {
                        return Err(SpectraBridgeError::failure(
                            "Substrate fee increased; refresh the preview and review again",
                        ));
                    }
                }
                request.fee_amount = Some(crate::amount::decimal::from_units(
                    prepared.fee,
                    u32::from(chain.native_decimals()),
                ));
                PreparedPayload::Substrate(prepared)
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Substrate network"));
            }
        })
    }

    pub(super) async fn sign_staged_substrate(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(match &stored.prepared {
            PreparedPayload::Substrate(prepared) => {
                crate::api::http::race(&eps, |endpoint| async move {
                    prepared
                        .validate_for_signing(
                            &SubstrateClient::new(Arc::new(vec![endpoint])),
                            chain,
                            &stored.view.sender,
                        )
                        .await
                })
                .await?;
                let key = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                let public =
                    hex::decode(signer.public_key_hex.as_deref().ok_or_else(|| {
                        SpectraBridgeError::failure("Missing sr25519 public key")
                    })?)?;
                let public: &[u8; 32] = public
                    .as_slice()
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid sr25519 public key"))?;
                let raw = prepared.sign(&key, public)?;
                let hash = format!(
                    "0x{}",
                    hex::encode(crate::derivation::primitives::blake2b::<32>(&[&raw]))
                );
                (
                    SignedPayload::Hex(format!("0x{}", hex::encode(raw))),
                    Some(hash),
                )
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Substrate transaction"));
            }
        })
    }

    /// Whether a signed Substrate transaction can no longer land.
    pub(super) async fn substrate_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(match &stored.prepared {
            PreparedPayload::Substrate(prepared) => {
                crate::api::http::race(&endpoints, |endpoint| async move {
                    prepared
                        .validate_for_submission(
                            &SubstrateClient::new(Arc::new(vec![endpoint])),
                            chain,
                        )
                        .await
                })
                .await?;
                false
            }
            _ => false,
        })
    }
}

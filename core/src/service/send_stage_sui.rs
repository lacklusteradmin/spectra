//! Sui: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;

impl WalletService {
    /// Recheck a prepared Sui transaction against the chain.
    pub(super) async fn validate_sui_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Sui(_) = &stored.prepared else {
            return Ok(());
        };
        let mut request = stored.request.clone();
        let refreshed = self
            .prepare_staged_protocol(chain, &mut request, &stored.view.sender)
            .await?;
        if serde_json::to_vec(&refreshed)? != serde_json::to_vec(&stored.prepared)? {
            return Err(SpectraBridgeError::failure(
                "Sui objects or gas changed; build and review again",
            ));
        }
        Ok(())
    }

    pub(super) async fn prepare_sui(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount { amount_u64, .. } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Sui => {
                let gas = request
                    .gas_budget
                    .as_deref()
                    .map(|v| crate::send::payload::fee_units(v, 9))
                    .transpose()?
                    .unwrap_or(10_000_000);
                let client = SuiClient::new(
                    self.endpoints_for(
                        chain,
                        &[
                            EndpointCapability::Verification,
                            EndpointCapability::Balance,
                            EndpointCapability::Fee,
                        ],
                    )
                    .await,
                );
                client.verify_network(chain).await?;
                PreparedPayload::Sui(if let Some(coin_type) = &request.contract_address {
                    crate::send::sui::prepare_token_transfer(
                        &client, sender, to, amount_u64, gas, coin_type,
                    )
                    .await?
                } else {
                    crate::send::sui::prepare_native_transfer(&client, sender, to, amount_u64, gas)
                        .await?
                })
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Sui network"));
            }
        })
    }

    pub(super) async fn sign_staged_sui(
        &self,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let seed = || crate::send::keys::Ed25519Seed::from_hex(&signer.private_key_hex);
        Ok(match &stored.prepared {
            PreparedPayload::SuiMerge(merge) => {
                let p = &merge.transaction;
                let (bytes, signature) = p.clone().sign(&seed()?)?;
                (
                    SignedPayload::Sui {
                        transaction: bytes,
                        signature,
                    },
                    Some(p.transaction_digest()),
                )
            }
            PreparedPayload::Sui(p) => {
                let (bytes, signature) = p.clone().sign(&seed()?)?;
                (
                    SignedPayload::Sui {
                        transaction: bytes,
                        signature,
                    },
                    Some(p.transaction_digest()),
                )
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Sui transaction"));
            }
        })
    }
}

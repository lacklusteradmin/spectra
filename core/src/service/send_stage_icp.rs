//! Internet Computer: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;

impl WalletService {
    pub(super) async fn prepare_icp(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount { amount_u64, .. } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Icp => PreparedPayload::Icp(
                crate::send::icp_stages::prepare_transfer(
                    &IcpClient::new(
                        self.endpoints_for(
                            chain,
                            &[EndpointCapability::Verification, EndpointCapability::Fee],
                        )
                        .await,
                    ),
                    sender,
                    to,
                    amount_u64,
                )
                .await?,
            ),
            _ => {
                return Err(SpectraBridgeError::failure(
                    "Not a Internet Computer network",
                ));
            }
        })
    }

    pub(super) async fn sign_staged_icp(
        &self,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        Ok(match &stored.prepared {
            PreparedPayload::Icp(p) => {
                if (crate::store::now_unix() * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns {
                    return Err(SpectraBridgeError::failure(
                        "ICP transaction expired; build and review again",
                    ));
                }
                let key = crate::send::keys::Secp256k1Key::from_hex(&signer.private_key_hex)?;
                (
                    SignedPayload::Json(p.sign(&key)?),
                    Some(p.transaction_hash()?),
                )
            }
            PreparedPayload::IcpStaking(p) => {
                if (crate::store::now_unix() * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns {
                    return Err(SpectraBridgeError::failure(
                        "ICP staking expired; build and review again",
                    ));
                }
                let calls = p.sign(&crate::send::keys::Secp256k1Key::from_hex(
                    &signer.private_key_hex,
                )?)?;
                let hash = calls.last().map(|c| c.request_id.clone()).ok_or_else(|| {
                    SpectraBridgeError::invalid("Missing ICP staking ingress IDs")
                })?;
                (SignedPayload::IcpStaking(calls), Some(hash))
            }
            _ => {
                return Err(SpectraBridgeError::failure(
                    "Not a Internet Computer transaction",
                ));
            }
        })
    }

    /// Whether a signed Internet Computer transaction can no longer land.
    pub(super) async fn icp_signature_expired(
        &self,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let now = crate::store::now_unix();
        Ok(match &stored.prepared {
            PreparedPayload::Icp(p) => (now * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns,
            PreparedPayload::IcpStaking(p) => (now * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns,
            _ => false,
        })
    }
}

//! Stellar: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::payment_memo::PaymentMemo;
use crate::send::stages::*;
use base64::{Engine, engine::general_purpose::STANDARD};

/// A Stellar signer's 64-byte key and its public half, from a 32-byte seed
/// or a 64-byte keypair.
fn stellar_signing_key(
    private_key_hex: &str,
) -> Result<(zeroize::Zeroizing<[u8; 64]>, [u8; 32]), SpectraBridgeError> {
    let bytes = zeroize::Zeroizing::new(hex::decode(private_key_hex)?);
    if !matches!(bytes.len(), 32 | 64) {
        return Err(SpectraBridgeError::failure("Invalid Stellar signing key"));
    }
    let seed: &[u8; 32] = bytes[..32]
        .try_into()
        .map_err(|_| SpectraBridgeError::failure("Invalid Stellar seed"))?;
    let public = ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes();
    let mut key = zeroize::Zeroizing::new([0u8; 64]);
    key[..32].copy_from_slice(seed);
    key[32..].copy_from_slice(&public);
    Ok((key, public))
}

impl WalletService {
    /// Recheck a prepared Stellar transaction against the chain.
    pub(super) async fn validate_stellar_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        // So may a Stellar account's master key's weight, or its thresholds.
        let needs = match &stored.prepared {
            PreparedPayload::StellarAccountMerge { .. } => {
                super::multisig_stellar::StellarThreshold::High
            }
            _ => super::multisig_stellar::StellarThreshold::Medium,
        };
        self.validate_stellar_master_key(chain, &stored.view.sender, needs)
            .await?;
        if matches!(
            stored.prepared,
            PreparedPayload::Stellar { .. } | PreparedPayload::StellarAssetPayment(_)
        ) {
            self.require_payment_memo(chain, &stored.view.recipient, stored.request.memo.as_ref())
                .await?;
        }
        match &stored.prepared {
            PreparedPayload::StellarAssetPayment(prepared) => {
                let fresh = crate::send::stellar_issued::PreparedStellarAssetPayment::plan(
                    &HorizonClient::new(
                        self.endpoints_for(
                            chain,
                            &[
                                EndpointCapability::Verification,
                                EndpointCapability::TokenBalance,
                            ],
                        )
                        .await,
                    ),
                    chain,
                    &stored.view.sender,
                    &stored.view.recipient,
                    &prepared.asset,
                    prepared.amount_stroops,
                    prepared.fee_stroops,
                )
                .await?;
                if !fresh.same_payment(prepared) {
                    return Err(SpectraBridgeError::failure(
                        "The asset payment changed; build and review again",
                    ));
                }
            }
            PreparedPayload::StellarChangeTrust(prepared) => {
                let (fresh, _) = crate::send::stellar_issued::PreparedStellarChangeTrust::plan(
                    &HorizonClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::Verification])
                            .await,
                    ),
                    chain,
                    &stored.view.sender,
                    &prepared.asset,
                    prepared.removes(),
                    prepared.fee_stroops,
                )
                .await?;
                if !fresh.same_change(prepared) {
                    return Err(SpectraBridgeError::failure(
                        "The trust line changed; build and review again",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn prepare_stellar(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount { eps, amount, .. } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Stellar if request.contract_address.is_some() => {
                self.validate_stellar_master_key(
                    chain,
                    sender,
                    super::multisig_stellar::StellarThreshold::Medium,
                )
                .await?;
                PreparedPayload::StellarAssetPayment(
                    crate::send::stellar_issued::PreparedStellarAssetPayment::plan(
                        &HorizonClient::new(
                            self.endpoints_for(
                                chain,
                                &[
                                    EndpointCapability::Verification,
                                    EndpointCapability::TokenBalance,
                                ],
                            )
                            .await,
                        ),
                        chain,
                        sender,
                        to,
                        request.contract_address.as_deref().unwrap_or_default(),
                        i64::try_from(amount)
                            .map_err(|_| SpectraBridgeError::failure("Amount too large"))?,
                        HorizonClient::new(
                            self.endpoints_for(chain, &[EndpointCapability::Fee]).await,
                        )
                        .fetch_base_fee()
                        .await?,
                    )
                    .await?,
                )
            }
            Chain::Stellar => {
                self.validate_stellar_master_key(
                    chain,
                    sender,
                    super::multisig_stellar::StellarThreshold::Medium,
                )
                .await?;
                let client = HorizonClient::new(eps);
                PreparedPayload::Stellar {
                    sequence: client
                        .fetch_sequence(sender)
                        .await?
                        .checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::failure("Sequence exhausted"))?,
                    fee_stroops: HorizonClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::Fee]).await,
                    )
                    .fetch_base_fee()
                    .await?,
                    amount_stroops: i64::try_from(amount)
                        .map_err(|_| SpectraBridgeError::failure("Amount too large"))?,
                }
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Stellar network"));
            }
        })
    }

    pub(super) async fn sign_staged_stellar(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let sequence = match &stored.prepared {
            PreparedPayload::Stellar { sequence, .. }
            | PreparedPayload::StellarAccountMerge { sequence, .. } => *sequence,
            PreparedPayload::StellarAssetPayment(prepared) => prepared.sequence,
            PreparedPayload::StellarChangeTrust(prepared) => prepared.sequence,
            _ => return Err(SpectraBridgeError::failure("Not a Stellar transaction")),
        };
        // The transaction takes the account's next sequence, so nothing may
        // have been sent from it since the review. A merge checks every
        // prerequisite again: the account or its destination may have
        // changed too.
        let unchanged = if let PreparedPayload::StellarAccountMerge { .. } = &stored.prepared {
            let (fresh, ..) = self
                .account_closing(
                    chain,
                    &stored.view.sender,
                    &stored.view.recipient,
                    stored.request.memo.is_some(),
                )
                .await?;
            matches!(fresh, PreparedPayload::StellarAccountMerge { sequence: s, .. } if s == sequence)
        } else {
            HorizonClient::new(
                self.endpoints_for(chain, &[EndpointCapability::Verification])
                    .await,
            )
            .fetch_sequence(&stored.view.sender)
            .await?
            .checked_add(1)
                == Some(sequence)
        };
        if !unchanged {
            return Err(SpectraBridgeError::failure(
                "Stellar sequence changed; build and review again",
            ));
        }
        let (key, public) = stellar_signing_key(&signer.private_key_hex)?;
        let passphrase = chain.stellar_network_passphrase()?.as_bytes();
        let memo = || PaymentMemo::stellar(stored.request.memo.as_ref());
        let raw = match &stored.prepared {
            PreparedPayload::Stellar {
                fee_stroops,
                amount_stroops,
                ..
            } => crate::send::stellar::build_signed_payment_xdr(
                &stored.view.sender,
                &stored.view.recipient,
                memo()?,
                None,
                *amount_stroops,
                *fee_stroops,
                sequence,
                passphrase,
                &key,
                &public,
            )?,
            PreparedPayload::StellarAssetPayment(prepared) => {
                crate::send::stellar::build_signed_payment_xdr(
                    &stored.view.sender,
                    &stored.view.recipient,
                    memo()?,
                    Some(&prepared.asset()?),
                    prepared.amount_stroops,
                    prepared.fee_stroops,
                    sequence,
                    passphrase,
                    &key,
                    &public,
                )?
            }
            PreparedPayload::StellarChangeTrust(prepared) => {
                crate::send::stellar::build_signed_change_trust_xdr(
                    &stored.view.sender,
                    &prepared.asset()?,
                    prepared.limit_stroops,
                    prepared.fee_stroops,
                    sequence,
                    passphrase,
                    &key,
                    &public,
                )?
            }
            PreparedPayload::StellarAccountMerge { fee_stroops, .. } => {
                crate::send::stellar::build_signed_account_merge_xdr(
                    &stored.view.sender,
                    &stored.view.recipient,
                    memo()?,
                    *fee_stroops,
                    sequence,
                    passphrase,
                    &key,
                    &public,
                )?
            }
            _ => unreachable!("the sequence match above refused every other payload"),
        };
        Ok((SignedPayload::Base64(STANDARD.encode(raw)), None))
    }
}

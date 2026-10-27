//! XRP Ledger: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::payment_memo::PaymentMemo;
use crate::send::stages::*;

impl WalletService {
    /// Recheck a prepared XRP Ledger transaction against the chain.
    pub(super) async fn validate_xrp_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        // The account's master key, which the wallet holds, may have been
        // disabled since the review.
        self.validate_xrp_master_key(chain, &stored.view.sender)
            .await?;
        if matches!(
            stored.prepared,
            PreparedPayload::Xrp { .. } | PreparedPayload::XrpIssuedPayment(_)
        ) {
            self.require_payment_memo(chain, &stored.view.recipient, stored.request.memo.as_ref())
                .await?;
        }
        match &stored.prepared {
            // Every prerequisite again, from the ledger as it is now: a
            // trust line, a freeze, an authorization or an issuer's rate may
            // have changed since the review.
            PreparedPayload::XrpIssuedPayment(prepared) => {
                let fresh = crate::send::xrp_issued::PreparedXrpIssuedPayment::plan(
                    &XrplClient::new(
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
                    &prepared.amount,
                    prepared.fee_drops,
                )
                .await?;
                if !fresh.same_payment(prepared) {
                    return Err(SpectraBridgeError::failure(
                        "The token's issuer fee changed; build and review again",
                    ));
                }
            }
            PreparedPayload::XrpTrustSet(prepared) => {
                let (fresh, _) = crate::send::xrp_issued::PreparedXrpTrustSet::plan(
                    &XrplClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::Verification])
                            .await,
                    ),
                    chain,
                    &stored.view.sender,
                    &prepared.asset,
                    prepared.removes(),
                    prepared.fee_drops,
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

    pub(super) async fn prepare_xrp(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount {
            eps, amount_u64, ..
        } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Xrp => {
                self.validate_xrp_master_key(chain, sender).await?;
                let client = XrplClient::new(eps);
                let fee_drops =
                    XrplClient::new(self.endpoints_for(chain, &[EndpointCapability::Fee]).await)
                        .fetch_fee()
                        .await?;
                crate::send::xrp::validate_drops(u128::from(fee_drops))?;
                match request.contract_address.as_deref() {
                    Some(asset) => PreparedPayload::XrpIssuedPayment(
                        crate::send::xrp_issued::PreparedXrpIssuedPayment::plan(
                            &XrplClient::new(
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
                            asset,
                            &request.amount_str,
                            fee_drops,
                        )
                        .await?,
                    ),
                    None => PreparedPayload::Xrp {
                        sequence: client.fetch_sequence(sender).await?,
                        fee_drops,
                        amount_drops: amount_u64,
                    },
                }
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a XRP Ledger network"));
            }
        })
    }

    pub(super) async fn sign_staged_xrp(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let sequence = match &stored.prepared {
            PreparedPayload::Xrp { sequence, .. }
            | PreparedPayload::XrpAccountDelete { sequence, .. } => *sequence,
            PreparedPayload::XrpIssuedPayment(prepared) => prepared.sequence,
            PreparedPayload::XrpTrustSet(prepared) => prepared.sequence,
            _ => {
                return Err(SpectraBridgeError::failure("Not a XRP Ledger transaction"));
            }
        };
        // Nothing may have been sent from the account since the review. A
        // deletion checks every prerequisite again: the account or its
        // destination may have changed too.
        let unchanged = if let PreparedPayload::XrpAccountDelete { .. } = &stored.prepared {
            let (fresh, ..) = self
                .account_closing(
                    chain,
                    &stored.view.sender,
                    &stored.view.recipient,
                    stored.request.memo.is_some(),
                )
                .await?;
            matches!(fresh, PreparedPayload::XrpAccountDelete { sequence: s, .. } if s == sequence)
        } else {
            XrplClient::new(
                self.endpoints_for(chain, &[EndpointCapability::Verification])
                    .await,
            )
            .fetch_sequence(&stored.view.sender)
            .await?
                == sequence
        };
        if !unchanged {
            return Err(SpectraBridgeError::failure(
                "XRP sequence changed; build and review again",
            ));
        }
        let key = signer.private_key()?;
        let public = signer
            .public_key_hex
            .as_deref()
            .ok_or_else(|| SpectraBridgeError::failure("Missing XRP public key"))?;
        let tag = || PaymentMemo::destination_tag(stored.request.memo.as_ref());
        let blob = match &stored.prepared {
            PreparedPayload::Xrp {
                fee_drops,
                amount_drops,
                ..
            } => crate::send::xrp::build_signed_payment(
                &stored.view.sender,
                &stored.view.recipient,
                tag()?,
                &crate::send::xrp::PaymentAmount::Drops(*amount_drops),
                None,
                *fee_drops,
                sequence,
                &key,
                public,
            )?,
            PreparedPayload::XrpIssuedPayment(prepared) => {
                let (amount, send_max) = prepared.amounts()?;
                crate::send::xrp::build_signed_payment(
                    &stored.view.sender,
                    &stored.view.recipient,
                    tag()?,
                    &crate::send::xrp::PaymentAmount::Issued(amount),
                    send_max.as_ref(),
                    prepared.fee_drops,
                    sequence,
                    &key,
                    public,
                )?
            }
            PreparedPayload::XrpTrustSet(prepared) => crate::send::xrp::build_signed_trust_set(
                &stored.view.sender,
                &prepared.limit()?,
                prepared.fee_drops,
                sequence,
                &key,
                public,
            )?,
            PreparedPayload::XrpAccountDelete { fee_drops, .. } => {
                crate::send::xrp::build_signed_account_delete(
                    &stored.view.sender,
                    &stored.view.recipient,
                    tag()?,
                    *fee_drops,
                    sequence,
                    &key,
                    public,
                )?
            }
            _ => unreachable!("the sequence match above refused every other payload"),
        };
        Ok((SignedPayload::Hex(blob), None))
    }
}

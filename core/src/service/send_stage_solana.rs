//! Solana: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;
use base64::{Engine, engine::general_purpose::STANDARD};

impl WalletService {
    /// Recheck a prepared Solana transaction against the chain.
    pub(super) async fn validate_solana_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        match &stored.prepared {
            PreparedPayload::Solana(prepared) if stored.view.staking.is_none() => {
                if let Some(token) = &prepared.token {
                    // Read every rule again: a fee, a hook account or an
                    // account state changed since review is a new transfer.
                    let client = SolanaClient::new(
                        self.endpoints_for(
                            chain,
                            &[
                                EndpointCapability::Verification,
                                EndpointCapability::TokenBalance,
                            ],
                        )
                        .await,
                    );
                    let owner = crate::derivation::solana::decode_b58_32(&stored.view.sender)?;
                    let recipient =
                        crate::derivation::solana::decode_b58_32(&stored.view.recipient)?;
                    let fresh = crate::send::solana_token::PreparedSolanaTokenTransfer::plan(
                        &client,
                        &owner,
                        &recipient,
                        &token.mint,
                        token.decimals,
                        token.amount,
                    )
                    .await?;
                    if !fresh.same_transfer(token)
                        || token.message(&owner, &recipient, &prepared.blockhash)?
                            != prepared.message
                    {
                        return Err(SpectraBridgeError::failure(
                            "The token's transfer fee or hook accounts changed; build and review again",
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn prepare_solana(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount {
            decimals,
            amount_u64,
            ..
        } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Solana => PreparedPayload::Solana(
                crate::send::solana::prepare_transfer(
                    &SolanaClient::new(
                        self.endpoints_for(
                            chain,
                            if request.contract_address.is_some() {
                                &[
                                    EndpointCapability::Verification,
                                    EndpointCapability::TokenBalance,
                                ]
                            } else {
                                &[EndpointCapability::Verification]
                            },
                        )
                        .await,
                    ),
                    sender,
                    to,
                    amount_u64,
                    request
                        .contract_address
                        .as_deref()
                        .map(|mint| (mint, decimals as u8)),
                )
                .await?,
            ),
            _ => {
                return Err(SpectraBridgeError::failure("Not a Solana network"));
            }
        })
    }

    pub(super) async fn sign_staged_solana(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let transaction = solana_transaction(&stored.prepared)
            .ok_or_else(|| SpectraBridgeError::failure("Not a Solana transaction"))?;
        if !SolanaClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        )
        .blockhash_is_valid(&transaction.blockhash)
        .await?
        {
            return Err(SpectraBridgeError::failure(
                "Solana blockhash expired; build and review again",
            ));
        }
        let raw = transaction.sign(&crate::send::keys::Ed25519Seed::from_hex(
            &signer.private_key_hex,
        )?)?;
        Ok((
            SignedPayload::Base64(STANDARD.encode(&raw)),
            Some(bs58::encode(&raw[1..65]).into_string()),
        ))
    }

    /// Whether a signed Solana transaction can no longer land.
    pub(super) async fn solana_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let Some(transaction) = solana_transaction(&stored.prepared) else {
            return Ok(false);
        };
        Ok(!SolanaClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        )
        .blockhash_is_valid(&transaction.blockhash)
        .await?)
    }
}

/// The transaction a Solana payload signs: a transfer's own, or the one that
/// closes token accounts.
fn solana_transaction(
    prepared: &PreparedPayload,
) -> Option<&crate::send::solana::PreparedSolanaTransaction> {
    match prepared {
        PreparedPayload::Solana(transaction) => Some(transaction),
        PreparedPayload::SolanaAccountClosure(closure) => Some(&closure.transaction),
        _ => None,
    }
}

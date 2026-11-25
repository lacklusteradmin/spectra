//! Starknet: building, revalidating, signing and expiring an ERC-20
//! transfer from a Ready account — behind the account's deployment while it
//! has none — and its send preview.
use std::sync::Arc;

use starknet_crypto::Felt;

use super::protocols::StagedAmount;
use super::*;
use crate::api::starknet_json_rpc::{FeeEstimate, ResourceBounds, StarknetClient};
use crate::derivation::starknet::{decode_address, felt_hex, parse_felt};
use crate::send::payload::SignedPayload;
use crate::send::preview_types::SendPreview;
use crate::send::starknet::{PreparedStarknetTransfer, StarknetTransferTerms};

/// Where a sending account stands on chain.
enum StarknetAccount {
    /// Deployed, with the nonce its next transaction takes.
    Deployed { nonce: u64 },
    /// Not deployed yet: a send deploys it first, under this public key.
    Undeployed { public_key: Felt },
}

/// The account a send must come from: a Ready account Spectra signs for,
/// deployed with no guardian whose co-signature it cannot add and owned by
/// the wallet's key, or one not deployed yet whose public key the wallet
/// recorded.
async fn sending_account(
    client: &StarknetClient,
    chain: Chain,
    sender: &Felt,
    public_key: Option<&str>,
) -> Result<StarknetAccount, SpectraBridgeError> {
    let network = chain.starknet_network()?;
    let public_key = public_key.and_then(parse_felt);
    match client.fetch_class_hash(sender).await? {
        Some(class) => {
            if Some(class) != parse_felt(network.account_class_hash) {
                return Err(SpectraBridgeError::refused(
                    "This Starknet account is a contract Spectra does not sign for (class %@).",
                    [felt_hex(&class)],
                ));
            }
            let (owner, guardian) = client.fetch_signers(sender).await?;
            if guardian != Felt::ZERO {
                return Err(SpectraBridgeError::refused(
                    "This %@ account has a guardian, whose co-signature Spectra cannot add.",
                    [chain.chain_display_name()],
                ));
            }
            if public_key.is_some_and(|key| key != owner) {
                return Err(SpectraBridgeError::refused(
                    "This wallet's key no longer owns its %@ account.",
                    [chain.chain_display_name()],
                ));
            }
            Ok(StarknetAccount::Deployed {
                nonce: client.fetch_nonce(sender).await?,
            })
        }
        None => public_key
            .map(|public_key| StarknetAccount::Undeployed { public_key })
            .ok_or_else(|| {
                SpectraBridgeError::refused(
                    "This %@ account is not deployed yet, and only a wallet holding its key can deploy it.",
                    [chain.chain_display_name()],
                )
            }),
    }
}

/// The bounds an estimate's transactions get, headroom above what each
/// used: the deployment's when there is one, then the transfer's.
async fn estimate_bounds(
    client: &StarknetClient,
    unpriced: &PreparedStarknetTransfer,
) -> Result<(ResourceBounds, Option<ResourceBounds>), SpectraBridgeError> {
    let (deploy, invoke) = unpriced.unsigned(ResourceBounds::ZERO, ResourceBounds::ZERO);
    let requests: Vec<serde_json::Value> = deploy
        .iter()
        .map(|deploy| deploy.request_body(true))
        .chain([invoke.request_body(true)])
        .collect();
    let estimates = client.estimate_fees(&requests).await?;
    let bounds = |estimate: &FeeEstimate| {
        estimate
            .bounds(crate::registry::STARKNET_FEE_HEADROOM_PERCENT)
            .ok_or_else(|| SpectraBridgeError::failure("Starknet fee bounds overflow"))
    };
    match estimates.as_slice() {
        [transfer] => Ok((bounds(transfer)?, None)),
        [deployment, transfer] => Ok((bounds(transfer)?, Some(bounds(deployment)?))),
        _ => Err(SpectraBridgeError::failure(
            "Starknet fee estimate: one estimate per transaction",
        )),
    }
}

/// The wallet's recorded Stark public key.
fn wallet_public_key(
    state: &crate::store::state::ResidentState,
    wallet_id: &str,
) -> Option<String> {
    state
        .wallets
        .iter()
        .find(|wallet| wallet.id == wallet_id)
        .and_then(|wallet| wallet.starknet_public_key.clone())
}

impl WalletService {
    async fn starknet_client(&self, chain: Chain, needs: &[EndpointCapability]) -> StarknetClient {
        StarknetClient::new(self.endpoints_for(chain, needs).await)
    }

    pub(super) async fn prepare_starknet(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let network = chain.starknet_network()?;
        if request.memo.is_some() {
            return Err(SpectraBridgeError::failure(
                "A Starknet transfer carries no memo",
            ));
        }
        let client = StarknetClient::new(Arc::clone(&staged.eps));
        client.verify_network(chain).await?;
        let sender_felt = decode_address(chain, sender)?;
        let fee_token = parse_felt(network.fee_token)
            .ok_or_else(|| SpectraBridgeError::failure("Invalid Starknet fee token"))?;
        let token = match &request.contract_address {
            Some(contract) => decode_address(chain, contract)?,
            None => fee_token,
        };
        let public_key = wallet_public_key(&self.app_state().await, &request.wallet_id);
        let account = sending_account(&client, chain, &sender_felt, public_key.as_deref()).await?;
        // What the transfer moves is there before anything is estimated: a
        // simulated transfer of more than the balance only fails.
        let held = client.fetch_token_balance(&token, &sender_felt).await?;
        if held < staged.amount {
            return Err(crate::send::error::SendError::insufficient_funds().into());
        }
        let token_hex = felt_hex(&token);
        let terms = |nonce, resource_bounds, deployment| StarknetTransferTerms {
            sender,
            token: &token_hex,
            recipient: &request.to_address,
            amount: staged.amount,
            nonce,
            resource_bounds,
            deployment,
        };
        let (nonce, deploying) = match account {
            StarknetAccount::Deployed { nonce } => (nonce, None),
            StarknetAccount::Undeployed { public_key } => (1, Some(public_key)),
        };
        let unpriced = crate::send::starknet::prepare_transfer(
            chain,
            terms(
                nonce,
                ResourceBounds::ZERO,
                deploying.map(|key| (key, ResourceBounds::ZERO)),
            ),
        )?;
        let (transfer_bounds, deploy_bounds) = estimate_bounds(&client, &unpriced).await?;
        let prepared = crate::send::starknet::prepare_transfer(
            chain,
            terms(nonce, transfer_bounds, deploying.zip(deploy_bounds)),
        )?;
        let max_fee = prepared.max_fee()?;
        let strk = if token == fee_token {
            held
        } else {
            client.fetch_balance(chain, &sender_felt).await?
        };
        let required = max_fee
            .checked_add(if token == fee_token { staged.amount } else { 0 })
            .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
        if strk < required {
            let decimals = u32::from(chain.native_decimals());
            let symbol = chain.coin_symbol();
            let required = crate::amount::decimal::from_units(required, decimals);
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                [symbol, required.as_str(), symbol],
            ));
        }
        request.fee_amount = Some(crate::amount::decimal::from_units(
            max_fee,
            u32::from(chain.native_decimals()),
        ));
        Ok(PreparedPayload::Starknet(prepared))
    }

    /// Recheck a prepared transfer against the chain: the same network, an
    /// account still undeployed where the transfer deploys it or still at
    /// the transfer's nonce where it does not, and the funds for the
    /// amount and the most the fees can be.
    pub(super) async fn validate_starknet_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Starknet(prepared) = &stored.prepared else {
            return Ok(());
        };
        let network = chain.starknet_network()?;
        let client = self
            .starknet_client(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        if prepared.chain_id != network.chain_id {
            return Err(SpectraBridgeError::failure(
                "Starknet transaction was prepared for another network",
            ));
        }
        let public_key = match &prepared.deployment {
            Some(deployment) => Some(felt_hex(&deployment.public_key)),
            None => wallet_public_key(&self.app_state().await, &stored.view.wallet_id),
        };
        match (
            sending_account(&client, chain, &prepared.sender, public_key.as_deref()).await?,
            &prepared.deployment,
        ) {
            (StarknetAccount::Undeployed { .. }, Some(_)) => {}
            (StarknetAccount::Deployed { nonce }, None) if nonce == prepared.nonce => {}
            _ => {
                return Err(SpectraBridgeError::failure(
                    "Starknet account nonce changed; build and review again",
                ));
            }
        }
        let fee_token = parse_felt(network.fee_token)
            .ok_or_else(|| SpectraBridgeError::failure("Invalid Starknet fee token"))?;
        let held = client
            .fetch_token_balance(&prepared.token, &prepared.sender)
            .await?;
        let strk = if prepared.token == fee_token {
            held
        } else {
            client.fetch_balance(chain, &prepared.sender).await?
        };
        let required = prepared
            .max_fee()?
            .checked_add(if prepared.token == fee_token {
                prepared.amount
            } else {
                0
            })
            .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
        if held < prepared.amount || strk < required {
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee.",
                [chain.coin_symbol()],
            ));
        }
        Ok(())
    }

    pub(super) async fn sign_staged_starknet(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &crate::service::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let PreparedPayload::Starknet(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::failure("Not a Starknet transaction"));
        };
        let key =
            crate::derivation::starknet::private_key(signer.private_key_array::<32>()?.as_slice())?;
        let signed = prepared.sign(chain, &key)?;
        let hash = felt_hex(&signed.invoke_hash);
        Ok((SignedPayload::Starknet(Box::new(signed)), Some(hash)))
    }

    /// Whether a signed Starknet transfer can no longer land: a Starknet
    /// transaction has no expiry, but once the account's nonce has passed
    /// the transfer's, another transaction took it.
    pub(super) async fn starknet_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let PreparedPayload::Starknet(prepared) = &stored.prepared else {
            return Ok(false);
        };
        let client = self
            .starknet_client(chain, &[EndpointCapability::Verification])
            .await;
        client.verify_network(chain).await?;
        Ok(client.fetch_nonce(&prepared.sender).await? > prepared.nonce)
    }

    /// A Starknet send's quote: the most a transfer of STRK from `address`
    /// can cost — with the account's deployment when it has none — and what
    /// its STRK can pay. The estimate moves at most one fri to a placeholder
    /// recipient, so an empty account is quoted too.
    pub(in crate::service) async fn preview_starknet_send(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let network = chain.starknet_network()?;
        let client = self
            .starknet_client(
                chain,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        let sender = decode_address(chain, address)?;
        let public_key = self
            .app_state()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.chain_id == chain && wallet.address.as_deref() == Some(address))
            .and_then(|wallet| wallet.starknet_public_key.clone());
        let account = sending_account(&client, chain, &sender, public_key.as_deref()).await?;
        let balance = client.fetch_balance(chain, &sender).await?;
        let (nonce, deployment) = match account {
            StarknetAccount::Deployed { nonce } => (nonce, None),
            StarknetAccount::Undeployed { public_key } => (
                1,
                Some(crate::send::starknet::PreparedStarknetDeployment {
                    public_key,
                    class_hash: parse_felt(network.account_class_hash).ok_or_else(|| {
                        SpectraBridgeError::failure("Invalid Starknet account class")
                    })?,
                    resource_bounds: ResourceBounds::ZERO,
                }),
            ),
        };
        let deploying = deployment.is_some();
        // Built directly rather than through `prepare_transfer`, which
        // refuses a zero amount: this transfer is only simulated.
        let quoted = PreparedStarknetTransfer {
            chain_id: network.chain_id.to_string(),
            sender,
            token: parse_felt(network.fee_token)
                .ok_or_else(|| SpectraBridgeError::failure("Invalid Starknet fee token"))?,
            recipient: Felt::ONE,
            amount: balance.min(1),
            nonce,
            resource_bounds: ResourceBounds::ZERO,
            deployment,
        };
        let (transfer_bounds, deploy_bounds) = estimate_bounds(&client, &quoted).await?;
        let priced = PreparedStarknetTransfer {
            resource_bounds: transfer_bounds,
            deployment: quoted.deployment.map(|deployment| {
                crate::send::starknet::PreparedStarknetDeployment {
                    resource_bounds: deploy_bounds.unwrap_or(ResourceBounds::ZERO),
                    ..deployment
                }
            }),
            ..quoted
        };
        let fee = priced.max_fee()?;
        let units = |amount: u128| {
            crate::amount::decimal::from_units(amount, u32::from(chain.native_decimals()))
        };
        Ok(SendPreview {
            estimated_network_fee: units(fee),
            spendable_balance: units(balance),
            max_sendable: units(balance.saturating_sub(fee)),
            fee_rate_description: Some(if deploying {
                "The most the network can charge, deploying the account and sending: estimates with 50% headroom on gas and its price".into()
            } else {
                "The most the network can charge: an estimate with 50% headroom on gas and its price".into()
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "../tests/send_stage_starknet.rs"]
mod tests;

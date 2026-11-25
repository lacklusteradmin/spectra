//! Cosmos SDK networks: building, revalidating, signing and expiring a
//! transfer of the network's own coin, and its send preview.
use std::sync::Arc;

use super::protocols::StagedAmount;
use super::*;
use crate::api::cosmos_rest::{CosmosAccount, CosmosClient};
use crate::send::cosmos::{CosmosTransferTerms, PreparedCosmosTransfer};
use crate::send::payload::SignedPayload;
use crate::send::payment_memo::PaymentMemo;
use crate::send::preview_types::SendPreview;

/// Headroom over the gas a simulation used: the same transaction can use a
/// little more once it carries its signature and the state it reads moves.
const GAS_HEADROOM_PERCENT: u64 = 40;

/// The gas a preview prices when the node cannot simulate for the address
/// (one that has never received anything has no account to simulate from),
/// and the placeholder limit a simulated transaction carries.
const UNSIMULATED_TRANSFER_GAS: u64 = crate::registry::COSMOS_TRANSFER_GAS_ESTIMATE;

/// The gas limit a simulation's `gas_used` calls for.
fn gas_limit(gas_used: u64) -> Option<u64> {
    gas_used
        .checked_mul(100 + GAS_HEADROOM_PERCENT)
        .map(|gas| gas.div_ceil(100))
}

/// The price a transaction pays per unit of gas on `chain`: the network's
/// floor, or its fee market's current price and a fifth more (the price may
/// rise before the transaction lands) where that is higher.
async fn gas_price(
    client: &CosmosClient,
    chain: Chain,
) -> Result<crate::registry::GasPrice, SpectraBridgeError> {
    let floor = chain.cosmos_network()?.gas_price;
    Ok(match client.fetch_market_gas_price(chain).await? {
        Some(market) => {
            let raised = crate::registry::GasPrice {
                numerator: market.numerator.saturating_mul(6),
                denominator: market.denominator.saturating_mul(5),
            };
            // Both fractions are small; comparing cross products is exact.
            if raised.numerator.saturating_mul(floor.denominator)
                > floor.numerator.saturating_mul(raised.denominator)
            {
                raised
            } else {
                floor
            }
        }
        None => floor,
    })
}

/// The account a send must come from, refused while the chain has none.
async fn existing_account(
    client: &CosmosClient,
    chain: Chain,
    sender: &str,
) -> Result<CosmosAccount, SpectraBridgeError> {
    client.fetch_account(sender).await?.ok_or_else(|| {
        SpectraBridgeError::failed(
            "%@ has no account at this address yet: it has never received %@.",
            [chain.chain_display_name(), chain.coin_symbol()],
        )
    })
}

/// The gas limit and fee a transfer needs, from a node's simulation of it
/// priced at the network's gas price.
async fn price_transfer(
    client: &CosmosClient,
    chain: Chain,
    unpriced: &PreparedCosmosTransfer,
) -> Result<(u64, u128), SpectraBridgeError> {
    let used = client.simulate(&unpriced.simulation_bytes()).await?;
    let limit = gas_limit(used).ok_or_else(|| SpectraBridgeError::failure("Gas overflow"))?;
    let fee = gas_price(client, chain)
        .await?
        .fee_for(limit)
        .ok_or_else(|| SpectraBridgeError::failure("Fee overflow"))?;
    Ok((limit, fee))
}

impl WalletService {
    async fn cosmos_client(&self, chain: Chain, needs: &[EndpointCapability]) -> CosmosClient {
        CosmosClient::new(self.endpoints_for(chain, needs).await)
    }

    pub(super) async fn prepare_cosmos(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        if request.contract_address.is_some() {
            return Err(SpectraBridgeError::failure(
                "Cosmos sends carry the network's own coin only",
            ));
        }
        let client = CosmosClient::new(Arc::clone(&staged.eps));
        client.verify_network(chain).await?;
        let account = existing_account(&client, chain, sender).await?;
        let height = client.fetch_latest_height().await?;
        let timeout_blocks = chain.cosmos_timeout_blocks()?;
        let memo = PaymentMemo::text(request.memo.as_ref(), chain)?;
        let terms = |gas_limit: u64, fee: u128| CosmosTransferTerms {
            sender,
            recipient: &request.to_address,
            amount: staged.amount,
            memo,
            account_number: account.account_number,
            sequence: account.sequence,
            gas_limit,
            fee,
            timeout_height: height + timeout_blocks,
        };
        let unpriced =
            crate::send::cosmos::prepare_transfer(chain, terms(UNSIMULATED_TRANSFER_GAS, 0))?;
        let fees = self.cosmos_client(chain, &[EndpointCapability::Fee]).await;
        let (gas_limit, mut fee) = price_transfer(&fees, chain, &unpriced).await?;
        // A reviewed fee that is higher than the network asks for is kept:
        // it is what the person agreed to pay.
        if let Some(reviewed) = request.fee_amount.as_deref() {
            let reviewed =
                crate::send::payload::fee_units(reviewed, u32::from(chain.native_decimals()))?;
            fee = fee.max(u128::from(reviewed));
        }
        let spendable = client.fetch_spendable(chain, sender).await?;
        let required = staged
            .amount
            .checked_add(fee)
            .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
        if spendable < required {
            let decimals = u32::from(chain.native_decimals());
            let symbol = chain.coin_symbol();
            let required = crate::amount::decimal::from_units(required, decimals);
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                [symbol, required.as_str(), symbol],
            ));
        }
        let prepared = crate::send::cosmos::prepare_transfer(chain, terms(gas_limit, fee))?;
        request.fee_amount = Some(crate::amount::decimal::from_units(
            fee,
            u32::from(chain.native_decimals()),
        ));
        Ok(PreparedPayload::Cosmos(prepared))
    }

    /// Recheck a prepared transfer against the chain: the same network and
    /// account, the sequence still next, the coin to pay for it, and a
    /// timeout height still ahead.
    pub(super) async fn validate_cosmos_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Cosmos(prepared) = &stored.prepared else {
            return Ok(());
        };
        let client = self
            .cosmos_client(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        if prepared.chain_id != chain.cosmos_network()?.chain_id {
            return Err(SpectraBridgeError::failure(
                "Cosmos transaction was prepared for another network",
            ));
        }
        let account = existing_account(&client, chain, &prepared.sender).await?;
        if account.account_number != prepared.account_number
            || account.sequence != prepared.sequence
        {
            return Err(SpectraBridgeError::failure(
                "Cosmos account sequence changed; build and review again",
            ));
        }
        if client.fetch_latest_height().await? >= prepared.timeout_height {
            return Err(SpectraBridgeError::failure(
                "Cosmos transaction timed out; build and review again",
            ));
        }
        let required = prepared
            .amount
            .checked_add(prepared.fee)
            .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
        if client.fetch_spendable(chain, &prepared.sender).await? < required {
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee.",
                [chain.coin_symbol()],
            ));
        }
        Ok(())
    }

    pub(super) async fn sign_staged_cosmos(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &crate::service::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let PreparedPayload::Cosmos(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::failure("Not a Cosmos transaction"));
        };
        let key = secp256k1::SecretKey::from_slice(signer.private_key_array::<32>()?.as_slice())
            .map_err(|_| SpectraBridgeError::failure("Invalid secp256k1 key"))?;
        let (raw, hash) = prepared.sign(chain, &key)?;
        use base64::Engine as _;
        Ok((
            SignedPayload::Base64(base64::engine::general_purpose::STANDARD.encode(raw)),
            Some(hash),
        ))
    }

    /// Whether a signed Cosmos transaction can no longer land: the chain is
    /// at or past its timeout height.
    pub(super) async fn cosmos_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let PreparedPayload::Cosmos(prepared) = &stored.prepared else {
            return Ok(false);
        };
        let client = self
            .cosmos_client(chain, &[EndpointCapability::Verification])
            .await;
        client.verify_network(chain).await?;
        Ok(client.fetch_latest_height().await? >= prepared.timeout_height)
    }

    /// A Cosmos send's quote: the fee a node's simulation of a transfer from
    /// `address` prices, and what its coin can pay. An address the chain
    /// has no account for yet holds nothing to send.
    pub(in crate::service) async fn preview_cosmos_send(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let client = self
            .cosmos_client(
                chain,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        let decimals = u32::from(chain.native_decimals());
        let units = |amount: u128| crate::amount::decimal::from_units(amount, decimals);
        let (fee, spendable) = match client.fetch_account(address).await? {
            Some(account) => {
                let height = client.fetch_latest_height().await?;
                let unpriced = crate::send::cosmos::prepare_transfer(
                    chain,
                    CosmosTransferTerms {
                        sender: address,
                        recipient: address,
                        amount: 1,
                        memo: "",
                        account_number: account.account_number,
                        sequence: account.sequence,
                        gas_limit: UNSIMULATED_TRANSFER_GAS,
                        fee: 0,
                        timeout_height: height + 1,
                    },
                )?;
                let (_, fee) = price_transfer(&client, chain, &unpriced).await?;
                (fee, client.fetch_spendable(chain, address).await?)
            }
            None => (
                gas_price(&client, chain)
                    .await?
                    .fee_for(UNSIMULATED_TRANSFER_GAS)
                    .ok_or_else(|| SpectraBridgeError::failure("Fee overflow"))?,
                0,
            ),
        };
        Ok(SendPreview {
            estimated_network_fee: units(fee),
            spendable_balance: units(spendable),
            max_sendable: units(spendable.saturating_sub(fee)),
            fee_rate_description: Some(
                "Simulated gas at the network's gas price; vesting coins excluded".into(),
            ),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "../tests/send_stage_cosmos.rs"]
mod tests;

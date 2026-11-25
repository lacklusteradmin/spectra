//! Filecoin: building, revalidating, signing and expiring a transfer of FIL,
//! and its send preview.
use std::sync::Arc;

use super::protocols::StagedAmount;
use super::*;
use crate::api::lotus_json_rpc::LotusClient;
use crate::derivation::filecoin::FilecoinProtocol;
use crate::send::filecoin::{
    FilecoinTransferTerms, MAX_FEE, METHOD_SEND, estimate_message, transfer_method,
};
use crate::send::payload::SignedPayload;
use crate::send::preview_types::SendPreview;

/// The method a transfer to `recipient` calls, from what the chain holds
/// there. An ID or actor address names an actor that already exists, since
/// nothing creates one at either; a send to one that does not is refused.
async fn recipient_method(
    client: &LotusClient,
    chain: Chain,
    recipient: &str,
) -> Result<u64, SpectraBridgeError> {
    let protocol = crate::derivation::filecoin::decode_address(chain, recipient)?.protocol();
    let ethereum_actor = match protocol {
        FilecoinProtocol::Id | FilecoinProtocol::Actor => client
            .fetch_actor(recipient)
            .await?
            .ok_or_else(|| {
                SpectraBridgeError::refused("No actor exists at %@ to receive FIL.", [recipient])
            })?
            .delegated_address
            .is_some_and(|address| {
                crate::derivation::filecoin::decode_address(chain, &address)
                    .is_ok_and(|address| address.protocol() == FilecoinProtocol::Delegated)
            }),
        _ => false,
    };
    Ok(transfer_method(protocol, ethereum_actor))
}

fn insufficient(chain: Chain, required: u128) -> SpectraBridgeError {
    let symbol = chain.coin_symbol();
    let required = crate::amount::decimal::from_units(required, u32::from(chain.native_decimals()));
    SpectraBridgeError::failed(
        "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
        [symbol, required.as_str(), symbol],
    )
}

impl WalletService {
    async fn lotus_client(&self, chain: Chain, needs: &[EndpointCapability]) -> LotusClient {
        LotusClient::new(self.endpoints_for(chain, needs).await)
    }

    pub(super) async fn prepare_filecoin(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        if request.contract_address.is_some() {
            return Err(SpectraBridgeError::failure("Filecoin sends carry FIL only"));
        }
        let client = LotusClient::new(Arc::clone(&staged.eps));
        client.verify_network(chain).await?;
        let method = recipient_method(&client, chain, &request.to_address).await?;
        // A node estimates gas by running the message, which fails without
        // the value to send.
        let balance = client.fetch_balance(sender).await?;
        if balance < staged.amount {
            return Err(insufficient(chain, staged.amount));
        }
        let nonce = client.fetch_next_nonce(sender).await?;
        let gas = client
            .estimate_gas(
                &estimate_message(sender, &request.to_address, staged.amount, method, nonce),
                MAX_FEE,
            )
            .await?;
        let prepared = crate::send::filecoin::prepare_transfer(
            chain,
            FilecoinTransferTerms {
                sender,
                recipient: &request.to_address,
                value: staged.amount,
                method,
                nonce,
                gas,
            },
        )?;
        let (Some(fee), Some(required)) = (prepared.maximum_fee(), prepared.maximum_cost()) else {
            return Err(SpectraBridgeError::failure("Amount overflow"));
        };
        if balance < required {
            return Err(insufficient(chain, required));
        }
        request.fee_amount = Some(crate::amount::decimal::from_units(
            fee,
            u32::from(chain.native_decimals()),
        ));
        Ok(PreparedPayload::Filecoin(prepared))
    }

    /// Recheck a prepared message against the chain: the same network, the
    /// method its recipient takes, its nonce still the next, and the FIL to
    /// pay the most it can cost.
    pub(super) async fn validate_filecoin_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Filecoin(prepared) = &stored.prepared else {
            return Ok(());
        };
        let client = self
            .lotus_client(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        if recipient_method(&client, chain, &prepared.recipient).await? != prepared.method {
            return Err(SpectraBridgeError::failure(
                "Filecoin recipient changed; build and review again",
            ));
        }
        if client.fetch_next_nonce(&prepared.sender).await? != prepared.nonce {
            return Err(SpectraBridgeError::failure(
                "Filecoin nonce changed; build and review again",
            ));
        }
        let required = prepared
            .maximum_cost()
            .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
        if client.fetch_balance(&prepared.sender).await? < required {
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee.",
                [chain.coin_symbol()],
            ));
        }
        Ok(())
    }

    pub(super) async fn sign_staged_filecoin(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &crate::service::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let PreparedPayload::Filecoin(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::failure("Not a Filecoin message"));
        };
        let key = secp256k1::SecretKey::from_slice(signer.private_key_array::<32>()?.as_slice())
            .map_err(|_| SpectraBridgeError::failure("Invalid secp256k1 key"))?;
        let (signed, cid) = prepared.sign(chain, &key)?;
        Ok((SignedPayload::Filecoin(signed), Some(cid)))
    }

    /// Whether a signed message can no longer land: the chain has executed
    /// a message from its sender with its nonce, this one or another. A
    /// message has no expiry of its own.
    pub(super) async fn filecoin_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let PreparedPayload::Filecoin(prepared) = &stored.prepared else {
            return Ok(false);
        };
        let client = self
            .lotus_client(chain, &[EndpointCapability::Verification])
            .await;
        client.verify_network(chain).await?;
        Ok(client
            .fetch_actor(&prepared.sender)
            .await?
            .is_some_and(|actor| actor.nonce > prepared.nonce))
    }

    /// A Filecoin send's quote: the most a transfer from `address` to itself
    /// can cost in gas, as a node estimates it, and the FIL to pay it. A node
    /// estimates by running the message, which an address holding no FIL
    /// cannot send; it is quoted the most any message may pay.
    pub(in crate::service) async fn preview_filecoin_send(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let client = self
            .lotus_client(
                chain,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        let balance = client.fetch_balance(address).await?;
        let actor = match balance {
            0 => None,
            _ => client.fetch_actor(address).await?,
        };
        let (fee, description) = match actor {
            Some(actor) => {
                let gas = client
                    .estimate_gas(
                        &estimate_message(address, address, 1, METHOD_SEND, actor.nonce),
                        MAX_FEE,
                    )
                    .await?;
                let fee = u128::from(gas.limit)
                    .checked_mul(gas.fee_cap)
                    .filter(|fee| *fee <= MAX_FEE)
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("Filecoin gas estimate is out of range")
                    })?;
                (
                    fee,
                    "Gas limit at the fee cap: the most a transfer can cost",
                )
            }
            None => (MAX_FEE, "No FIL to send: the most any message may pay"),
        };
        let units = |amount: u128| {
            crate::amount::decimal::from_units(amount, u32::from(chain.native_decimals()))
        };
        Ok(SendPreview {
            estimated_network_fee: units(fee),
            spendable_balance: units(balance),
            max_sendable: units(balance.saturating_sub(fee)),
            fee_rate_description: Some(description.into()),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "../tests/send_stage_filecoin.rs"]
mod tests;

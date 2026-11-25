//! Tezos: building, revalidating, signing and expiring a transfer of tez,
//! and its send preview.
//!
//! A transfer's gas and storage come from a node's simulation of it. An
//! account whose key is not on chain cannot be simulated until its reveal
//! names that key, which only signing has: its first transfer is limited by
//! the protocol's own costs for a reveal and for a transfer to an implicit
//! account, and is simulated with its key just before it is signed.
use super::protocols::{StagedAmount, protocol_u64};
use super::*;
use crate::api::tezos_rpc::{
    TezosAccount, TezosBlock, TezosClient, TezosConsumption, TezosContent,
};
use crate::derivation::tezos::decode_address;
use crate::registry::{TezosNetwork, TezosProtocol};
use crate::send::payload::SignedPayload;
use crate::send::preview_types::SendPreview;
use crate::send::tezos::{PreparedTezosTransfer, TezosTransferTerms};

/// The storage an operation that consumed `used` needs: the origination size
/// of every account or contract it creates, exactly, and what the contracts
/// it calls add to their storage, with headroom.
fn storage_needed(
    network: &TezosNetwork,
    protocol: &TezosProtocol,
    used: &TezosConsumption,
) -> Option<u64> {
    let created = used.allocations.checked_mul(protocol.origination_size)?;
    let paid = match used.paid_storage {
        0 => 0,
        paid => paid.checked_add(network.storage_headroom)?,
    };
    created.checked_add(paid)
}

/// What a node says a transfer needs before it is built: the protocol it is
/// forged for, its branch, the sender's account and the limits each of its
/// operations takes.
struct TezosPlan {
    protocol: &'static TezosProtocol,
    branch: TezosBlock,
    account: TezosAccount,
    reveal_gas_limit: Option<u64>,
    gas_limit: u64,
    storage_limit: u64,
}

/// The protocol a verified node runs, refused when `registry` does not
/// list it or the next block will run another.
async fn current_protocol(
    client: &TezosClient,
    network: &TezosNetwork,
) -> Result<&'static TezosProtocol, SpectraBridgeError> {
    let (protocol, next) = client.fetch_protocols().await?;
    match network.protocol(&protocol) {
        Some(known) if next == protocol => Ok(known),
        _ => Err(SpectraBridgeError::refused(
            "Tezos runs protocol %@, whose operations Spectra does not yet sign.",
            [if next == protocol { protocol } else { next }],
        )),
    }
}

/// Plan a transfer of `amount` mutez from `sender` to `destination` on a
/// verified node.
async fn plan_transfer(
    client: &TezosClient,
    chain: Chain,
    sender: &str,
    destination: &str,
    amount: u64,
) -> Result<TezosPlan, SpectraBridgeError> {
    client.verify_network(chain).await?;
    let network = chain.tezos_network()?;
    let protocol = current_protocol(client, network).await?;
    let branch = client.fetch_block(network.branch_depth).await?;
    let account = client.fetch_account(sender).await?;
    let counter = account
        .counter
        .checked_add(1)
        .ok_or_else(|| SpectraBridgeError::failure("Tezos counter overflows"))?;
    let headroom = |gas: u64| {
        gas.checked_add(network.gas_headroom)
            .ok_or_else(|| SpectraBridgeError::failure("Tezos gas overflows"))
    };
    if account.revealed {
        if account.balance < u128::from(amount) {
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee.",
                [chain.coin_symbol()],
            ));
        }
        let simulated = client
            .simulate(
                network.chain_id,
                &branch.hash,
                &[TezosContent::Transaction {
                    source: sender.to_string(),
                    fee: 0,
                    counter,
                    gas_limit: protocol.hard_gas_limit_per_operation,
                    storage_limit: protocol.hard_storage_limit_per_operation,
                    amount,
                    destination: destination.to_string(),
                }],
            )
            .await?;
        let used = simulated
            .first()
            .ok_or_else(|| SpectraBridgeError::failure("Tezos simulation answered nothing"))?;
        return Ok(TezosPlan {
            protocol,
            reveal_gas_limit: None,
            gas_limit: headroom(used.gas())?,
            storage_limit: storage_needed(network, protocol, used)
                .ok_or_else(|| SpectraBridgeError::failure("Tezos storage overflows"))?,
            branch,
            account,
        });
    }
    // The reveal names the key, which only signing has: no node can
    // simulate the batch yet. A transfer to an implicit account costs the
    // protocol's fixed gas and, when that account is empty, its creation; a
    // contract's code could cost anything.
    if !decode_address(destination)?.is_implicit() {
        return Err(SpectraBridgeError::refused(
            "%@ has not revealed its key yet, which its first transfer does. Send that one to a tz address; a contract can be paid after.",
            [sender],
        ));
    }
    let empty = client.fetch_account(destination).await?.balance == 0;
    Ok(TezosPlan {
        protocol,
        reveal_gas_limit: Some(headroom(protocol.reveal_gas)?),
        gas_limit: headroom(protocol.implicit_transfer_gas)?,
        storage_limit: if empty { protocol.origination_size } else { 0 },
        branch,
        account,
    })
}

/// The transfer `plan` describes.
fn prepared_from(
    chain: Chain,
    plan: &TezosPlan,
    sender: &str,
    destination: &str,
    amount: u128,
) -> Result<PreparedTezosTransfer, SpectraBridgeError> {
    Ok(crate::send::tezos::prepare_transfer(
        chain,
        TezosTransferTerms {
            source: sender,
            destination,
            amount,
            protocol: plan.protocol.hash,
            branch: &plan.branch.hash,
            branch_level: plan.branch.level,
            counter: plan.account.counter,
            reveal_gas_limit: plan.reveal_gas_limit,
            gas_limit: plan.gas_limit,
            storage_limit: plan.storage_limit,
        },
    )?)
}

/// What the batch costs the sender beyond the amount: its fees and the most
/// it burns for storage. This is the fee a Tezos send reviews.
fn network_cost(
    chain: Chain,
    prepared: &PreparedTezosTransfer,
) -> Result<u128, SpectraBridgeError> {
    prepared
        .total_fee()
        .checked_add(prepared.max_burn(chain)?)
        .ok_or_else(|| SpectraBridgeError::failure("Tezos fee overflows"))
}

impl WalletService {
    async fn tezos_client(&self, chain: Chain, needs: &[EndpointCapability]) -> TezosClient {
        TezosClient::new(self.endpoints_for(chain, needs).await)
    }

    pub(super) async fn prepare_tezos(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        if request.contract_address.is_some() {
            return Err(SpectraBridgeError::failure(
                "Tezos sends carry tez only; FA1.2 and FA2 tokens are not sent",
            ));
        }
        let amount = protocol_u64(staged.amount)?;
        let client = self
            .tezos_client(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                    EndpointCapability::Fee,
                ],
            )
            .await;
        let decimals = u32::from(chain.native_decimals());
        let symbol = chain.coin_symbol();
        let plan = plan_transfer(&client, chain, sender, &request.to_address, amount).await?;
        let prepared = prepared_from(chain, &plan, sender, &request.to_address, staged.amount)?;
        let required = prepared.most_debited(chain)?;
        if plan.account.balance < required {
            let required = crate::amount::decimal::from_units(required, decimals);
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                [symbol, required.as_str(), symbol],
            ));
        }
        request.fee_amount = Some(crate::amount::decimal::from_units(
            network_cost(chain, &prepared)?,
            decimals,
        ));
        Ok(PreparedPayload::Tezos(prepared))
    }

    /// Recheck a prepared transfer against the chain before it is signed:
    /// the same network and protocol, its branch still live, the sender's
    /// counter and key where the batch left them, and the tez to pay for
    /// it all.
    pub(super) async fn validate_tezos_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Tezos(prepared) = &stored.prepared else {
            return Ok(());
        };
        prepared.validate(chain)?;
        let client = self
            .tezos_client(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        let protocol = current_protocol(&client, chain.tezos_network()?).await?;
        if protocol.hash != prepared.protocol {
            return Err(SpectraBridgeError::failure(
                "Tezos changed protocol; build and review again",
            ));
        }
        if client.fetch_head_level().await? >= prepared.expires_at_level {
            return Err(SpectraBridgeError::failure(
                "Tezos operation expired; build and review again",
            ));
        }
        let account = client.fetch_account(&prepared.source).await?;
        let first = prepared
            .reveal
            .map_or(prepared.transaction.counter, |reveal| reveal.counter);
        if account.counter.checked_add(1) != Some(first)
            || account.revealed != prepared.reveal.is_none()
        {
            return Err(SpectraBridgeError::failure(
                "Tezos account counter changed; build and review again",
            ));
        }
        if account.balance < prepared.most_debited(chain)? {
            return Err(SpectraBridgeError::failed(
                "Insufficient %@ for the amount plus the network fee.",
                [chain.coin_symbol()],
            ));
        }
        Ok(())
    }

    /// Simulate the exact batch, its reveal naming the signing key, and sign
    /// it only when every operation fits the limits it was reviewed with.
    pub(super) async fn sign_staged_tezos(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &crate::service::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let PreparedPayload::Tezos(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::failure("Not a Tezos transaction"));
        };
        let protocol = prepared.validate(chain)?;
        let key = crate::send::keys::Ed25519Seed::from_hex(&signer.private_key_hex)?;
        let client = self
            .tezos_client(
                chain,
                &[EndpointCapability::Verification, EndpointCapability::Fee],
            )
            .await;
        client.verify_network(chain).await?;
        let network = chain.tezos_network()?;
        let used = client
            .simulate(
                network.chain_id,
                &prepared.branch,
                &prepared.contents(&key.public_key()),
            )
            .await?;
        let limits = prepared
            .reveal
            .iter()
            .chain(std::iter::once(&prepared.transaction));
        for (used, limits) in used.iter().zip(limits) {
            let storage = storage_needed(network, protocol, used)
                .ok_or_else(|| SpectraBridgeError::failure("Tezos storage overflows"))?;
            if used.gas() > limits.gas_limit || storage > limits.storage_limit {
                return Err(SpectraBridgeError::failure(
                    "Tezos operation needs more gas or storage than reviewed; build and review again",
                ));
            }
        }
        let (signed, hash) = prepared.sign(chain, &key)?;
        Ok((SignedPayload::Hex(hex::encode(signed)), Some(hash)))
    }

    /// Whether a signed Tezos operation can no longer be included: the
    /// chain has reached the level its branch expires at.
    pub(super) async fn tezos_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let PreparedPayload::Tezos(prepared) = &stored.prepared else {
            return Ok(false);
        };
        let client = self
            .tezos_client(chain, &[EndpointCapability::Verification])
            .await;
        client.verify_network(chain).await?;
        Ok(client.fetch_head_level().await? >= prepared.expires_at_level)
    }

    /// A Tezos send's quote: what a transfer from `address` to `destination`
    /// (or to itself, when none is named yet) costs beyond its amount — its
    /// fees and the most it burns, which includes creating an empty
    /// recipient — and what its tez can pay.
    pub(in crate::service) async fn preview_tezos_send(
        &self,
        chain: Chain,
        address: &str,
        destination: Option<&str>,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let client = self
            .tezos_client(
                chain,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        let destination = destination
            .filter(|d| !d.trim().is_empty())
            .unwrap_or(address);
        let plan = plan_transfer(&client, chain, address, destination, 1).await?;
        let prepared = prepared_from(chain, &plan, address, destination, 1)?;
        let cost = network_cost(chain, &prepared)?;
        let units = |amount: u128| {
            crate::amount::decimal::from_units(amount, u32::from(chain.native_decimals()))
        };
        Ok(SendPreview {
            estimated_network_fee: units(cost),
            spendable_balance: units(plan.account.balance),
            max_sendable: units(plan.account.balance.saturating_sub(cost)),
            fee_rate_description: Some(
                "Simulated gas at the network's minimal fee, and the storage burn of creating an empty recipient".into(),
            ),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "../tests/send_stage_tezos.rs"]
mod tests;

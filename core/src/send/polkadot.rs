//! Substrate native transfers, bound to a validated runtime contract.

use super::substrate::{blake2b_256, decode_hash_hex};
use crate::api::substrate_json_rpc::{PolkadotExtension, PolkadotRuntime, SubstrateClient};
use crate::derivation::polkadot::decode_ss58;
use crate::registry::Chain;
use crate::send::error::SendError;
use parity_scale_codec::{Compact, Encode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedPolkadotTransaction {
    pub runtime: PolkadotRuntime,
    pub sender: [u8; 32],
    /// Exact metadata-encoded call, shared by transfers and staking.
    pub call_data: Vec<u8>,
    pub nonce: u32,
    pub amount: u128,
    pub fee: u128,
    /// Polling starts after this finalized head, then persists progress.
    pub finalized_number: u64,
}

impl PreparedPolkadotTransaction {
    /// SignedPayload encodes call, all Extra, then all AdditionalSigned in
    /// metadata order. Immortal mortality uses genesis for its checkpoint.
    /// Fees are paid in the chain's native asset, without a tip.
    fn extensions(&self) -> Result<(Vec<u8>, Vec<u8>), SendError> {
        let mut extra = Vec::new();
        let mut additional = Vec::new();
        let genesis = decode_hash_hex(&self.runtime.genesis_hash)?;
        for extension in &self.runtime.extensions {
            match extension {
                PolkadotExtension::Unit => {}
                PolkadotExtension::SpecVersion => {
                    additional.extend(self.runtime.spec_version.to_le_bytes())
                }
                PolkadotExtension::TransactionVersion => {
                    additional.extend(self.runtime.transaction_version.to_le_bytes())
                }
                PolkadotExtension::Genesis => additional.extend(genesis),
                PolkadotExtension::Mortality => {
                    extra.push(0);
                    additional.extend(genesis);
                }
                PolkadotExtension::Nonce => extra.extend(Compact(self.nonce).encode()),
                PolkadotExtension::NativePayment => extra.push(0), // zero native tip
                PolkadotExtension::AssetPayment => extra.extend([0, 0]), // zero tip, None asset_id
                PolkadotExtension::MetadataHash => {
                    extra.push(0);
                    additional.push(0);
                } // Disabled, None
            }
        }
        Ok((extra, additional))
    }

    pub fn signing_payload(&self) -> Result<Vec<u8>, SendError> {
        let (extra, additional) = self.extensions()?;
        let mut payload = self.call_data.clone();
        payload.extend(extra);
        payload.extend(additional);
        Ok(payload)
    }

    fn extrinsic(&self, signature: [u8; 64]) -> Result<Vec<u8>, SendError> {
        let mut body = vec![0x84, 0]; // signed v4, MultiAddress::Id
        body.extend(self.sender);
        body.push(1); // MultiSignature::Sr25519
        body.extend(signature);
        body.extend(self.extensions()?.0);
        body.extend(&self.call_data);
        let mut bytes = Compact(u32::try_from(body.len()).map_err(SendError::invalid)?).encode();
        bytes.extend(body);
        Ok(bytes)
    }

    /// QueryInfo decodes a signed envelope without verifying its signature.
    /// Its length, call and extension bytes match the real send.
    pub fn fee_extrinsic(&self) -> Result<Vec<u8>, SendError> {
        self.extrinsic([0; 64])
    }

    /// Sign with `private_key`, the sender's sr25519 seed or a soft
    /// junction's expanded key (`substrate_path::signing_keypair`).
    pub fn sign(&self, private_key: &[u8], public_key: &[u8; 32]) -> Result<Vec<u8>, SendError> {
        if *public_key != self.sender {
            return Err(SendError::invalid(
                "sr25519 key does not match the reviewed sender",
            ));
        }
        let pair = crate::derivation::substrate_path::signing_keypair(private_key, &self.sender)
            .map_err(|_| SendError::invalid("sr25519 key does not match the reviewed sender"))?;
        let payload = self.signing_payload()?;
        let input = if payload.len() > 256 {
            blake2b_256(&payload).to_vec()
        } else {
            payload
        };
        self.extrinsic(pair.sign_simple(b"substrate", &input).to_bytes())
    }

    fn validate_funds(
        &self,
        balance: crate::api::substrate_json_rpc::SubstrateBalance,
    ) -> Result<(), SendError> {
        let required = self
            .amount
            .checked_add(self.fee)
            .ok_or_else(|| SendError::invalid("Amount and fee overflow"))?;
        if required > balance.keep_alive_spendable(self.runtime.existential_deposit) {
            return Err(SendError::invalid(
                "Insufficient Substrate funds after freezes, fee and existential deposit",
            ));
        }
        Ok(())
    }

    pub async fn validate_for_signing(
        &self,
        client: &SubstrateClient,
        chain: Chain,
        address: &str,
    ) -> Result<(), SendError> {
        let context = client.polkadot_context(chain).await?;
        if context.runtime != self.runtime || client.fetch_nonce(address).await? != self.nonce {
            return Err(SendError::invalid(
                "Substrate runtime, network or nonce changed; build and review again",
            ));
        }
        if client
            .query_fee(&self.fee_extrinsic()?, &context.block_hash)
            .await?
            > self.fee
        {
            return Err(SendError::invalid(
                "Substrate fee increased; build and review again",
            ));
        }
        self.validate_funds(
            client
                .fetch_balance_at(chain, &self.sender, &context.block_hash)
                .await?,
        )
    }
    /// A Substrate signature has no fee cap: check the reviewed budget on
    /// every selected broadcast node, without requiring an unused nonce.
    pub async fn validate_for_submission(
        &self,
        client: &SubstrateClient,
        chain: Chain,
    ) -> Result<(), SendError> {
        let context = client.polkadot_context(chain).await?;
        if context.runtime != self.runtime {
            return Err(SendError::invalid(
                "Substrate runtime changed; build and review again",
            ));
        }
        if client
            .query_fee(&self.fee_extrinsic()?, &context.block_hash)
            .await?
            > self.fee
        {
            return Err(SendError::invalid(
                "Substrate fee increased; build and review again",
            ));
        }
        Ok(())
    }
}

pub async fn prepare_transfer(
    client: &SubstrateClient,
    chain: Chain,
    sender: &str,
    recipient: &str,
    amount: u128,
) -> Result<PreparedPolkadotTransaction, SendError> {
    if chain.substrate_balance_bytes() == Some(8) && amount > u128::from(u64::MAX) {
        return Err(SendError::invalid(
            "Amount exceeds the Substrate runtime balance type",
        ));
    }
    if amount == 0 {
        return Err(SendError::invalid(
            "Substrate transfer amount must be positive",
        ));
    }
    let sender_key = decode_ss58(sender)?;
    let recipient_key = decode_ss58(recipient)?;
    let context = client.polkadot_context(chain).await?;
    let mut prepared = PreparedPolkadotTransaction {
        call_data: native_transfer_call(&context.runtime, &recipient_key, amount),
        runtime: context.runtime,
        sender: sender_key,
        nonce: client.fetch_nonce(sender).await?,
        amount,
        fee: 0,
        finalized_number: context.finalized_number,
    };
    prepared.fee = client
        .query_fee(&prepared.fee_extrinsic()?, &context.block_hash)
        .await?;
    prepared.validate_funds(
        client
            .fetch_balance_at(chain, &sender_key, &context.block_hash)
            .await?,
    )?;
    let destination = client
        .fetch_balance_at(chain, &recipient_key, &context.block_hash)
        .await?;
    if destination
        .free
        .checked_add(amount)
        .is_none_or(|v| v < prepared.runtime.existential_deposit)
    {
        return Err(SendError::invalid(
            "Recipient balance would be below the Substrate existential deposit or overflow",
        ));
    }
    Ok(prepared)
}

/// Largest compact amount and actual nonce bound the preview's encoded fee.
pub async fn preview_transfer(
    client: &SubstrateClient,
    chain: Chain,
    address: &str,
) -> Result<(u128, u128, usize), SendError> {
    let context = client.polkadot_context(chain).await?;
    let sender = decode_ss58(address)?;
    let prepared = PreparedPolkadotTransaction {
        call_data: native_transfer_call(
            &context.runtime,
            &sender,
            if chain.substrate_balance_bytes() == Some(8) {
                u128::from(u64::MAX)
            } else {
                u128::MAX
            },
        ),
        runtime: context.runtime,
        sender,
        nonce: client.fetch_nonce(address).await?,
        amount: if chain.substrate_balance_bytes() == Some(8) {
            u128::from(u64::MAX)
        } else {
            u128::MAX
        },
        fee: 0,
        finalized_number: context.finalized_number,
    };
    let extrinsic = prepared.fee_extrinsic()?;
    let fee = client.query_fee(&extrinsic, &context.block_hash).await?;
    let balance = client
        .fetch_balance_at(chain, &sender, &context.block_hash)
        .await?;
    Ok((
        fee,
        balance.keep_alive_spendable(prepared.runtime.existential_deposit),
        extrinsic.len(),
    ))
}

/// Native transfers and staking use one extrinsic envelope and one signature model.
pub fn native_transfer_call(
    runtime: &PolkadotRuntime,
    recipient: &[u8; 32],
    amount: u128,
) -> Vec<u8> {
    transfer_call_at(
        (runtime.transfer_pallet, runtime.transfer_call),
        recipient,
        amount,
    )
}

/// `Balances.transfer_keep_alive` at a runtime's `(pallet, call)` indices.
pub(crate) fn transfer_call_at(indices: (u8, u8), recipient: &[u8; 32], amount: u128) -> Vec<u8> {
    let mut call = vec![indices.0, indices.1, 0];
    call.extend(recipient);
    call.extend(Compact(amount).encode());
    call
}

pub async fn prepare_call(
    client: &SubstrateClient,
    chain: Chain,
    sender: &str,
    context: crate::api::substrate_json_rpc::PolkadotContext,
    call_data: Vec<u8>,
    spend_amount: u128,
) -> Result<PreparedPolkadotTransaction, SendError> {
    if call_data.len() < 2 {
        return Err(SendError::invalid("Invalid Substrate call"));
    }
    let sender_key = decode_ss58(sender)?;
    let mut prepared = PreparedPolkadotTransaction {
        runtime: context.runtime,
        sender: sender_key,
        call_data,
        nonce: client.fetch_nonce(sender).await?,
        amount: spend_amount,
        fee: 0,
        finalized_number: context.finalized_number,
    };
    prepared.fee = client
        .query_fee(&prepared.fee_extrinsic()?, &context.block_hash)
        .await?;
    prepared.validate_funds(
        client
            .fetch_balance_at(chain, &sender_key, &context.block_hash)
            .await?,
    )?;
    Ok(prepared)
}

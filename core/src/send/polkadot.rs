//! Asset Hub native transfers, bound to a validated runtime contract.

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
    pub recipient: [u8; 32],
    pub nonce: u32,
    pub amount: u128,
    pub fee: u128,
    /// Polling starts after this finalized head, then persists progress.
    pub finalized_number: u64,
}

impl PreparedPolkadotTransaction {
    fn call(&self) -> Vec<u8> {
        let mut call = vec![self.runtime.transfer_pallet, self.runtime.transfer_call, 0];
        call.extend(self.recipient);
        call.extend(Compact(self.amount).encode());
        call
    }

    /// SignedPayload encodes call, all Extra, then all AdditionalSigned in
    /// metadata order. Immortal mortality uses genesis for its checkpoint.
    /// Fees are paid in native DOT/WND, without a tip.
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
        let mut payload = self.call();
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
        body.extend(self.call());
        let mut bytes = Compact(u32::try_from(body.len()).map_err(SendError::invalid)?).encode();
        bytes.extend(body);
        Ok(bytes)
    }

    /// QueryInfo decodes a signed envelope without verifying its signature.
    /// Its length, call and extension bytes match the real send.
    pub fn fee_extrinsic(&self) -> Result<Vec<u8>, SendError> {
        self.extrinsic([0; 64])
    }

    pub fn sign(
        &self,
        private_key: &[u8; 32],
        public_key: &[u8; 32],
    ) -> Result<Vec<u8>, SendError> {
        let mini =
            schnorrkel::MiniSecretKey::from_bytes(private_key).map_err(SendError::invalid)?;
        let pair = mini.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
        let pair = if pair.public.to_bytes() == self.sender {
            pair
        } else {
            // The derivation API also offers Uniform expansion. Select it
            // only when its derived public key equals the reviewed account.
            mini.expand_to_keypair(schnorrkel::ExpansionMode::Uniform)
        };
        if pair.public.to_bytes() != self.sender || *public_key != self.sender {
            return Err(SendError::invalid(
                "sr25519 key does not match the reviewed sender",
            ));
        }
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
                "Insufficient Asset Hub funds after freezes, fee and existential deposit",
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
                "Asset Hub runtime, network or nonce changed; build and review again",
            ));
        }
        if client
            .query_fee(&self.fee_extrinsic()?, &context.block_hash)
            .await?
            > self.fee
        {
            return Err(SendError::invalid(
                "Asset Hub fee increased; build and review again",
            ));
        }
        self.validate_funds(
            client
                .fetch_balance_at(&self.sender, &context.block_hash)
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
                "Asset Hub runtime changed; build and review again",
            ));
        }
        if client
            .query_fee(&self.fee_extrinsic()?, &context.block_hash)
            .await?
            > self.fee
        {
            return Err(SendError::invalid(
                "Asset Hub fee increased; build and review again",
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
    if amount == 0 {
        return Err(SendError::invalid(
            "Asset Hub transfer amount must be positive",
        ));
    }
    let sender_key = decode_ss58(sender)?;
    let recipient_key = decode_ss58(recipient)?;
    let context = client.polkadot_context(chain).await?;
    let mut prepared = PreparedPolkadotTransaction {
        runtime: context.runtime,
        sender: sender_key,
        recipient: recipient_key,
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
            .fetch_balance_at(&sender_key, &context.block_hash)
            .await?,
    )?;
    let destination = client
        .fetch_balance_at(&recipient_key, &context.block_hash)
        .await?;
    if destination
        .free
        .checked_add(amount)
        .is_none_or(|v| v < prepared.runtime.existential_deposit)
    {
        return Err(SendError::invalid(
            "Recipient balance would be below the Asset Hub existential deposit or overflow",
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
        runtime: context.runtime,
        sender,
        recipient: sender,
        nonce: client.fetch_nonce(address).await?,
        amount: u128::MAX,
        fee: 0,
        finalized_number: context.finalized_number,
    };
    let extrinsic = prepared.fee_extrinsic()?;
    let fee = client.query_fee(&extrinsic, &context.block_hash).await?;
    let balance = client
        .fetch_balance_at(&sender, &context.block_hash)
        .await?;
    Ok((
        fee,
        balance.keep_alive_spendable(prepared.runtime.existential_deposit),
        extrinsic.len(),
    ))
}

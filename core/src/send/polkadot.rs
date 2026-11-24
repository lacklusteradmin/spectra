//! Substrate transfers — the network's coin, and on Asset Hub an `Assets`
//! pallet asset — bound to a validated runtime contract.

use super::substrate::decode_hash_hex;
use crate::api::substrate_json_rpc::assets::{AssetAccountStatus, AssetStatus, AssetTransferState};
use crate::api::substrate_json_rpc::{
    AssetTransferCall, PolkadotExtension, PolkadotRuntime, SubstrateClient,
};
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
    /// What leaves the account of the network's coin, beside the fee.
    #[serde(with = "crate::send::stages::units_u128")]
    pub amount: u128,
    /// The asset an `Assets.transfer` moves, checked again before signing.
    pub asset: Option<PreparedAssetTransfer>,
    #[serde(with = "crate::send::stages::units_u128")]
    pub fee: u128,
    /// Polling starts after this finalized head, then persists progress.
    pub finalized_number: u64,
}

/// An `Assets.transfer`: which asset, to whom, how much of it, and the
/// decimals the amount was reviewed at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedAssetTransfer {
    pub id: u32,
    pub recipient: [u8; 32],
    #[serde(with = "crate::send::stages::units_u128")]
    pub amount: u128,
    pub decimals: u8,
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
            crate::derivation::primitives::blake2b::<32>(&[&payload]).to_vec()
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
        if let Some(asset) = &self.asset {
            let state = client
                .asset_transfer_state(chain, asset.id, &self.sender, &asset.recipient)
                .await?;
            check_asset_transfer(&state, asset.amount)?;
            if state.decimals != asset.decimals {
                return Err(SendError::invalid("Asset precision changed; review again"));
            }
            if asset_transfer_call(state.call, asset) != self.call_data {
                return Err(SendError::invalid(
                    "Substrate runtime changed; build and review again",
                ));
            }
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
    let sender_key = decode_ss58(chain, sender)?;
    let recipient_key = decode_ss58(chain, recipient)?;
    let context = client.polkadot_context(chain).await?;
    let mut prepared = PreparedPolkadotTransaction {
        call_data: native_transfer_call(&context.runtime, &recipient_key, amount),
        runtime: context.runtime,
        sender: sender_key,
        nonce: client.fetch_nonce(sender).await?,
        amount,
        asset: None,
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
    let sender = decode_ss58(chain, address)?;
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
        asset: None,
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
    let sender_key = decode_ss58(chain, sender)?;
    let mut prepared = PreparedPolkadotTransaction {
        runtime: context.runtime,
        sender: sender_key,
        call_data,
        nonce: client.fetch_nonce(sender).await?,
        amount: spend_amount,
        asset: None,
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

/// `Assets.transfer` of `asset` at the runtime's indices: the asset id, the
/// recipient as `MultiAddress::Id`, the amount.
pub(crate) fn asset_transfer_call(
    call: AssetTransferCall,
    asset: &PreparedAssetTransfer,
) -> Vec<u8> {
    let mut bytes = vec![call.pallet, call.call];
    bytes.extend(Compact(asset.id).encode());
    bytes.push(0);
    bytes.extend(asset.recipient);
    bytes.extend(Compact(asset.amount).encode());
    bytes
}

/// Refuse what the runtime would reject, and what it would take but lose:
/// a holding left below the asset's minimum balance is removed, and what it
/// held is gone with it.
fn check_asset_transfer(state: &AssetTransferState, amount: u128) -> Result<(), SendError> {
    let details = state.details;
    match details.status {
        AssetStatus::Live => {}
        AssetStatus::Frozen => {
            return Err(SendError::invalid(
                "This asset is frozen; it cannot be sent",
            ));
        }
        AssetStatus::Destroying => {
            return Err(SendError::invalid(
                "This asset is being destroyed; it cannot be sent",
            ));
        }
    }
    let held = state
        .sender
        .ok_or_else(|| SendError::invalid("This account holds none of this asset"))?;
    if held.status != AssetAccountStatus::Liquid {
        return Err(SendError::invalid(
            "This account's holding of the asset is frozen; it cannot send it",
        ));
    }
    let rest = held
        .balance
        .checked_sub(amount)
        .ok_or_else(|| SendError::invalid("Insufficient asset balance"))?;
    if rest != 0 && rest < details.min_balance {
        return Err(SendError::invalid(
            "Send all of it, or leave at least the asset's minimum balance: less would be removed",
        ));
    }
    match state.recipient {
        Some(holding) if holding.status == AssetAccountStatus::Blocked => {
            return Err(SendError::invalid(
                "The recipient's holding of this asset is blocked; it cannot receive it",
            ));
        }
        Some(holding) if holding.balance.checked_add(amount).is_none() => {
            return Err(SendError::invalid("Recipient asset balance would overflow"));
        }
        Some(_) => {}
        None if amount < details.min_balance => {
            return Err(SendError::invalid(
                "The recipient holds none of this asset; send at least its minimum balance",
            ));
        }
        None if !details.is_sufficient
            && state.recipient_native.free < state.context.runtime.existential_deposit =>
        {
            return Err(SendError::invalid(
                "This asset cannot open an account: the recipient needs DOT of its own first",
            ));
        }
        None => {}
    }
    Ok(())
}

/// Build an `Assets.transfer` of asset `id`, its fee paid in the network's
/// coin, after checking it against the state it will meet.
pub async fn prepare_asset_transfer(
    client: &SubstrateClient,
    chain: Chain,
    sender: &str,
    recipient: &str,
    id: u32,
    amount: u128,
) -> Result<PreparedPolkadotTransaction, SendError> {
    if amount == 0 {
        return Err(SendError::invalid(
            "Substrate transfer amount must be positive",
        ));
    }
    let sender_key = decode_ss58(chain, sender)?;
    let recipient_key = decode_ss58(chain, recipient)?;
    let state = client
        .asset_transfer_state(chain, id, &sender_key, &recipient_key)
        .await?;
    check_asset_transfer(&state, amount)?;
    let asset = PreparedAssetTransfer {
        id,
        recipient: recipient_key,
        amount,
        decimals: state.decimals,
    };
    let mut prepared = PreparedPolkadotTransaction {
        call_data: asset_transfer_call(state.call, &asset),
        runtime: state.context.runtime,
        sender: sender_key,
        nonce: client.fetch_nonce(sender).await?,
        amount: 0,
        asset: Some(asset),
        fee: 0,
        finalized_number: state.context.finalized_number,
    };
    let block = state.context.block_hash;
    prepared.fee = client.query_fee(&prepared.fee_extrinsic()?, &block).await?;
    prepared.validate_funds(client.fetch_balance_at(chain, &sender_key, &block).await?)?;
    Ok(prepared)
}

/// An asset transfer's fee, quoted at the largest amount so the transfer
/// built later costs no more, and the account's balance of the asset.
pub async fn preview_asset_transfer(
    client: &SubstrateClient,
    chain: Chain,
    address: &str,
    id: u32,
) -> Result<(u128, u128, usize), SendError> {
    let sender = decode_ss58(chain, address)?;
    let state = client
        .asset_transfer_state(chain, id, &sender, &sender)
        .await?;
    let asset = PreparedAssetTransfer {
        id,
        recipient: sender,
        amount: u128::MAX,
        decimals: state.decimals,
    };
    let prepared = PreparedPolkadotTransaction {
        call_data: asset_transfer_call(state.call, &asset),
        runtime: state.context.runtime,
        sender,
        nonce: client.fetch_nonce(address).await?,
        amount: 0,
        asset: Some(asset),
        fee: 0,
        finalized_number: state.context.finalized_number,
    };
    let extrinsic = prepared.fee_extrinsic()?;
    let fee = client
        .query_fee(&extrinsic, &state.context.block_hash)
        .await?;
    Ok((
        fee,
        state.sender.map_or(0, |holding| holding.balance),
        extrinsic.len(),
    ))
}

#[cfg(test)]
#[path = "tests/polkadot.rs"]
mod tests;

#[cfg(test)]
mod asset_tests {
    use super::*;
    use serde_json::Value;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/asset-hub-assets.json")).unwrap()
    }

    #[test]
    fn asset_transfers_match_independent_polkadot_js_calls() {
        let fixture = fixture();
        let recipient = [9; 32];
        for network in ["polkadot", "westend"] {
            let vectors = &fixture[network];
            let call = AssetTransferCall {
                pallet: vectors["transfer"]["pallet"].as_u64().unwrap() as u8,
                call: vectors["transfer"]["call"].as_u64().unwrap() as u8,
            };
            for (name, id, amount) in [("usdt", 1984, 1_234_567), ("max", u32::MAX, u128::MAX)] {
                let encoded = asset_transfer_call(
                    call,
                    &PreparedAssetTransfer {
                        id,
                        recipient,
                        amount,
                        decimals: 6,
                    },
                );
                assert_eq!(
                    format!("0x{}", hex::encode(encoded)),
                    vectors["calls"][name],
                    "{network} {name}"
                );
            }
        }
    }
}

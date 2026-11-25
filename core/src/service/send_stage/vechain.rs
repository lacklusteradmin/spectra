//! VeChainThor: building, revalidating, signing and expiring a transfer of
//! VET or of a VIP-180 token, and its send preview. Gas is paid in VTHO, so
//! a VET transfer draws on two balances and a VTHO transfer on one.
use std::sync::Arc;

use super::protocols::StagedAmount;
use super::*;
use crate::api::thor_rest::{ThorBlock, ThorCall, ThorClient};
use crate::send::payload::SignedPayload;
use crate::send::preview_types::SendPreview;
use crate::send::vechain::{PreparedVechainTransfer, VechainTransferTerms};

/// The gas a clause that runs code is given past what the node's simulation
/// used, as thor's own client gives it: a fifth of it and at least 15,000.
/// Under an exact limit each nested call keeps back a 64th of what remains
/// (EIP-150) and can run out.
fn gas_limit(intrinsic: u64, used: u64) -> Option<u64> {
    let margin = if used == 0 { 0 } else { (used / 5).max(15_000) };
    intrinsic.checked_add(used)?.checked_add(margin)
}

/// The gas a token transfer's code is assumed to use where it cannot be
/// simulated (the sender holds too little to run it): what VTHO's
/// `transfer` used live to a new recipient, rounded up.
const UNSIMULATED_TOKEN_TRANSFER_GAS: u64 = 30_000;

/// The fee cap thor's own client signs: twice the base fee, which covers
/// several blocks of its 12.5% rises, and the tip. A transaction pays the
/// base fee and its tip, never more than this.
fn max_fee_per_gas(base_fee: u128, tip: u128) -> Option<u128> {
    base_fee.checked_mul(2)?.checked_add(tip)
}

/// What a transaction priced on `block` pays per unit of gas: the base fee
/// and the tip.
fn fee_per_gas(block: &ThorBlock, tip: u128) -> Result<u128, SpectraBridgeError> {
    block
        .base_fee_per_gas
        .ok_or_else(|| SpectraBridgeError::failure("VeChainThor block carries no base fee"))?
        .checked_add(tip)
        .ok_or_else(|| SpectraBridgeError::failure("Fee overflow"))
}

/// The VTHO a transaction priced at `per_gas` costs.
fn fee_of(gas: u64, per_gas: u128) -> Result<u128, SpectraBridgeError> {
    u128::from(gas)
        .checked_mul(per_gas)
        .ok_or_else(|| SpectraBridgeError::failure("Fee overflow"))
}

/// The clause `prepared` carries, as a node simulates it.
fn call_of(clause: &(String, u128, Vec<u8>)) -> ThorCall<'_> {
    ThorCall {
        to: &clause.0,
        value: clause.1,
        data: &clause.2,
    }
}

impl WalletService {
    async fn thor_client(&self, chain: Chain, needs: &[EndpointCapability]) -> ThorClient {
        ThorClient::new(self.endpoints_for(chain, needs).await)
    }

    /// Refuse a transfer the sender's balances cannot pay: the amount from
    /// its own asset, the fee from VTHO, and both from VTHO when VTHO is
    /// what is sent.
    async fn require_vechain_funds(
        &self,
        client: &ThorClient,
        chain: Chain,
        prepared: &PreparedVechainTransfer,
        fee: u128,
    ) -> Result<(), SpectraBridgeError> {
        let account = client.fetch_account(&prepared.sender).await?;
        let pays_own_fee = chain.pays_own_fee(prepared.token.as_deref());
        let vtho_needed = if pays_own_fee {
            prepared.amount.checked_add(fee)
        } else {
            Some(fee)
        }
        .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
        if account.energy < vtho_needed {
            let vtho = chain.fee_holding_template().symbol;
            let needed =
                crate::amount::decimal::from_units(vtho_needed, u32::from(chain.native_decimals()));
            return Err(if pays_own_fee {
                SpectraBridgeError::failed(
                    "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                    [vtho.as_str(), needed.as_str(), vtho.as_str()],
                )
            } else {
                SpectraBridgeError::failed(
                    "Insufficient %@ for the %@ network fee (%@ %@).",
                    [
                        vtho.as_str(),
                        chain.chain_display_name(),
                        needed.as_str(),
                        vtho.as_str(),
                    ],
                )
            });
        }
        let held = match &prepared.token {
            None => account.balance,
            Some(_) if pays_own_fee => return Ok(()),
            Some(token) => client.fetch_token_balance(token, &prepared.sender).await?.0,
        };
        if held < prepared.amount {
            return Err(SpectraBridgeError::failure(
                "Insufficient balance for the amount",
            ));
        }
        Ok(())
    }

    pub(super) async fn prepare_vechain(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        if request.memo.is_some() {
            return Err(SpectraBridgeError::failure(
                "VeChainThor transfers carry no memo",
            ));
        }
        let client = ThorClient::new(Arc::clone(&staged.eps));
        client.verify_network(chain).await?;
        let block = client.fetch_best_block().await?;
        let tip = client.fetch_priority_fee().await?;
        let base_fee = block
            .base_fee_per_gas
            .ok_or_else(|| SpectraBridgeError::failure("VeChainThor block carries no base fee"))?;
        let max_fee = max_fee_per_gas(base_fee, tip)
            .ok_or_else(|| SpectraBridgeError::failure("Fee overflow"))?;
        let block_ref: [u8; 8] = block.id[..8].try_into().expect("8 bytes");
        let nonce: [u8; 8] = rand::random();
        let terms = |gas: u64| VechainTransferTerms {
            sender,
            recipient: &request.to_address,
            token: request.contract_address.as_deref(),
            amount: staged.amount,
            block_ref,
            expiration: crate::registry::THOR_EXPIRATION_BLOCKS,
            max_priority_fee_per_gas: tip,
            max_fee_per_gas: max_fee,
            gas,
            nonce,
        };
        // Priced by simulating the very clause the transaction carries.
        let unpriced =
            crate::send::vechain::prepare_transfer(chain, terms(crate::send::vechain::MAX_TX_GAS))?;
        let clause = unpriced.clause()?;
        let used = client.simulate(sender, call_of(&clause)).await?;
        let gas = gas_limit(crate::send::vechain::intrinsic_gas(&clause.2), used)
            .ok_or_else(|| SpectraBridgeError::failure("Gas overflow"))?;
        let prepared = crate::send::vechain::prepare_transfer(chain, terms(gas))?;
        let fee = fee_of(gas, fee_per_gas(&block, tip)?)?;
        self.require_vechain_funds(&client, chain, &prepared, fee)
            .await?;
        request.fee_amount = Some(crate::amount::decimal::from_units(
            fee,
            u32::from(chain.native_decimals()),
        ));
        Ok(PreparedPayload::Vechain(prepared))
    }

    /// Recheck a prepared transfer against the chain: the same network, a
    /// block still within its expiration, and balances that still pay it.
    pub(super) async fn validate_vechain_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Vechain(prepared) = &stored.prepared else {
            return Ok(());
        };
        let client = self
            .thor_client(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        if prepared.chain_tag != chain.thor_network()?.chain_tag() {
            return Err(SpectraBridgeError::failure(
                "VeChainThor transaction was prepared for another network",
            ));
        }
        let block = client.fetch_best_block().await?;
        if u64::from(block.number) >= prepared.last_block() {
            return Err(SpectraBridgeError::failure(
                "VeChainThor transaction expired; build and review again",
            ));
        }
        // What the block's base fee and the signed tip cost now, within the
        // signed cap.
        let per_gas =
            fee_per_gas(&block, prepared.max_priority_fee_per_gas)?.min(prepared.max_fee_per_gas);
        self.require_vechain_funds(&client, chain, prepared, fee_of(prepared.gas, per_gas)?)
            .await
    }

    pub(super) async fn sign_staged_vechain(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &crate::service::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let PreparedPayload::Vechain(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::failure("Not a VeChainThor transaction"));
        };
        let key = secp256k1::SecretKey::from_slice(signer.private_key_array::<32>()?.as_slice())
            .map_err(|_| SpectraBridgeError::failure("Invalid secp256k1 key"))?;
        let (raw, id) = prepared.sign(chain, &key)?;
        Ok((
            SignedPayload::Hex(format!("0x{}", hex::encode(raw))),
            Some(id),
        ))
    }

    /// Whether a signed transaction can no longer land: the next block is
    /// past its expiration.
    pub(super) async fn vechain_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let PreparedPayload::Vechain(prepared) = &stored.prepared else {
            return Ok(false);
        };
        let client = self
            .thor_client(chain, &[EndpointCapability::Verification])
            .await;
        client.verify_network(chain).await?;
        Ok(u64::from(client.fetch_best_block().await?.number) >= prepared.last_block())
    }

    /// A VeChainThor send's quote: the VTHO a node's simulation of the
    /// transfer of `amount` from `address` to `destination` costs at the
    /// block's base fee and the suggested tip, and what can be sent of VET,
    /// VTHO or the token `token` names (its contract and stored decimals).
    /// A transfer the sender's balance cannot run is priced at its intrinsic
    /// gas, and a token's at a typical transfer's besides.
    pub(in crate::service) async fn preview_vechain_send(
        &self,
        chain: Chain,
        address: &str,
        destination: &str,
        token: Option<(&str, u8)>,
        amount: u128,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let client = self
            .thor_client(
                chain,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        client.verify_network(chain).await?;
        let block = client.fetch_best_block().await?;
        let tip = client.fetch_priority_fee().await?;
        let account = client.fetch_account(address).await?;
        let (contract, held) = match token {
            None => (None, account.balance),
            Some((contract, stored_decimals)) => {
                let (held, decimals) = client.fetch_token_balance(contract, address).await?;
                if decimals != stored_decimals {
                    return Err(SpectraBridgeError::invalid(
                        "Token precision differs from the stored deployment",
                    ));
                }
                (Some(contract), held)
            }
        };
        let unpriced = crate::send::vechain::prepare_transfer(
            chain,
            VechainTransferTerms {
                sender: address,
                recipient: destination,
                token: contract,
                amount: amount.max(1),
                block_ref: block.id[..8].try_into().expect("8 bytes"),
                expiration: crate::registry::THOR_EXPIRATION_BLOCKS,
                max_priority_fee_per_gas: 0,
                max_fee_per_gas: 0,
                gas: crate::send::vechain::MAX_TX_GAS,
                nonce: [0; 8],
            },
        )?;
        let clause = unpriced.clause()?;
        let intrinsic = crate::send::vechain::intrinsic_gas(&clause.2);
        let used = match client.simulate(address, call_of(&clause)).await {
            Ok(used) => used,
            Err(crate::api::error::ApiError::Rejected(_)) if contract.is_some() => {
                UNSIMULATED_TOKEN_TRANSFER_GAS
            }
            Err(crate::api::error::ApiError::Rejected(_)) => 0,
            Err(error) => return Err(error.into()),
        };
        let gas = gas_limit(intrinsic, used)
            .ok_or_else(|| SpectraBridgeError::failure("Gas overflow"))?;
        let fee = fee_of(gas, fee_per_gas(&block, tip)?)?;
        let decimals = token.map_or(u32::from(chain.native_decimals()), |(_, d)| u32::from(d));
        let units = |amount: u128| crate::amount::decimal::from_units(amount, decimals);
        let max_sendable = if chain.pays_own_fee(contract) {
            held.saturating_sub(fee)
        } else {
            held
        };
        Ok(SendPreview {
            estimated_network_fee: crate::amount::decimal::from_units(
                fee,
                u32::from(chain.native_decimals()),
            ),
            spendable_balance: units(held),
            max_sendable: units(max_sendable),
            fee_rate_description: Some(
                "Simulated gas at the block's base fee and the suggested tip, paid in VTHO".into(),
            ),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "../tests/send_stage_vechain.rs"]
mod tests;

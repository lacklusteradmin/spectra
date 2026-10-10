//! A Sui wallet's coin objects, and merging a type's objects into one.
//!
//! Sui holds a balance as `Coin<T>` objects, and every receipt adds one. A
//! send has to name each object it spends, so a balance spread over many
//! costs more gas and, past a few hundred objects, more than one
//! transaction. Merging joins a type's objects: SUI by paying gas with every
//! coin, which the network joins into the gas coin; another type with
//! `MergeCoins`, gas paid apart in SUI. Built, dry-run for its cost, and
//! signed and broadcast through the ordinary send stages under
//! [`WalletOperation::MergeCoins`].

use super::*;
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};
use crate::send::sui::{GasCoin, MAX_SUI_MERGE, MAX_TOKEN_MERGE, PreparedSuiMerge, owned_coins};

const SUI: &str = "0x2::sui::SUI";

/// One coin type the wallet holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SuiCoinType {
    pub coin_type: String,
    /// The token's symbol where Spectra knows it, else the type's own name.
    pub symbol: String,
    /// How many `Coin<T>` objects hold it.
    pub objects: u64,
    /// Their total, as an exact decimal, or the raw integer where the
    /// type's decimals could not be read.
    pub balance: String,
    /// Whether merging would join objects: more than one holds it.
    pub mergeable: bool,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The coin types the wallet holds and how many objects hold each, read
    /// from a verified node; SUI first, then by object count.
    pub async fn wallet_coin_objects(
        &self,
        wallet_id: String,
    ) -> Result<Vec<SuiCoinType>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.sui_owner(&wallet_id).await?;
            let client = this.sui_client(chain).await?;
            let state = this.app_state().await;
            let mut types = Vec::new();
            for (coin_type, objects, total) in client.fetch_coin_object_counts(&owner).await? {
                let native = coin_type.ends_with("::sui::SUI");
                let decimals = if native {
                    Some(chain.native_decimals())
                } else {
                    client.fetch_coin_decimals(&coin_type).await
                };
                types.push(SuiCoinType {
                    symbol: if native {
                        chain.coin_symbol().to_string()
                    } else {
                        super::send_records::send_asset_names(&state, chain, Some(&coin_type)).0
                    },
                    balance: match decimals {
                        Some(decimals) => crate::decimal::from_units(total, u32::from(decimals)),
                        None => total.to_string(),
                    },
                    mergeable: objects > 1,
                    coin_type,
                    objects,
                });
            }
            types.sort_by_key(|entry| {
                (
                    !entry.coin_type.ends_with("::sui::SUI"),
                    std::cmp::Reverse(entry.objects),
                )
            });
            Ok(types)
        })
        .await
    }

    /// Build the transaction that merges the wallet's `coin_type` objects
    /// into one, prepared and stored like any send, to be signed and
    /// broadcast through the same stages. Merges at most 256 SUI coins or
    /// 500 of another type's objects at once; another merge takes the rest.
    pub async fn build_coin_merge(
        &self,
        wallet_id: String,
        coin_type: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.sui_owner(&wallet_id).await?;
            if this.stored_wallet(&wallet_id).await?.is_watch_only() {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
            let coin_type = coin_type.trim().to_string();
            let native = coin_type == SUI || coin_type.ends_with("::sui::SUI");
            let client = this.sui_client(chain).await?;
            let gas_price = client.fetch_reference_gas_price().await?;
            let objects = owned_coins(
                &client,
                &owner,
                &coin_type,
                if native {
                    MAX_SUI_MERGE
                } else {
                    MAX_TOKEN_MERGE
                },
            )
            .await?;
            if objects.len() < 2 {
                return Err(SpectraBridgeError::invalid(
                    "One object holds this coin; there is nothing to merge.",
                ));
            }
            // A dry run prices the merge; the budget covers its computation
            // and storage with a fifth to spare, before the storage rebate.
            let provisional: u64 = 50_000_000;
            let prepare = |budget: u64, gas: Vec<GasCoin>| {
                let (gas, merged) = if native {
                    (objects.clone(), Vec::new())
                } else {
                    (gas, objects.clone())
                };
                PreparedSuiMerge::prepare(&owner, &coin_type, budget, gas_price, gas, merged)
            };
            let gas_for = |budget: u64| {
                let client = &client;
                let owner = &owner;
                async move {
                    if native {
                        Ok(Vec::new())
                    } else {
                        crate::send::sui::select_coins(client, owner, SUI, budget).await
                    }
                }
            };
            // The trial pays at most what the wallet's SUI covers.
            let trial_budget = if native {
                provisional.min(objects.iter().map(|coin| coin.balance).sum())
            } else {
                provisional.min(client.fetch_balance(&owner).await?.mist)
            };
            let trial = prepare(trial_budget, gas_for(trial_budget).await?)?;
            let (computation, storage, _rebate) =
                client.dry_run_gas(&trial.transaction.bytes).await?;
            let budget = (computation + storage)
                .saturating_mul(6)
                .div_ceil(5)
                .max(gas_price.saturating_mul(2_000));
            let prepared = prepare(budget, gas_for(budget).await?)?;
            let network_fee =
                crate::decimal::from_units(u128::from(budget), u32::from(chain.native_decimals()));
            let count = prepared.object_count() as u64;
            let signing_payload_hex = hex::encode(&prepared.transaction.bytes);
            let symbol = if native {
                chain.coin_symbol().to_string()
            } else {
                super::send_records::send_asset_names(
                    &this.app_state().await,
                    chain,
                    Some(&coin_type),
                )
                .0
            };
            let request = crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: wallet_id.clone(),
                password: None,
                to_address: owner.clone(),
                amount_str: "0".into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: Some(network_fee.clone()),
                fee_amount: Some(network_fee.clone()),
                evm_overrides: None,
                sign_only: false,
                memo: None,
            };
            let prepared = PreparedPayload::SuiMerge(prepared);
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id,
                    chain_id: chain,
                    sender: owner.clone(),
                    recipient: owner,
                    amount: "0".into(),
                    asset: coin_type.clone(),
                    symbol,
                    staking: None,
                    operation: Some(WalletOperation::MergeCoins {
                        coin_type,
                        objects: count,
                        network_fee,
                    }),
                    created_at: crate::store::now_unix().floor(),
                    review_digest: String::new(),
                    review: SendArtifactReview::default(),
                    prepared_details: serde_json::to_string_pretty(&prepared)?,
                    signing_payload_hex,
                    signed_payload: None,
                    transaction_hash: None,
                    attempts: Vec::new(),
                    selected_endpoints: Vec::new(),
                    memo: None,
                },
                request,
                prepared,
                submission: None,
                signed_digest: None,
                substrate_verified_through: None,
                icp_staking_receipts: vec![],
            };
            stored.view.review_digest = stored.digest()?;
            this.save_send_artifact(&stored, Vec::new()).await?;
            Ok(stored.view)
        })
        .await
    }
}

impl WalletService {
    /// A Sui wallet's network and address, or a refusal.
    async fn sui_owner(&self, wallet_id: &str) -> Result<(Chain, String), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        let chain = wallet.chain_id;
        if chain.mainnet_counterpart() != Chain::Sui {
            return Err(SpectraBridgeError::invalid(
                "Only a Sui wallet holds coin objects.",
            ));
        }
        let owner = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
            .to_string();
        Ok((chain, owner))
    }

    async fn sui_client(&self, chain: Chain) -> Result<SuiClient, SpectraBridgeError> {
        let client = SuiClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        client.verify_network(chain).await?;
        Ok(client)
    }
}

#[cfg(test)]
#[path = "tests/wallet_sui_coins.rs"]
mod tests;

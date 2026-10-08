//! The storage deposits NEAR token contracts hold for an account, and
//! getting them back.
//!
//! A NEP-141 token holds a balance only for an account it has registered,
//! and registering one takes a NEP-145 storage deposit, usually 0.00125
//! NEAR, which the contract keeps until the account unregisters. No node
//! lists every contract an account registered with, so core asks
//! `storage_balance_of` of the contracts the wallet's token inventory,
//! holdings and history name, and lists those holding a deposit while the
//! account holds none of their token. Unregistering is `storage_unregister`
//! through the ordinary send stages under
//! [`WalletOperation::RefundTokenStorage`], one contract a transaction since
//! a NEAR transaction has one receiver. It never passes `force`, with which
//! a contract burns a balance instead of refusing.

use super::*;
use crate::api::error::ApiError;
use crate::send::near::{PreparedNearFunctionCall, STORAGE_UNREGISTER, STORAGE_UNREGISTER_ARGS};
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};
use futures::{StreamExt, TryStreamExt};

/// A token contract holding a storage deposit the account can have back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NearStorageDeposit {
    pub contract: String,
    /// The token's symbol where Spectra knows it, else the contract.
    pub symbol: String,
    /// What unregistering returns, as an exact decimal of NEAR.
    pub refund: String,
}

/// The storage deposits a NEAR wallet's account can have back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NearStorageDeposits {
    pub account: String,
    /// By contract.
    pub deposits: Vec<NearStorageDeposit>,
    /// What they return together, as an exact decimal of NEAR.
    pub refundable: String,
}

const NOT_NEAR: &str = "Only a NEAR account holds token storage deposits.";

/// How many contracts are read at once.
const CONCURRENT_READS: usize = 8;

/// The deposit `contract` would return to `account` on unregistering, or
/// `None` when it holds none, the account holds its token, or it answers
/// neither NEP-145 nor NEP-141.
async fn refundable_deposit(
    node: &NearClient,
    contract: &str,
    account: &str,
) -> Result<Option<u128>, ApiError> {
    let deposit = match node.fetch_storage_balance(contract, account).await {
        Ok(Some(deposit)) if deposit > 0 => deposit,
        Ok(_) | Err(ApiError::Rejected(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    match node.fetch_ft_balance_of(contract, account).await {
        Ok(0) => Ok(Some(deposit)),
        Ok(_) | Err(ApiError::Rejected(_) | ApiError::Decode(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// What unregistering `account` from `contract` needs: a deposit the
/// contract holds, which it returns, and none of its token in the account,
/// which `storage_unregister` without `force` refuses after the gas is spent.
async fn refund_preconditions(
    node: &NearClient,
    contract: &str,
    account: &str,
) -> Result<u128, SpectraBridgeError> {
    let no_deposit =
        || SpectraBridgeError::refused("%@ holds no storage deposit for this account.", [contract]);
    let deposit = match node.fetch_storage_balance(contract, account).await {
        Ok(Some(deposit)) if deposit > 0 => deposit,
        Ok(_) | Err(ApiError::Rejected(_)) => return Err(no_deposit()),
        Err(error) => return Err(error.into()),
    };
    match node.fetch_ft_balance_of(contract, account).await {
        Ok(0) => Ok(deposit),
        Ok(_) => Err(SpectraBridgeError::invalid(
            "The account still holds this token. Send all of it away first: unregistering refuses while the account holds any.",
        )),
        Err(ApiError::Rejected(_) | ApiError::Decode(_)) => Err(SpectraBridgeError::invalid(
            "This contract reports no token balance, so Spectra cannot tell that the account holds none.",
        )),
        Err(error) => Err(error.into()),
    }
}

/// What a refund's build reads, from one verified node.
struct RefundReads {
    deposit: u128,
    fee: u128,
    nonce: u64,
    block_hash: [u8; 32],
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The token contracts holding a storage deposit the wallet's NEAR
    /// account can have back: those its inventory, holdings and history
    /// name that have registered it while it holds none of their token.
    pub async fn wallet_token_storage(
        &self,
        wallet_id: String,
    ) -> Result<NearStorageDeposits, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, chain, account) = this.near_account(&wallet_id, NOT_NEAR).await?;
            let contracts = this.near_token_contracts(&wallet, chain, &account).await?;
            let endpoints = this.near_nodes(chain).await?;
            let found = crate::api::http::race(&endpoints, |endpoint| {
                let (contracts, account) = (contracts.clone(), account.clone());
                async move {
                    let node = Arc::new(NearClient::new(Arc::new(vec![endpoint])));
                    node.verify_network(chain).await?;
                    // Owned per read: a borrowed stream item makes the
                    // future's `Send` depend on every lifetime.
                    let reads: Vec<Option<(String, u128)>> = futures::stream::iter(contracts)
                        .map(|contract| {
                            let (node, account) = (node.clone(), account.clone());
                            async move {
                                Ok::<_, ApiError>(
                                    refundable_deposit(&node, &contract, &account)
                                        .await?
                                        .map(|deposit| (contract, deposit)),
                                )
                            }
                        })
                        .buffered(CONCURRENT_READS)
                        .try_collect()
                        .await?;
                    Ok::<_, ApiError>(reads.into_iter().flatten().collect::<Vec<_>>())
                }
            })
            .await?;
            let decimals = u32::from(chain.native_decimals());
            let refundable = found.iter().map(|(_, deposit)| deposit).sum();
            Ok(NearStorageDeposits {
                deposits: found
                    .into_iter()
                    .map(|(contract, deposit)| NearStorageDeposit {
                        symbol: token_symbol(&wallet, chain, &contract),
                        refund: crate::decimal::from_units(deposit, decimals),
                        contract,
                    })
                    .collect(),
                refundable: crate::decimal::from_units(refundable, decimals),
                account,
            })
        })
        .await
    }

    /// Build the transaction that unregisters the wallet's account from
    /// `contract` and returns its storage deposit, prepared and stored like
    /// any send, to be signed and broadcast through the same stages. A
    /// contract holding no deposit and an account still holding the token
    /// are refused.
    pub async fn build_token_storage_refund(
        &self,
        wallet_id: String,
        contract: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, chain, account) = this.near_account(&wallet_id, NOT_NEAR).await?;
            if wallet.is_watch_only() {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
            let signer = super::wallet_near_keys::near_signing_key(&wallet, &account)?;
            let contract =
                crate::tokens::validate_protocol_identifier(chain, "NEP-141", contract.trim())
                    .map_err(|_| {
                        SpectraBridgeError::refused("%@ is not a NEAR token contract.", [&contract])
                    })?;
            let gas = chain
                .near_token_gas_limit()
                .ok_or_else(|| SpectraBridgeError::failure("Missing NEAR gas limit"))?;
            let endpoints = this.near_nodes(chain).await?;
            let reads = crate::api::http::race(&endpoints, |endpoint| {
                let (contract, account) = (&contract, &account);
                async move {
                    let node = NearClient::new(Arc::new(vec![endpoint]));
                    node.verify_network(chain).await?;
                    let deposit = refund_preconditions(&node, contract, account).await?;
                    let fee = node
                        .function_call_fee_budget(
                            account,
                            contract,
                            STORAGE_UNREGISTER,
                            STORAGE_UNREGISTER_ARGS.len(),
                            gas,
                        )
                        .await?;
                    // The one yoctoNEAR the call attaches leaves the balance
                    // before the deposit comes back.
                    if node.fetch_spendable_balance(account).await? < fee + 1 {
                        return Err(SpectraBridgeError::invalid(
                            "Insufficient spendable NEAR for the network fee",
                        ));
                    }
                    let nonce = node
                        .fetch_full_access_key_nonce(account, &bs58::encode(signer).into_string())
                        .await?
                        .checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::failure("Nonce exhausted"))?;
                    let block_hash = crate::derivation::solana::decode_b58_32(
                        &node.fetch_latest_block_hash().await?,
                    )?;
                    Ok::<_, SpectraBridgeError>(RefundReads {
                        deposit,
                        fee,
                        nonce,
                        block_hash,
                    })
                }
            })
            .await?;
            let prepared = PreparedNearFunctionCall::storage_unregister(
                &account,
                signer,
                reads.nonce,
                &contract,
                gas,
                reads.block_hash,
                reads.fee,
            );
            let decimals = u32::from(chain.native_decimals());
            let network_fee = crate::decimal::from_units(reads.fee, decimals);
            let refund = crate::decimal::from_units(reads.deposit, decimals);
            let request = crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: wallet_id.clone(),
                password: None,
                to_address: account.clone(),
                amount_str: refund.clone(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: Some(network_fee.clone()),
                evm_overrides: None,
                sign_only: false,
                memo: None,
            };
            let signing_payload_hex = hex::encode(&prepared.message);
            let prepared = PreparedPayload::NearFunctionCall(prepared);
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id,
                    chain_id: chain,
                    sender: account.clone(),
                    recipient: account,
                    amount: refund.clone(),
                    asset: chain.coin_symbol().into(),
                    symbol: chain.coin_symbol().into(),
                    staking: None,
                    operation: Some(WalletOperation::RefundTokenStorage {
                        contract,
                        refund,
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
    /// The NEAR nodes this network reads from.
    async fn near_nodes(&self, chain: Chain) -> Result<Vec<String>, SpectraBridgeError> {
        self.api_endpoints(
            chain,
            crate::EndpointApi::NearJsonRpc,
            &[EndpointCapability::Verification],
        )
        .await
    }

    /// The token contracts the wallet's NEP-141 holdings, its history and
    /// its indexer's inventory name, emptied ones included, without
    /// repeats.
    async fn near_token_contracts(
        &self,
        wallet: &crate::store::state::WalletState,
        chain: Chain,
        account: &str,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let prefix = format!("{}:nep-141:", chain.str_id());
        let mut contracts: std::collections::BTreeSet<String> = wallet
            .holdings
            .iter()
            .filter(|holding| holding.chain_id == chain)
            .filter_map(|holding| {
                holding
                    .deployment_id()
                    .strip_prefix(&prefix)
                    .map(Into::into)
            })
            .collect();
        contracts.extend(
            self.transactions_for_wallet(wallet.id.clone())
                .await?
                .into_iter()
                .filter(|record| record.chain_id == chain)
                .filter_map(|record| record.deployment_id?.strip_prefix(&prefix).map(Into::into)),
        );
        let indexer = self
            .listing_endpoints(chain, crate::EndpointApi::Nearblocks)
            .await?;
        if !indexer.is_empty() {
            contracts.extend(
                crate::api::nearblocks::NearblocksClient::new(Arc::new(indexer))
                    .fetch_ft_contracts(account)
                    .await?,
            );
        }
        Ok(contracts.into_iter().collect())
    }

    /// Before signing and before broadcast: the contract still holds the
    /// reviewed deposit, the account still holds none of its token, and the
    /// nonce, fee and spendable balance still fit, all on one verified node.
    pub(super) async fn validate_token_storage_refund(
        &self,
        stored: &StoredSend,
        prepared: &PreparedNearFunctionCall,
    ) -> Result<(), SpectraBridgeError> {
        let Some(WalletOperation::RefundTokenStorage {
            contract, refund, ..
        }) = &stored.view.operation
        else {
            return Err(SpectraBridgeError::invalid(
                "The wallet operation was altered",
            ));
        };
        let chain = stored.view.chain_id;
        let account = &stored.view.sender;
        let reviewed_fee = prepared
            .fee_budget
            .parse::<u128>()
            .map_err(SpectraBridgeError::invalid)?;
        let endpoints = self.near_nodes(chain).await?;
        crate::api::http::race(&endpoints, |endpoint| async move {
            let node = NearClient::new(Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let deposit = refund_preconditions(&node, contract, account).await?;
            if crate::decimal::from_units(deposit, u32::from(chain.native_decimals())) != *refund {
                return Err(SpectraBridgeError::invalid(
                    "The contract's storage deposit changed; build and review again",
                ));
            }
            if node
                .fetch_full_access_key_nonce(
                    account,
                    &bs58::encode(prepared.public_key).into_string(),
                )
                .await?
                .checked_add(1)
                != Some(prepared.nonce)
            {
                return Err(SpectraBridgeError::invalid(
                    "NEAR access-key nonce changed; review again",
                ));
            }
            let fee = node
                .function_call_fee_budget(
                    account,
                    contract,
                    &prepared.method,
                    prepared.args.len(),
                    prepared.gas,
                )
                .await?;
            if fee > reviewed_fee {
                return Err(SpectraBridgeError::invalid(
                    "NEAR fee exceeds reviewed budget; review again",
                ));
            }
            if node.fetch_spendable_balance(account).await? < reviewed_fee + 1 {
                return Err(SpectraBridgeError::invalid(
                    "Insufficient spendable NEAR for the network fee",
                ));
            }
            Ok(())
        })
        .await
    }
}

/// The symbol a holding or the catalog gives `contract`, else the contract.
fn token_symbol(wallet: &crate::store::state::WalletState, chain: Chain, contract: &str) -> String {
    let id = format!("{}:nep-141:{contract}", chain.str_id());
    wallet
        .holdings
        .iter()
        .find(|holding| holding.chain_id == chain && holding.deployment_id() == id)
        .map(|holding| holding.symbol.clone())
        .or_else(|| crate::tokens::deployment(&id).map(|token| token.symbol.clone()))
        .unwrap_or_else(|| contract.to_string())
}

#[cfg(test)]
#[path = "tests/wallet_near_storage.rs"]
mod tests;

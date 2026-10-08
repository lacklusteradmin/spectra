//! The ERC-20 allowances an EVM wallet has granted, and revoking one; and
//! an Ethereum wallet's ENS primary name.
//!
//! An allowance lets a contract move the wallet's tokens without asking
//! again, and outlives the dapp it was given to. Which allowances exist is
//! not a node question: the wallet's `Approval` logs come from an address
//! indexer, and each is confirmed by a live `allowance` read, so a spent or
//! replaced approval is not shown. Revoking is `approve(spender, 0)` on the
//! token, built, signed and broadcast through the ordinary send stages under
//! its own operation ([`WalletOperation::RevokeApproval`]).

use super::*;
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};

/// One allowance the wallet has granted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TokenApproval {
    pub token: String,
    /// The token's symbol, or empty where the contract gives none.
    pub symbol: String,
    pub spender: String,
    /// What the spender may move, as an exact decimal of the token, or the
    /// raw integer where the token's decimals could not be read.
    pub allowance: String,
    /// Whether it is effectively without limit: 2⁹⁶ − 1 or more, where the
    /// maximum allowances wallets grant sit.
    pub unlimited: bool,
}

/// The allowances a wallet has granted, and whether the indexer scan read
/// every approval the wallet logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TokenApprovals {
    pub chain: Chain,
    pub approvals: Vec<TokenApproval>,
    pub complete: bool,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The ERC-20 allowances the wallet has granted and that still stand.
    /// A network with no address indexer is refused rather than answered
    /// with an empty list, which would read as "none".
    pub async fn wallet_token_approvals(
        &self,
        wallet_id: String,
    ) -> Result<TokenApprovals, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.evm_owner(&wallet_id, APPROVALS_REFUSAL).await?;
            let indexers = this
                .api_endpoints(chain, crate::EndpointApi::Blockscout, &[EndpointCapability::History])
                .await?;
            if indexers.is_empty() {
                return Err(SpectraBridgeError::refused(
                    "%@ needs an address indexer to find token approvals. Add a Blockscout-compatible endpoint for it.",
                    [chain.chain_display_name()],
                ));
            }
            let client = crate::api::blockscout::BlockscoutClient::new();
            let logs = crate::api::http::race(&indexers, |base| {
                let client = &client;
                let owner = &owner;
                async move {
                    client
                        .fetch_approval_logs(owner, crate::registry::EvmHistorySource::Open(&base))
                        .await
                }
            })
            .await?;
            let mut pairs: Vec<(String, String)> = Vec::new();
            for log in logs.logs {
                let pair = (log.token, log.spender);
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
            let rpc = EvmClient::new(
                this.endpoints_for(chain, &[EndpointCapability::TokenBalance]).await,
                chain.evm_chain_id()?,
            );
            let allowances = rpc.fetch_erc20_allowances(&owner, &pairs).await?;
            let unlimited_from = (num_bigint::BigUint::from(1u8) << 96u32) - 1u8;
            // Only a live read says an approval stands: one whose allowance
            // cannot be read is not shown as granted.
            let standing: Vec<((String, String), num_bigint::BigUint)> = pairs
                .into_iter()
                .zip(allowances)
                .filter_map(|(pair, allowance)| allowance.ok().map(|allowance| (pair, allowance)))
                .filter(|(_, allowance)| *allowance != num_bigint::BigUint::ZERO)
                .collect();
            let mut tokens: Vec<String> = Vec::new();
            for ((token, _), _) in &standing {
                if !tokens.contains(token) {
                    tokens.push(token.clone());
                }
            }
            let metadata: HashMap<String, Option<crate::api::evm_json_rpc::Erc20Metadata>> = tokens
                .iter()
                .cloned()
                .zip(rpc.fetch_erc20_metadata_many(&tokens).await?)
                .collect();
            let approvals = standing
                .into_iter()
                .map(|((token, spender), allowance)| {
                    let meta = metadata.get(&token).cloned().flatten();
                    TokenApproval {
                        allowance: match &meta {
                            Some(meta) => crate::decimal::from_unit_digits(
                                &allowance.to_string(),
                                u32::from(meta.decimals),
                            )
                            .unwrap_or_else(|| allowance.to_string()),
                            None => allowance.to_string(),
                        },
                        unlimited: allowance >= unlimited_from,
                        symbol: meta.map(|meta| meta.symbol).unwrap_or_default(),
                        token,
                        spender,
                    }
                })
                .collect();
            Ok(TokenApprovals {
                chain,
                approvals,
                complete: logs.complete,
            })
        })
        .await
    }

    /// Build the transaction that sets the wallet's allowance for `spender`
    /// on `token` back to zero: `approve(spender, 0)`, prepared and stored
    /// like any send, to be signed and broadcast through the same stages.
    /// Refuses an allowance that is already zero.
    pub async fn build_approval_revocation(
        &self,
        wallet_id: String,
        token: String,
        spender: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.evm_owner(&wallet_id, APPROVALS_REFUSAL).await?;
            let state = this.app_state().await;
            if state
                .wallets
                .iter()
                .find(|wallet| wallet.id == wallet_id)
                .is_some_and(|wallet| wallet.is_watch_only())
            {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
            for address in [&token, &spender] {
                if !crate::send::flow::is_valid_send_address(chain, address.clone()) {
                    return Err(SpectraBridgeError::refused(
                        "Not an address on %@: %@",
                        [chain.chain_display_name(), address.as_str()],
                    ));
                }
            }
            let token = token.to_ascii_lowercase();
            let spender = spender.to_ascii_lowercase();
            let rpc = EvmClient::new(
                this.endpoints_for(chain, &[EndpointCapability::TokenBalance])
                    .await,
                chain.evm_chain_id()?,
            );
            let standing = rpc
                .fetch_erc20_allowances(&owner, &[(token.clone(), spender.clone())])
                .await?
                .pop()
                .ok_or_else(|| SpectraBridgeError::failure("allowance read returned nothing"))??;
            if standing == num_bigint::BigUint::ZERO {
                return Err(SpectraBridgeError::invalid(
                    "Nothing to revoke: this spender's allowance is already zero.",
                ));
            }
            let request = crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: wallet_id.clone(),
                password: None,
                to_address: token.clone(),
                amount_str: "0".into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                sign_only: false,
            };
            let data = crate::send::evm::encode_erc20_approve(&spender, 0)?;
            let overrides = crate::send::evm::EvmSendOverrides {
                nonce: Some(this.next_send_nonce(chain, &owner).await?),
                ..Default::default()
            };
            let endpoints = this.endpoints_for(chain, &[EndpointCapability::Fee]).await;
            let prepared = crate::api::http::race(&endpoints, |endpoint| {
                let this = &this;
                let owner = &owner;
                let token = &token;
                let data = &data;
                let overrides = &overrides;
                async move {
                    this.validate_endpoint_network(chain, &endpoint).await?;
                    crate::send::evm::prepare_transfer(
                        &EvmClient::new(Arc::new(vec![endpoint]), chain.evm_chain_id()?),
                        owner,
                        token,
                        0,
                        data,
                        overrides,
                    )
                    .await
                }
            })
            .await?;
            this.validate_evm_funds(chain, &owner, &prepared).await?;
            let symbol = rpc
                .fetch_erc20_metadata(&token)
                .await
                .map(|meta| meta.symbol)
                .ok()
                .filter(|symbol| !symbol.is_empty())
                .unwrap_or_else(|| {
                    super::send_records::send_asset_names(&state, chain, Some(&token)).0
                });
            let network_fee = crate::decimal::from_units(
                prepared.maximum_fee_wei()?,
                u32::from(chain.native_decimals()),
            );
            let prepared = PreparedPayload::Evm(prepared);
            let signing_payload_hex = match &prepared {
                PreparedPayload::Evm(p) => hex::encode(p.signing_payload()?),
                _ => unreachable!("the revocation was prepared as an EVM call"),
            };
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id,
                    chain_id: chain,
                    sender: owner,
                    recipient: token.clone(),
                    amount: "0".into(),
                    asset: token.clone(),
                    symbol,
                    staking: None,
                    operation: Some(WalletOperation::RevokeApproval {
                        token,
                        spender,
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

    /// The Ethereum wallet's ENS primary name, shown only when it resolves
    /// back to the wallet's address. `None` on every other network.
    pub async fn wallet_ens_name(
        &self,
        wallet_id: String,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this
                .evm_owner(&wallet_id, "Only an EVM wallet has an ENS name.")
                .await?;
            if !chain.resolves_ens_names() {
                return Ok(None);
            }
            let rpc = EvmClient::new(
                this.endpoints_for(chain, &[EndpointCapability::Verification])
                    .await,
                chain.evm_chain_id()?,
            );
            Ok(rpc.fetch_ens_primary_name(&owner).await?)
        })
        .await
    }
}

const APPROVALS_REFUSAL: &str = "Only an EVM wallet grants token approvals.";

impl WalletService {
    /// An EVM wallet's network and address, or `refusal` for any other.
    pub(super) async fn evm_owner(
        &self,
        wallet_id: &str,
        refusal: &'static str,
    ) -> Result<(Chain, String), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        let chain = wallet.chain_id;
        if !chain.is_evm() {
            return Err(SpectraBridgeError::invalid(refusal));
        }
        let owner = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
            .to_ascii_lowercase();
        Ok((chain, owner))
    }
}

#[cfg(test)]
#[path = "tests/wallet_approvals.rs"]
mod tests;

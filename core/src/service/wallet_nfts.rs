//! The ERC-721 and ERC-1155 tokens an EVM wallet holds, and sending one.
//!
//! An NFT is a collection's contract and a token id; an ERC-1155 holding is
//! a quantity of one. Neither is a balance with decimals or a holding in the
//! portfolio. Which NFTs an address holds is an indexer question, answered
//! by a Blockscout instance's inventory. Sending one is the collection's
//! `safeTransferFrom`, built only after the standard is read from the
//! contract (ERC-165) and the wallet's ownership or quantity is read live,
//! and signed and broadcast through the ordinary send stages under
//! [`WalletOperation::TransferNft`]. Ownership is read again before signing.

use super::*;
use crate::api::evm_nft::NftStandard;
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};

const NFT_REFUSAL: &str = "Only an EVM wallet holds ERC-721 and ERC-1155 tokens.";
const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

/// One NFT a wallet holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletNft {
    pub standard: NftStandard,
    /// The collection's contract, lowercase.
    pub contract: String,
    /// The token's id: a whole number in decimal, of any size.
    pub token_id: String,
    /// How many the wallet holds, a whole number: one for an ERC-721 token.
    pub quantity: String,
    /// The collection's name, or empty.
    pub collection: String,
    /// The collection's symbol, or `NFT`.
    pub symbol: String,
    /// The token's own name from its metadata, where it has one.
    pub name: Option<String>,
}

/// The NFTs a wallet holds, and whether the inventory was read to its end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletNfts {
    pub chain: Chain,
    pub nfts: Vec<WalletNft>,
    pub complete: bool,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The NFTs the wallet holds, from its network's Blockscout inventory. A
    /// network with no explorer that keeps one is refused rather than
    /// answered with an empty list, which would read as "none".
    pub async fn wallet_nfts(&self, wallet_id: String) -> Result<WalletNfts, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.evm_owner(&wallet_id, NFT_REFUSAL).await?;
            let explorers: Vec<String> = this
                .api_endpoints(
                    chain,
                    crate::EndpointApi::Blockscout,
                    &[EndpointCapability::TokenDiscovery],
                )
                .await?
                .into_iter()
                .filter(|base| crate::api::blockscout::serves_nft_inventory(base))
                .collect();
            if explorers.is_empty() {
                return Err(SpectraBridgeError::refused(
                    "%@ needs a Blockscout explorer to list NFTs. Add one for it.",
                    [chain.chain_display_name()],
                ));
            }
            let client = crate::api::blockscout::BlockscoutClient::new();
            let (holdings, complete) = crate::api::http::race(&explorers, |base| {
                let client = &client;
                let owner = &owner;
                async move { client.fetch_nft_inventory(owner, &base).await }
            })
            .await?;
            Ok(WalletNfts {
                chain,
                nfts: holdings
                    .into_iter()
                    .map(|holding| WalletNft {
                        standard: holding.standard,
                        contract: holding.contract,
                        token_id: holding.token_id,
                        quantity: holding.quantity,
                        collection: holding.collection,
                        symbol: crate::tokens::nft_symbol(&holding.symbol),
                        name: holding.name,
                    })
                    .collect(),
                complete,
            })
        })
        .await
    }

    /// Build the transaction that sends `quantity` of the NFT `token_id` of
    /// `contract` to `recipient`: the collection's `safeTransferFrom`,
    /// prepared and stored like any send, to be signed and broadcast
    /// through the same stages. The standard is read from the contract and
    /// the wallet's ownership or quantity is read live; an ERC-721 token is
    /// sent with a quantity of one.
    pub async fn build_nft_transfer(
        &self,
        wallet_id: String,
        contract: String,
        token_id: String,
        quantity: String,
        recipient: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.evm_owner(&wallet_id, NFT_REFUSAL).await?;
            if this.stored_wallet(&wallet_id).await?.is_watch_only() {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
            for address in [&contract, &recipient] {
                if !crate::send::flow::is_valid_send_address(chain, address.clone()) {
                    return Err(SpectraBridgeError::refused(
                        "Not an address on %@: %@",
                        [chain.chain_display_name(), address.as_str()],
                    ));
                }
            }
            let contract = contract.to_ascii_lowercase();
            let recipient = recipient.to_ascii_lowercase();
            if recipient == owner {
                return Err(SpectraBridgeError::invalid(
                    "The recipient is this wallet's own address.",
                ));
            }
            if recipient == ZERO_ADDRESS {
                return Err(SpectraBridgeError::invalid(
                    "An NFT cannot be sent to the zero address.",
                ));
            }
            let token_id = crate::api::evm_nft::canonical_uint256(&token_id)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid token id"))?;
            let quantity = crate::api::evm_nft::canonical_uint256(&quantity)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid quantity"))?;
            let rpc = EvmClient::new(
                this.endpoints_for(chain, &[EndpointCapability::TokenBalance])
                    .await,
                chain.evm_chain_id()?,
            );
            let standard = rpc.fetch_nft_standard(&contract).await?.ok_or_else(|| {
                SpectraBridgeError::refused(
                    "%@ is not an ERC-721 or ERC-1155 collection.",
                    [contract.as_str()],
                )
            })?;
            validate_nft_holding(&rpc, standard, &contract, &owner, &token_id, &quantity).await?;
            let data = crate::send::evm::encode_nft_transfer(
                standard, &owner, &recipient, &token_id, &quantity,
            )?;
            let overrides = crate::send::evm::EvmSendOverrides {
                nonce: Some(this.next_send_nonce(chain, &owner).await?),
                ..Default::default()
            };
            let endpoints = this.endpoints_for(chain, &[EndpointCapability::Fee]).await;
            let prepared = crate::api::http::race(&endpoints, |endpoint| {
                let this = &this;
                let owner = &owner;
                let contract = &contract;
                let data = &data;
                let overrides = &overrides;
                async move {
                    this.validate_endpoint_network(chain, &endpoint).await?;
                    crate::send::evm::prepare_transfer(
                        &EvmClient::new(Arc::new(vec![endpoint]), chain.evm_chain_id()?),
                        owner,
                        contract,
                        0,
                        data,
                        overrides,
                    )
                    .await
                }
            })
            .await?;
            this.validate_evm_funds(chain, &owner, &prepared).await?;
            // Display only: a collection without `name()` or `symbol()` is
            // still sent, under its contract and `NFT`.
            let (collection, symbol) = rpc
                .fetch_collection_label(&contract)
                .await
                .unwrap_or_default();
            let network_fee = crate::decimal::from_units(
                prepared.maximum_fee_wei()?,
                u32::from(chain.native_decimals()),
            );
            let signing_payload_hex = hex::encode(prepared.signing_payload()?);
            let request = crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: wallet_id.clone(),
                password: None,
                to_address: recipient.clone(),
                amount_str: quantity.clone(),
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
            let prepared = PreparedPayload::Evm(prepared);
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id,
                    chain_id: chain,
                    sender: owner,
                    recipient,
                    amount: quantity.clone(),
                    asset: contract.clone(),
                    symbol: crate::tokens::nft_symbol(&symbol),
                    staking: None,
                    operation: Some(WalletOperation::TransferNft {
                        contract,
                        standard,
                        token_id,
                        quantity,
                        collection,
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
}

impl WalletService {
    /// Before signing an NFT transfer: the wallet still owns the token, or
    /// still holds the quantity, read live again.
    pub(super) async fn validate_nft_transfer_state(
        &self,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let Some(WalletOperation::TransferNft {
            contract,
            standard,
            token_id,
            quantity,
            ..
        }) = &stored.view.operation
        else {
            return Ok(());
        };
        let chain = stored.view.chain_id;
        let rpc = EvmClient::new(
            self.endpoints_for(chain, &[EndpointCapability::TokenBalance])
                .await,
            chain.evm_chain_id()?,
        );
        validate_nft_holding(
            &rpc,
            *standard,
            contract,
            &stored.view.sender,
            token_id,
            quantity,
        )
        .await
    }
}

/// That `owner` can send `quantity` of `token_id`: owns the ERC-721 token
/// (and sends exactly one), or holds at least `quantity` of the ERC-1155 id.
async fn validate_nft_holding(
    rpc: &EvmClient,
    standard: NftStandard,
    contract: &str,
    owner: &str,
    token_id: &str,
    quantity: &str,
) -> Result<(), SpectraBridgeError> {
    let wanted = crate::api::evm_nft::parse_uint256(quantity)?;
    match standard {
        NftStandard::Erc721 => {
            if wanted != num_bigint::BigUint::from(1u8) {
                return Err(SpectraBridgeError::invalid(
                    "An ERC-721 token is sent one at a time.",
                ));
            }
            if !rpc
                .fetch_erc721_owner(contract, token_id)
                .await?
                .is_some_and(|holder| holder.eq_ignore_ascii_case(owner))
            {
                return Err(SpectraBridgeError::refused(
                    "This wallet does not own token %@ of this collection.",
                    [token_id],
                ));
            }
        }
        NftStandard::Erc1155 => {
            if wanted == num_bigint::BigUint::ZERO {
                return Err(SpectraBridgeError::invalid(
                    "The quantity must be at least one.",
                ));
            }
            let held = rpc.fetch_erc1155_balance(contract, owner, token_id).await?;
            if held < wanted {
                return Err(SpectraBridgeError::refused(
                    "This wallet holds %@ of token %@, fewer than %@.",
                    [held.to_string(), token_id.to_string(), quantity.to_string()],
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/wallet_nfts.rs"]
mod tests;

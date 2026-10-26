//! Network history: which adapter reads a chain's history, and the one
//! shape every chain's transfers are stored in.
use super::*;
use crate::api::endpoint::EndpointApi;
use crate::api::history_page::{HistoryAmount, HistoryEntry, HistoryOutcome};
use crate::store::wallet_domain::{TransactionKind, TransactionStatus};

/// One transaction of a chain's history in the shape every chain shares, with
/// nothing chain-specific left to read.
#[derive(Debug, Clone, uniffi::Record)]
// Serialized only to compare rows against the test fixtures.
#[cfg_attr(test, derive(serde::Serialize))]
pub struct NormalizedHistoryItem {
    pub deployment_id: Option<String>,
    pub kind: TransactionKind,
    pub status: TransactionStatus,
    pub asset_display_name: String,
    pub symbol: String,
    pub chain_id: crate::registry::Chain,
    /// The magnitude, as an exact decimal; `kind` says which way it went.
    pub amount: String,
    pub counterparty: String,
    pub tx_hash: String,
    pub block_height: Option<i64>,
    /// Unix seconds.
    pub timestamp: f64,
}

/// An adapter's entry as `chain` stores it: its asset named by the contract
/// it carries, never by a ticker, and its amount at that asset's precision.
///
/// A contract that does not normalize for `chain`, an amount that is not a
/// decimal and a time that is not a positive number of seconds each refuse
/// the row; a time the chain has not given yet is stored as the unknown 0.
pub(crate) fn normalize_history_entry(
    chain: Chain,
    entry: HistoryEntry,
) -> Option<NormalizedHistoryItem> {
    let deployment_id =
        crate::catalog::tokens::deployment_id_for(chain, entry.contract.as_deref())?;
    let (asset_display_name, symbol) = match &entry.contract {
        None => (
            chain.coin_name().to_string(),
            chain.coin_symbol().to_string(),
        ),
        Some(contract) => match crate::catalog::tokens::deployment(&deployment_id) {
            Some(token) => (token.name.clone(), token.symbol.clone()),
            // An unregistered contract cannot borrow another asset's name or
            // ticker; it is shown as itself.
            None => (contract.clone(), contract.clone()),
        },
    };
    let amount = match entry.amount {
        HistoryAmount::Units(units) => {
            crate::amount::decimal::from_units(units, chain.native_decimals().into())
        }
        HistoryAmount::Decimal(text) => {
            crate::amount::decimal::canonical(text.trim().trim_start_matches('-'))?
        }
    };
    let timestamp = match entry.timestamp {
        None => 0.0,
        Some(seconds) => Some(seconds).filter(|s| s.is_finite() && *s > 0.0)?,
    };
    Some(NormalizedHistoryItem {
        deployment_id: Some(deployment_id),
        kind: if entry.incoming {
            TransactionKind::Receive
        } else {
            TransactionKind::Send
        },
        status: match entry.outcome {
            HistoryOutcome::Pending => TransactionStatus::Pending,
            HistoryOutcome::Confirmed => TransactionStatus::Confirmed,
            HistoryOutcome::Failed => TransactionStatus::Failed,
        },
        asset_display_name,
        symbol,
        chain_id: chain,
        amount,
        counterparty: entry.counterparty,
        tx_hash: entry.txid,
        block_height: entry.block_height,
        timestamp,
    })
}

/// Pagination describes the provider page before unknown tokens are filtered.
#[derive(Debug)]
pub(crate) struct EvmHistoryPage {
    pub decoded: crate::service::history_decode::EvmHistoryPageDecoded,
    pub exhausted: bool,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch history for `address` on `chain_id` and normalize the raw
    /// chain-specific shape into a standard `NormalizedHistoryItem` array,
    /// returning typed records directly across the FFI boundary.
    pub async fn fetch_normalized_history(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
    ) -> Result<Vec<NormalizedHistoryItem>, SpectraBridgeError> {
        let this = self.clone();
        crate::service::worker::run(async move {
            let this = &this;
            Ok(this
                .fetch_normalized_history_page(chain_id, &address, None)
                .await?
                .items)
        })
        .await
    }
}
impl WalletService {
    /// Whether `address` has any history on `chain`: the first page holds a
    /// transfer, whatever it is.
    pub(crate) async fn has_history(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<bool, SpectraBridgeError> {
        Ok(!self
            .fetch_history_entries(chain, address, None)
            .await?
            .items
            .is_empty())
    }
    pub(crate) async fn fetch_normalized_history_page(
        &self,
        chain: Chain,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<NormalizedHistoryItem>, SpectraBridgeError> {
        Ok(self
            .fetch_history_entries(chain, address, cursor)
            .await?
            .filter_map(|entry| normalize_history_entry(chain, entry)))
    }
    pub(crate) async fn fetch_history_entries(
        &self,
        chain: Chain,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<HistoryEntry>, SpectraBridgeError> {
        if chain.mainnet_counterpart() == Chain::Monero {
            return Ok(crate::api::HistoryPage {
                items: self.monero_history(chain, address).await?,
                next_cursor: None,
            });
        }
        fetch_history_page(address, chain, cursor, self).await
    }
    /// Fetch one page of EVM transaction history for `address` from the
    /// configured Etherscan-compatible explorers:
    ///   1. `txlist` — native ETH/EVM transfers
    ///   2. `tokentx` — ERC-20 token transfers
    ///   3. `tokennfttx` and `token1155tx` — ERC-721 and ERC-1155 transfers
    ///
    /// `tokens` lists the known tokens to include. Only transfers whose
    /// contract matches a known token are returned; pass an empty list to
    /// skip token transfers entirely. Every NFT transfer of the address is
    /// returned: each names its own token, with no list to match.
    pub(crate) async fn fetch_evm_history_page(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
        tokens: Vec<TokenDescriptor>,
        page: u32,
        page_size: u32,
    ) -> Result<EvmHistoryPage, SpectraBridgeError> {
        use crate::service::history_decode::{
            EvmHistoryPageDecoded, EvmNativeTransferItem, EvmNftTransferItem, EvmTokenTransferItem,
        };

        // Only EVM chains are supported.
        let chain = evm_network(chain_id)?;
        for token in &tokens {
            let standard = if token.standard.is_empty() {
                chain.token_standard_for_identifier(&token.contract)
            } else {
                &token.standard
            };
            crate::catalog::tokens::validate_protocol_identifier(chain, standard, &token.contract)?;
        }

        // History is served by indexers, independently for native and token transfers.
        let client = crate::api::blockscout::BlockscoutClient::new();
        let sources = self
            .api_endpoints(
                chain,
                EndpointApi::Blockscout,
                &[EndpointCapability::History],
            )
            .await?;
        if sources.is_empty() {
            return Err(SpectraBridgeError::failure(
                "no explorer configured for this chain",
            ));
        }
        let native_entries = crate::api::http::race(&sources, |base| {
            let client = &client;
            let address = &address;
            async move {
                client
                    .fetch_history(
                        address,
                        crate::registry::EvmHistorySource::Open(&base),
                        page,
                        page_size,
                    )
                    .await
            }
        })
        .await?;
        let token_sources = self
            .api_endpoints(
                chain,
                EndpointApi::Blockscout,
                &[EndpointCapability::TokenHistory],
            )
            .await?;
        let (raw_tokens, tokens_full) = if tokens.is_empty() {
            (vec![], false)
        } else {
            crate::api::http::race(&token_sources, |base| {
                let client = &client;
                let address = &address;
                async move {
                    client
                        .fetch_token_transfers(
                            address,
                            crate::registry::EvmHistorySource::Open(&base),
                            page,
                            page_size,
                        )
                        .await
                }
            })
            .await?
        };
        let mut raw_nfts = Vec::new();
        let mut nfts_full = false;
        if !token_sources.is_empty() {
            for standard in [
                crate::api::evm_nft::NftStandard::Erc721,
                crate::api::evm_nft::NftStandard::Erc1155,
            ] {
                let (entries, full) = crate::api::http::race(&token_sources, |base| {
                    let client = &client;
                    let address = &address;
                    async move {
                        client
                            .fetch_nft_transfers(
                                address,
                                crate::registry::EvmHistorySource::Open(&base),
                                standard,
                                page,
                                page_size,
                            )
                            .await
                    }
                })
                .await?;
                raw_nfts.extend(entries);
                nfts_full |= full;
            }
        }

        let page_size = page_size.clamp(1, 500) as usize;
        let exhausted = native_entries.len() < page_size && !tokens_full && !nfts_full;

        // Build a lookup map from contract address (lowercased) → known token metadata.
        let addr_lower = address.to_lowercase();
        let token_map: std::collections::HashMap<String, (String, String, u8, String)> = tokens
            .iter()
            .map(|t| {
                (
                    t.contract.to_lowercase(),
                    (
                        t.symbol.clone(),
                        t.name.clone().unwrap_or_default(),
                        t.decimals,
                        if t.standard.is_empty() {
                            chain.token_standard_for_identifier(&t.contract).into()
                        } else {
                            t.standard.clone()
                        },
                    ),
                )
            })
            .collect();

        let tokens_decoded: Vec<EvmTokenTransferItem> = raw_tokens
            .into_iter()
            .filter_map(|entry| {
                let (symbol, token_name, decimals, standard) =
                    token_map.get(&entry.contract)?.clone();
                if entry.from != addr_lower && entry.to != addr_lower {
                    return None;
                }
                Some(EvmTokenTransferItem {
                    standard,
                    contract_address: entry.contract,
                    token_name,
                    symbol,
                    from_address: entry.from,
                    to_address: entry.to,
                    amount_decimal: crate::amount::decimal::from_unit_digits(
                        &entry.amount_raw,
                        u32::from(decimals),
                    )?,
                    transaction_hash: entry.txid,
                    block_number: entry.block_number as i64,
                    timestamp: entry.timestamp as f64,
                })
            })
            .collect();

        let native_decoded = native_entries
            .into_iter()
            .map(|e| {
                // A value that is not an integer is not a transfer of nothing.
                let amount_decimal = crate::amount::decimal::from_unit_digits(&e.value_wei, 18)
                    .ok_or_else(|| {
                        SpectraBridgeError::failure(format!("transfer {}: malformed value", e.txid))
                    })?;
                let status = crate::store::wallet_domain::TransactionStatus::from_raw(&e.status)
                    .ok_or_else(|| {
                        SpectraBridgeError::failure(format!("transfer {}: unknown status", e.txid))
                    })?;
                Ok(EvmNativeTransferItem {
                    status,
                    // Compared against the lowercased wallet address, as token
                    // transfers already are; an explorer may answer checksummed.
                    from_address: e.from.to_lowercase(),
                    to_address: e.to.to_lowercase(),
                    amount_decimal,
                    transaction_hash: e.txid,
                    block_number: e.block_number as i64,
                    timestamp: e.timestamp as f64,
                })
            })
            .collect::<Result<Vec<_>, SpectraBridgeError>>()?;

        let nfts_decoded = raw_nfts
            .into_iter()
            .map(|entry| EvmNftTransferItem {
                standard: entry.standard,
                contract_address: entry.contract,
                token_id: entry.token_id,
                quantity: entry.quantity,
                collection: entry.collection,
                symbol: entry.symbol,
                from_address: entry.from,
                to_address: entry.to,
                transaction_hash: entry.txid,
                block_number: entry.block_number as i64,
                timestamp: entry.timestamp as f64,
            })
            .collect();

        Ok(EvmHistoryPage {
            decoded: EvmHistoryPageDecoded {
                tokens: tokens_decoded,
                nfts: nfts_decoded,
                native: native_decoded,
            },
            exhausted,
        })
    }
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct AssetCursor {
    native: Option<String>,
    token: Option<String>,
    native_done: bool,
    token_done: bool,
}
impl AssetCursor {
    fn next(&self) -> Result<Option<String>, SpectraBridgeError> {
        Ok((!(self.native_done && self.token_done))
            .then(|| serde_json::to_string(self))
            .transpose()?)
    }
}
async fn fetch_history_page(
    address: &str,
    chain: Chain,
    cursor: Option<&str>,
    service: &WalletService,
) -> Result<crate::api::HistoryPage<HistoryEntry>, SpectraBridgeError> {
    use EndpointApi as Api;
    let requirements = &[EndpointCapability::History];
    if chain.uses_utxo_client() {
        return Ok(service
            .utxo_client(chain, requirements)
            .await
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::utxo::UtxoHistoryEntry::history_entry));
    }
    if chain.is_evm() {
        let sources = service
            .api_endpoints(chain, Api::Blockscout, requirements)
            .await?;
        let client = crate::api::blockscout::BlockscoutClient::new();
        let page = crate::api::history_page::page_number(cursor)?;
        let items = crate::api::http::race(&sources, |base| {
            let client = &client;
            async move {
                client
                    .fetch_history(
                        address,
                        crate::registry::EvmHistorySource::Open(&base),
                        page,
                        50,
                    )
                    .await
            }
        })
        .await?;
        let next_cursor = (items.len() == 50).then(|| (page + 1).to_string());
        return Ok(crate::api::HistoryPage { items, next_cursor }
            .filter_map(crate::api::blockscout::EvmHistoryEntry::history_entry));
    }
    if chain.mainnet_counterpart() == Chain::Aptos {
        let endpoints = service
            .api_endpoints(chain, Api::AptosIndexer, requirements)
            .await?;
        let (_, nodes) = service
            .fetch_endpoints(chain, &[EndpointCapability::Balance])
            .await?;
        let expected = chain
            .aptos_chain_id()
            .ok_or_else(|| SpectraBridgeError::failure("missing Aptos chain id"))?;
        return Ok(crate::api::aptos_indexer::AptosIndexerClient::new(
            Arc::new(endpoints),
            expected,
        )
        .fetch_history_page(address, cursor, &AptosClient::new(nodes))
        .await?);
    }
    if chain.mainnet_counterpart() == Chain::Tron {
        let node_endpoints = service
            .endpoints_for(
                chain,
                &[
                    EndpointCapability::TokenBalance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        let accounts = service
            .tron_account_endpoints(
                chain,
                &node_endpoints,
                &[
                    EndpointCapability::History,
                    EndpointCapability::TokenHistory,
                ],
            )
            .await?;
        return Ok(crate::api::trongrid_v1::TrongridClient::with_trc10(
            Arc::new(accounts),
            chain,
            TronHttpClient::new(node_endpoints),
        )
        .fetch_history_page(address, 50, cursor)
        .await?
        .filter_map(crate::api::trongrid_v1::TronTransfer::history_entry));
    }
    let (api, endpoints) = service.fetch_endpoints(chain, requirements).await?;
    match api {
        Api::SolanaJsonRpc => Ok(SolanaClient::new(endpoints)
            .fetch_unified_history_page(address, 50, cursor)
            .await?
            .filter_map(crate::api::solana_json_rpc::SolanaTransfer::history_entry)),
        Api::Horizon => Ok(HorizonClient::new(endpoints)
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::horizon::StellarHistoryEntry::history_entry)),
        Api::XrplJsonRpc => Ok(XrplClient::new(endpoints)
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::xrpl_json_rpc::XrpHistoryEntry::history_entry)),
        Api::Koios => Ok(KoiosClient::new(endpoints)
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::koios::CardanoHistoryEntry::history_entry)),
        Api::SubstrateJsonRpc => Err(SpectraBridgeError::failure(format!(
            "{}: no keyless history source configured",
            chain.chain_display_name()
        ))),
        Api::SuiJsonRpc => Ok(SuiClient::new(endpoints)
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::sui_json_rpc::SuiHistoryEntry::history_entry)),
        Api::ToncenterV2 => {
            let mut position: AssetCursor = cursor
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default();
            let mut items = Vec::new();
            if !position.native_done {
                let page = ToncenterV2Client::new(endpoints)
                    .fetch_history_page(address, position.native.as_deref())
                    .await?
                    .filter_map(crate::api::toncenter_v2::TonHistoryEntry::history_entry);
                items.extend(page.items);
                position.native_done = page.next_cursor.is_none();
                position.native = page.next_cursor;
            }
            if !position.token_done {
                let sources = service
                    .api_endpoints(chain, Api::ToncenterV3, &[EndpointCapability::TokenHistory])
                    .await?;
                let owner = crate::derivation::ton::parse_ton_address(address)?;
                let raw = format!("{}:{}", owner.workchain, hex::encode(owner.account_id));
                let page = crate::api::toncenter_v3::ToncenterV3Client::new(Arc::new(sources))
                    .fetch_jetton_history_page(chain, &raw, position.token.as_deref())
                    .await?;
                items.extend(page.items);
                position.token_done = page.next_cursor.is_none();
                position.token = page.next_cursor;
            }
            Ok(crate::api::HistoryPage {
                items,
                next_cursor: position.next()?,
            })
        }
        Api::NearJsonRpc => {
            let mut position: AssetCursor = cursor
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default();
            let mut items = Vec::new();
            if !position.native_done {
                let indexers = service
                    .api_endpoints(chain, Api::Nearblocks, requirements)
                    .await?;
                let page = crate::api::nearblocks::NearblocksClient::new(Arc::new(indexers))
                    .fetch_history_page(address, position.native.as_deref())
                    .await?
                    .filter_map(crate::api::nearblocks::NearHistoryEntry::history_entry);
                items.extend(page.items);
                position.native_done = page.next_cursor.is_none();
                position.native = page.next_cursor;
            }
            if !position.token_done {
                let indexers = service
                    .api_endpoints(chain, Api::Nearblocks, &[EndpointCapability::TokenHistory])
                    .await?;
                let page = crate::api::nearblocks::NearblocksClient::new(Arc::new(indexers))
                    .fetch_ft_history_page(address, position.token.as_deref())
                    .await?;
                items.extend(page.items);
                position.token_done = page.next_cursor.is_none();
                position.token = page.next_cursor;
            }
            Ok(crate::api::HistoryPage {
                items,
                next_cursor: position.next()?,
            })
        }
        Api::IcpRosetta => Ok(IcpClient::new(endpoints)
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::icp_rosetta::IcpHistoryEntry::history_entry)),
        Api::Insight => Ok(InsightClient::new(endpoints)
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::insight::DcrHistoryEntry::history_entry)),
        Api::KaspaRest => Ok(KaspaClient::new(endpoints)
            .fetch_history_page(address, cursor)
            .await?
            .filter_map(crate::api::kaspa_rest::KasHistoryEntry::history_entry)),
        c => Err(SpectraBridgeError::failure(format!(
            "unsupported history API: {c:?}"
        ))),
    }
}

#[cfg(test)]
#[path = "tests/network_history.rs"]
mod tests;

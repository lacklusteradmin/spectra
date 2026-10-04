//! Network history: service adapters and dispatch.
use super::*;

/// Pagination describes the provider page before unknown tokens are filtered.
#[derive(Debug)]
pub(crate) struct EvmHistoryPage {
    pub decoded: crate::fetch::history_decode::EvmHistoryPageDecoded,
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
    ) -> Result<Vec<crate::fetch::history_decode::NormalizedHistoryItem>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
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
    pub(crate) async fn fetch_history(
        &self,
        chain: Chain,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        Ok(self.fetch_history_page(chain, &address, None).await?.items)
    }
    pub(crate) async fn fetch_normalized_history_page(
        &self,
        chain: Chain,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<
        crate::api::HistoryPage<crate::fetch::history_decode::NormalizedHistoryItem>,
        SpectraBridgeError,
    > {
        let page = self.fetch_history_page(chain, address, cursor).await?;
        let items = crate::fetch::history::normalize_chain_history(chain, &page.items)
            .into_iter()
            .map(|e| crate::fetch::history_decode::NormalizedHistoryItem {
                deployment_id: e.deployment_id,
                kind: e.kind,
                status: e.status,
                asset_display_name: e.asset_display_name,
                symbol: e.symbol,
                chain_id: e.chain_id,
                amount: e.amount,
                counterparty: e.counterparty,
                tx_hash: e.tx_hash,
                block_height: e.block_height,
                timestamp: e.timestamp,
            })
            .collect();
        Ok(crate::api::HistoryPage {
            items,
            next_cursor: page.next_cursor,
        })
    }
    async fn fetch_history_page(
        &self,
        chain: Chain,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<RawHistoryPage, SpectraBridgeError> {
        if chain.mainnet_counterpart() == Chain::Monero {
            return Ok(RawHistoryPage {
                items: self.monero_history(chain, address).await?,
                next_cursor: None,
            });
        }
        fetch_history_page(address, chain, cursor, self).await
    }
    /// Fetch one page of EVM transaction history for `address`.
    ///
    /// Runs two requests in parallel against the configured Etherscan-compatible
    /// explorer endpoint:
    ///   1. `txlist` — native ETH/EVM transfers
    ///   2. `tokentx` — ERC-20 token transfers
    ///
    /// `tokens` lists the known tokens to include. Only transfers whose
    /// contract matches a known token are returned; pass an empty list to
    /// skip token transfers entirely.
    pub(crate) async fn fetch_evm_history_page(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
        tokens: Vec<TokenDescriptor>,
        page: u32,
        page_size: u32,
    ) -> Result<EvmHistoryPage, SpectraBridgeError> {
        use crate::fetch::history_decode::{
            EvmHistoryPageDecoded, EvmNativeTransferItem, EvmTokenTransferItem,
        };

        // Only EVM chains are supported.
        let chain = evm_network(chain_id)?;
        for token in &tokens {
            let standard = if token.standard.is_empty() {
                chain.token_standard_for_identifier(&token.contract)
            } else {
                &token.standard
            };
            crate::tokens::validate_protocol_identifier(chain, standard, &token.contract)?;
        }

        // History is served by indexers, independently for native and token transfers.
        let client = crate::api::blockscout::BlockscoutClient::new();
        let sources = self
            .api_endpoints(
                chain,
                crate::EndpointApi::Blockscout,
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
        let raw_tokens = if tokens.is_empty() {
            vec![]
        } else {
            let sources = self
                .api_endpoints(
                    chain,
                    crate::EndpointApi::Blockscout,
                    &[EndpointCapability::TokenHistory],
                )
                .await?;
            crate::api::http::race(&sources, |base| {
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

        let page_size = page_size.clamp(1, 500) as usize;
        let exhausted = native_entries.len() < page_size && raw_tokens.len() < page_size;

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
            .filter_map(|mut entry| {
                let key = entry.contract.to_lowercase();
                let (sym, name, dec, standard) = token_map.get(&key)?.clone();
                entry.symbol = sym;
                entry.token_name = name;
                if dec != entry.decimals {
                    entry.decimals = dec;
                    entry.amount_display =
                        crate::decimal::from_unit_digits(&entry.amount_raw, u32::from(dec))?;
                }
                if entry.from != addr_lower && entry.to != addr_lower {
                    return None;
                }
                Some(EvmTokenTransferItem {
                    standard,
                    contract_address: entry.contract,
                    token_name: entry.token_name,
                    symbol: entry.symbol,
                    decimals: entry.decimals as i32,
                    from_address: entry.from,
                    to_address: entry.to,
                    amount_decimal: entry.amount_display,
                    transaction_hash: entry.txid,
                    block_number: entry.block_number as i64,
                    log_index: entry.log_index as i64,
                    timestamp: entry.timestamp as f64,
                })
            })
            .collect();

        let native_decoded = native_entries
            .into_iter()
            .map(|e| {
                // A value that is not an integer is not a transfer of nothing.
                let amount_decimal = crate::decimal::from_unit_digits(&e.value_wei, 18)
                    .ok_or_else(|| {
                        SpectraBridgeError::failure(format!("transfer {}: malformed value", e.txid))
                    })?;
                Ok(EvmNativeTransferItem {
                    status: e.status,
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

        Ok(EvmHistoryPage {
            decoded: EvmHistoryPageDecoded {
                tokens: tokens_decoded,
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
fn page_values<T: serde::Serialize>(
    items: Vec<T>,
) -> Result<Vec<serde_json::Value>, SpectraBridgeError> {
    items
        .into_iter()
        .map(|item| serde_json::to_value(item).map_err(SpectraBridgeError::from))
        .collect()
}
struct RawHistoryPage {
    items: String,
    next_cursor: Option<String>,
}
fn serialized_page<T: serde::Serialize>(
    page: crate::api::HistoryPage<T>,
) -> Result<RawHistoryPage, SpectraBridgeError> {
    Ok(RawHistoryPage {
        items: json_response(&page.items)?,
        next_cursor: page.next_cursor,
    })
}
async fn fetch_history_page(
    address: &str,
    chain: Chain,
    cursor: Option<&str>,
    service: &WalletService,
) -> Result<RawHistoryPage, SpectraBridgeError> {
    use crate::EndpointApi as Api;
    let requirements = &[EndpointCapability::History];
    if chain.uses_utxo_client() {
        return serialized_page(
            service
                .utxo_client(chain, requirements)
                .await
                .fetch_history_page(address, cursor)
                .await?,
        );
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
        return serialized_page(crate::api::HistoryPage { items, next_cursor });
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
        return serialized_page(
            crate::api::aptos_indexer::AptosIndexerClient::new(Arc::new(endpoints), expected)
                .fetch_history_page(address, cursor, &AptosClient::new(nodes))
                .await?,
        );
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
        return serialized_page(
            crate::api::trongrid_v1::TrongridClient::with_trc10(
                Arc::new(accounts),
                chain,
                TronHttpClient::new(node_endpoints),
            )
            .fetch_history_page(address, 50, cursor)
            .await?,
        );
    }
    let (api, endpoints) = service.fetch_endpoints(chain, requirements).await?;
    match api {
        Api::SolanaJsonRpc => serialized_page(
            SolanaClient::new(endpoints)
                .fetch_unified_history_page(address, 50, cursor)
                .await?,
        ),
        Api::Horizon => serialized_page(
            HorizonClient::new(endpoints)
                .fetch_history_page(address, cursor)
                .await?,
        ),
        Api::XrplJsonRpc => serialized_page(
            XrplClient::new(endpoints)
                .fetch_history_page(address, cursor)
                .await?,
        ),
        Api::Koios => serialized_page(
            KoiosClient::new(endpoints)
                .fetch_history_page(address, cursor)
                .await?,
        ),
        Api::SubstrateJsonRpc => Err(SpectraBridgeError::failure(format!(
            "{}: no keyless history source configured",
            chain.chain_display_name()
        ))),
        Api::SuiJsonRpc => serialized_page(
            SuiClient::new(endpoints)
                .fetch_history_page(address, cursor)
                .await?,
        ),
        Api::ToncenterV2 => {
            let mut position: AssetCursor = cursor
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default();
            let mut items = Vec::new();
            if !position.native_done {
                let page = ToncenterV2Client::new(endpoints)
                    .fetch_history_page(address, position.native.as_deref())
                    .await?;
                items.extend(page_values(page.items)?);
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
            serialized_page(crate::api::HistoryPage {
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
                    .await?;
                items.extend(page_values(page.items)?);
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
            serialized_page(crate::api::HistoryPage {
                items,
                next_cursor: position.next()?,
            })
        }
        Api::IcpRosetta => serialized_page(
            IcpClient::new(endpoints)
                .fetch_history_page(address, cursor)
                .await?,
        ),
        Api::Insight => serialized_page(
            InsightClient::new(endpoints)
                .fetch_history_page(address, cursor)
                .await?,
        ),
        Api::KaspaRest => serialized_page(
            KaspaClient::new(endpoints)
                .fetch_history_page(address, cursor)
                .await?,
        ),
        c => Err(SpectraBridgeError::failure(format!(
            "unsupported history API: {c:?}"
        ))),
    }
}

impl WalletService {
    pub async fn fetch_history_summary(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
    ) -> Result<crate::diagnostics::HistorySummary, SpectraBridgeError> {
        let raw = self.fetch_history(chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_summary(raw))
    }
}

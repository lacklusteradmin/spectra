//! Network history: service adapters and dispatch.
use super::*;

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
            let raw = this.fetch_history(chain_id, address).await?;
            let entries = crate::fetch::history::normalize_chain_history(chain_id, &raw);
            Ok(entries
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
                .collect())
        })
        .await
    }
}
impl WalletService {
    pub(crate) async fn fetch_history(
        &self,
        chain: crate::registry::Chain,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        if chain.mainnet_counterpart() == Chain::Monero {
            return self.monero_history(chain, &address).await;
        }
        fetch_history(&address, chain, None, self).await
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
    pub async fn fetch_evm_history_page(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
        tokens: Vec<TokenDescriptor>,
        page: u32,
        page_size: u32,
    ) -> Result<crate::fetch::history_decode::EvmHistoryPageDecoded, SpectraBridgeError> {
        use crate::fetch::history_decode::{
            EvmHistoryPageDecoded, EvmNativeTransferItem, EvmTokenTransferItem,
        };

        // Only EVM chains are supported.
        let chain = evm_network(chain_id)?;

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
            return Err("no explorer configured for this chain".into());
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

        // Build a lookup map from contract address (lowercased) → known token metadata.
        let addr_lower = address.to_lowercase();
        let token_map: std::collections::HashMap<String, (String, String, u8)> = tokens
            .iter()
            .map(|t| {
                (
                    t.contract.to_lowercase(),
                    (
                        t.symbol.clone(),
                        t.name.clone().unwrap_or_default(),
                        t.decimals,
                    ),
                )
            })
            .collect();

        let tokens_decoded: Vec<EvmTokenTransferItem> = raw_tokens
            .into_iter()
            .filter_map(|mut entry| {
                let key = entry.contract.to_lowercase();
                let (sym, name, dec) = token_map.get(&key)?.clone();
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
                    .ok_or_else(|| format!("transfer {}: malformed value", e.txid))?;
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

        Ok(EvmHistoryPageDecoded {
            tokens: tokens_decoded,
            native: native_decoded,
        })
    }
}
async fn fetch_history(
    address: &str,
    chain: Chain,
    _token: Option<&str>,
    service: &WalletService,
) -> Result<String, SpectraBridgeError> {
    let requirements: &[EndpointCapability] = if chain.mainnet_counterpart() == Chain::Solana {
        &[
            EndpointCapability::History,
            EndpointCapability::TokenHistory,
        ]
    } else {
        &[EndpointCapability::History]
    };
    if chain.uses_utxo_client() {
        return json_response(
            &service
                .utxo_client(chain, requirements)
                .await
                .fetch_history(address, None)
                .await?,
        );
    }
    let (api, endpoints) = service.fetch_endpoints(chain, requirements).await?;
    use crate::EndpointApi as Api;
    match api {
        Api::EvmJsonRpc => {
            let sources = service
                .api_endpoints(chain, Api::Blockscout, &[EndpointCapability::History])
                .await?;
            let client = crate::api::blockscout::BlockscoutClient::new();
            let h = crate::api::http::race(&sources, |base| {
                let client = &client;
                async move {
                    client
                        .fetch_history(
                            address,
                            crate::registry::EvmHistorySource::Open(&base),
                            1,
                            50,
                        )
                        .await
                }
            })
            .await?;
            json_response(&h)
        }
        Api::SolanaJsonRpc => json_response(
            &SolanaClient::new(endpoints)
                .fetch_unified_history(address, 50)
                .await?,
        ),
        Api::TronHttp => {
            let accounts = service
                .tron_account_endpoints(
                    chain,
                    &endpoints,
                    &[
                        EndpointCapability::History,
                        EndpointCapability::TokenHistory,
                    ],
                )
                .await?;
            json_response(
                &crate::api::trongrid_v1::TrongridClient::new(Arc::new(accounts))
                    .fetch_history(address, 50)
                    .await?,
            )
        }
        Api::Horizon => json_response(&HorizonClient::new(endpoints).fetch_history(address).await?),
        Api::XrplJsonRpc => {
            json_response(&XrplClient::new(endpoints).fetch_history(address).await?)
        }
        Api::Koios => json_response(&KoiosClient::new(endpoints).fetch_history(address).await?),
        Api::SubstrateJsonRpc => Err(format!(
            "{}: no keyless history source configured",
            chain.chain_display_name()
        )
        .into()),
        Api::SuiJsonRpc => json_response(&SuiClient::new(endpoints).fetch_history(address).await?),
        Api::AptosRest => json_response(&AptosClient::new(endpoints).fetch_history(address).await?),
        Api::ToncenterV2 => json_response(
            &ToncenterV2Client::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Api::NearJsonRpc => {
            let indexers = service
                .api_endpoints(chain, Api::Nearblocks, &[EndpointCapability::History])
                .await?;
            if indexers.is_empty() {
                return Err("No NEAR history indexer configured".into());
            }
            json_response(
                &crate::api::nearblocks::NearblocksClient::new(Arc::new(indexers))
                    .fetch_history(address)
                    .await?,
            )
        }
        Api::IcpRosetta => json_response(&IcpClient::new(endpoints).fetch_history(address).await?),

        Api::Insight => json_response(&InsightClient::new(endpoints).fetch_history(address).await?),
        Api::KaspaRest => json_response(&KaspaClient::new(endpoints).fetch_history(address).await?),

        c => Err(SpectraBridgeError::from(format!("unsupported API: {c:?}"))),
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

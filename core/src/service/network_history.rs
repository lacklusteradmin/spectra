//! Network history: service adapters and dispatch.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch history for `address` on `chain_id` and normalize the raw
    /// chain-specific shape into a standard `NormalizedHistoryItem` array,
    /// returning typed records directly across the FFI boundary.
    pub async fn fetch_normalized_history(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<Vec<crate::fetch::history_decode::NormalizedHistoryItem>, SpectraBridgeError> {
        let raw = self.fetch_history(&chain_id, address).await?;
        let entries = crate::fetch::history::normalize_chain_history(&chain_id, &raw);
        Ok(entries
            .into_iter()
            .map(|e| crate::fetch::history_decode::NormalizedHistoryItem {
                deployment_id: e.deployment_id,
                kind: e.kind,
                status: e.status,
                asset_display_name: e.asset_display_name,
                symbol: e.symbol,
                chain_name: e.chain_name,
                amount: e.amount,
                counterparty: e.counterparty,
                tx_hash: e.tx_hash,
                block_height: e.block_height,
                timestamp: e.timestamp,
            })
            .collect())
    }
}
impl WalletService {
    pub(crate) async fn fetch_history(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_id(chain_id)?;
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
        chain_id: String,
        address: String,
        tokens: Vec<TokenDescriptor>,
        page: u32,
        page_size: u32,
    ) -> Result<crate::fetch::history_decode::EvmHistoryPageDecoded, SpectraBridgeError> {
        use crate::fetch::history_decode::{
            EvmHistoryPageDecoded, EvmNativeTransferItem, EvmTokenTransferItem,
        };

        // Only EVM chains are supported.
        let chain = evm_network_for_id(&chain_id)?;

        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id()?);

        let source = chain.evm_history_source();

        // Fetch native and token transfers concurrently.
        let (native_result, token_result) = tokio::join!(
            client.fetch_history(&address, source, page, page_size),
            async {
                if tokens.is_empty() {
                    Ok(Vec::new())
                } else {
                    client
                        .fetch_token_transfers(&address, source, page, page_size)
                        .await
                }
            }
        );

        let native_entries = native_result?;
        let raw_tokens = token_result?;

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
                        crate::fetch::evm::format_evm_decimals(&entry.amount_raw, dec);
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

        let native_decoded: Vec<EvmNativeTransferItem> = native_entries
            .into_iter()
            .map(|e| EvmNativeTransferItem {
                status: e.status,
                from_address: e.from,
                to_address: e.to,
                amount_decimal: crate::fetch::history_decode::decimal_string_from_wei(&e.value_wei),
                transaction_hash: e.txid,
                block_number: e.block_number as i64,
                timestamp: e.timestamp as f64,
            })
            .collect();

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
    let (api, endpoints) = service.fetch_endpoints(chain).await?;
    use crate::EndpointApi as Api;
    match api {
        Api::Esplora => json_response(
            &BitcoinClient::new(HttpClient::shared(), endpoints)
                .fetch_history(address, None)
                .await?,
        ),
        Api::Blockbook => json_response(
            &BlockbookClient::new(endpoints, chain)
                .fetch_history(address)
                .await?,
        ),
        Api::Whatsonchain => json_response(
            &BitcoinSvClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),

        Api::Blockcypher => json_response(
            &DogecoinClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Api::EvmJsonRpc => {
            let source = chain.evm_history_source();

            let h = EvmClient::new(endpoints, chain.evm_chain_id()?)
                .fetch_history(address, source, 1, 50)
                .await?;
            json_response(&h)
        }
        Api::SolanaJsonRpc => json_response(
            &SolanaClient::new(endpoints)
                .fetch_unified_history(address, 50)
                .await?,
        ),
        Api::TronHttp => {
            let tronscan = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Explorer))
                .await
                .first()
                .cloned()
                .unwrap_or_else(|| "https://apilist.tronscan.org".to_string());
            json_response(
                &TronClient::new(endpoints)
                    .fetch_unified_history(address, &tronscan, 50)
                    .await?,
            )
        }
        Api::Horizon => json_response(&StellarClient::new(endpoints).fetch_history(address).await?),
        Api::XrplJsonRpc => json_response(&XrpClient::new(endpoints).fetch_history(address).await?),
        Api::Koios => json_response(&CardanoClient::new(endpoints).fetch_history(address).await?),
        Api::SubstrateJsonRpc if chain.mainnet_counterpart() == Chain::Bittensor => json_response(
            &BittensorClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Api::SubstrateJsonRpc => json_response(
            &PolkadotClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Api::SuiJsonRpc => json_response(&SuiClient::new(endpoints).fetch_history(address).await?),
        Api::AptosRest => json_response(&AptosClient::new(endpoints).fetch_history(address).await?),
        Api::ToncenterV2 => json_response(&TonClient::new(endpoints).fetch_history(address).await?),
        Api::NearJsonRpc => {
            let indexer = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Explorer))
                .await
                .first()
                .cloned()
                .unwrap_or_else(|| "https://api.kitwallet.app".to_string());
            json_response(
                &NearClient::new(endpoints)
                    .fetch_history(address, &indexer)
                    .await?,
            )
        }
        Api::IcpRosetta => json_response(&IcpClient::new(endpoints).fetch_history(address).await?),
        Api::MoneroWalletRpc => {
            json_response(&MoneroClient::new(endpoints).fetch_history(0).await?)
        }

        Api::Insight => json_response(&DecredClient::new(endpoints).fetch_history(address).await?),
        Api::KaspaRest => json_response(&KaspaClient::new(endpoints).fetch_history(address).await?),

        c => Err(SpectraBridgeError::from(format!("unsupported API: {c:?}"))),
    }
}

impl WalletService {
    pub async fn fetch_history_summary(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<crate::diagnostics::HistorySummary, SpectraBridgeError> {
        let raw = self.fetch_history(&chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_summary(raw))
    }
}

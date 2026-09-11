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
        let entries = crate::history::normalize_chain_history(&chain_id, &raw);
        Ok(entries
            .into_iter()
            .map(|e| crate::fetch::history_decode::NormalizedHistoryItem {
                kind: e.kind,
                status: e.status,
                asset_name: e.asset_name,
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

    /// One history fetch, and everything the callers ask about it.
    ///
    /// Three methods did this — `fetch_history_has_activity`,
    /// `fetch_history_entry_count` and `fetch_history_confirmed_txids` — each
    /// running the same `fetch_history` and applying one projection, and the
    /// first was `entry_count > 0`.
    pub async fn fetch_history_summary(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<crate::diagnostics::HistorySummary, SpectraBridgeError> {
        let raw = self.fetch_history(&chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_summary(raw))
    }

    /// Fetch EVM history for diagnostics and return a fully-built row. On
    /// network or chain-support failure the row is seeded with the error.
    pub async fn fetch_evm_history_diagnostics(
        &self,
        chain_id: String,
        wallet_id: String,
        address: String,
    ) -> crate::diagnostics::HistoryDiagnostics {
        use crate::diagnostics::aggregate::{
            diagnostics_make_evm_error, diagnostics_make_evm_success_record,
        };
        match self
            .fetch_evm_history_page(chain_id, address.clone(), Vec::new(), 1, 50)
            .await
        {
            Ok(page) => diagnostics_make_evm_success_record(wallet_id, address, &page),
            Err(err) => diagnostics_make_evm_error(wallet_id, address, err.to_string()),
        }
    }
}
impl WalletService {
    pub(crate) async fn fetch_history(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_id(chain_id)?;
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
        let chain = chain_for_evm_id(&chain_id)?;

        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());

        let source = chain.evm_history_source();
        let etherscan_chain_id = chain.evm_chain_id();
        let api_key_owned: String = self
            .etherscan_api_key
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        let api_key_str = if api_key_owned.is_empty() {
            None
        } else {
            Some(api_key_owned.as_str())
        };

        // Fetch native and token transfers concurrently.
        let (native_result, token_result) = tokio::join!(
            client.fetch_history(
                &address,
                source,
                api_key_str,
                etherscan_chain_id,
                page,
                page_size
            ),
            async {
                if tokens.is_empty() {
                    Ok(Vec::new())
                } else {
                    client
                        .fetch_token_transfers(
                            &address,
                            source,
                            api_key_str,
                            etherscan_chain_id,
                            page,
                            page_size,
                        )
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
                        crate::fetch::chains::evm::format_evm_decimals(&entry.amount_raw, dec);
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
    let endpoints = service.endpoints_for(chain.str_id()).await;
    let dispatch = chain.mainnet_counterpart();
    match dispatch {
        Chain::Bitcoin => json_response(
            &BitcoinClient::new(HttpClient::shared(), endpoints)
                .fetch_history(address, None)
                .await?,
        ),
        Chain::BitcoinCash => json_response(
            &BitcoinCashClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Chain::BitcoinSV => json_response(
            &BitcoinSvClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Chain::Litecoin => json_response(
            &LitecoinClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Chain::Dogecoin => json_response(
            &DogecoinClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        c if c.is_evm() => {
            let source = chain.evm_history_source();
            let api_key_owned = service
                .etherscan_api_key
                .read()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_default();
            let api_key_str = if api_key_owned.is_empty() {
                None
            } else {
                Some(api_key_owned.as_str())
            };
            let h = EvmClient::new(endpoints, chain.evm_chain_id())
                .fetch_history(address, source, api_key_str, chain.evm_chain_id(), 1, 50)
                .await?;
            json_response(&h)
        }
        Chain::Solana => json_response(
            &SolanaClient::new(endpoints)
                .fetch_unified_history(address, 50)
                .await?,
        ),
        Chain::Tron => {
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
        Chain::Stellar => {
            json_response(&StellarClient::new(endpoints).fetch_history(address).await?)
        }
        Chain::Xrp => json_response(&XrpClient::new(endpoints).fetch_history(address).await?),
        Chain::Cardano => {
            let api_key = service
                .api_key_for(chain.str_id())
                .await
                .unwrap_or_default();
            json_response(
                &CardanoClient::new(endpoints, api_key)
                    .fetch_history(address)
                    .await?,
            )
        }
        Chain::Polkadot => {
            let subscan = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                .await;
            let api_key = service.api_key_for(chain.str_id()).await;
            json_response(
                &PolkadotClient::new(endpoints, subscan, api_key)
                    .fetch_history(address)
                    .await?,
            )
        }
        Chain::Sui => json_response(&SuiClient::new(endpoints).fetch_history(address).await?),
        Chain::Aptos => json_response(&AptosClient::new(endpoints).fetch_history(address).await?),
        Chain::Ton => {
            let api_key = service.api_key_for(chain.str_id()).await;
            json_response(
                &TonClient::new(endpoints, api_key)
                    .fetch_history(address)
                    .await?,
            )
        }
        Chain::Near => {
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
        Chain::Icp => json_response(&IcpClient::new(endpoints).fetch_history(address).await?),
        Chain::Monero => json_response(&MoneroClient::new(endpoints).fetch_history(0).await?),
        Chain::Zcash => json_response(&ZcashClient::new(endpoints).fetch_history(address).await?),
        Chain::BitcoinGold => json_response(
            &BitcoinGoldClient::new(endpoints)
                .fetch_history(address)
                .await?,
        ),
        Chain::Decred => json_response(&DecredClient::new(endpoints).fetch_history(address).await?),
        Chain::Kaspa => json_response(&KaspaClient::new(endpoints).fetch_history(address).await?),
        Chain::Dash => json_response(&DashClient::new(endpoints).fetch_history(address).await?),
        Chain::Bittensor => {
            let taostats = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                .await;
            let api_key = service.api_key_for(chain.str_id()).await;
            json_response(
                &BittensorClient::new(endpoints, taostats, api_key)
                    .fetch_history(address)
                    .await?,
            )
        }
        c => Err(SpectraBridgeError::from(format!(
            "unsupported chain: {c:?}"
        ))),
    }
}

//! Every read that leaves the process: balances, history, token holdings,
//! EVM probes, prices and fiat rates, plus the per-chain dispatch the three
//! free functions at the bottom of this file provide.
//!
//! An arm here builds its client from the endpoint list and runs the fetch;
//! adding a chain means one new arm per dispatcher, not a new enum case.

use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn fetch_erc20_balance_typed(
        &self,
        chain_id: String,
        contract: String,
        holder: String,
    ) -> Result<crate::fetch::chains::evm::Erc20Balance, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let endpoints = self.endpoints_for(chain.str_id()).await;
        let client = crate::fetch::chains::evm::EvmClient::new(endpoints, chain.evm_chain_id());
        client
            .fetch_erc20_balance(&contract, &holder)
            .await
            .map_err(Into::into)
    }

    /// Unified per-chain native balance summary, replacing chain-specific JSON
    /// decoding on the Swift side. Smallest unit is returned as a decimal
    /// string (sats / wei / lamports / yocto-NEAR / ...) so callers can `UInt64`
    /// or `BigInt` parse as appropriate. `amount_display` is the human-readable
    /// native amount as decimal string. `utxo_count` is 0 for non-UTXO chains.
    pub async fn fetch_native_balance_summary(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        fetch_native_balance_summary(&address, chain, self).await
    }

    // `fetch_history` lives in the plain-impl block below (JSON shuttle —
    // kept internal, not exported to Swift).

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

    /// Fetch history JSON for `address` on `chain_id` and return
    /// `true` when the response is a non-empty JSON array. Lets Swift
    /// avoid parsing the chain-specific history shape just to answer
    /// "has this address seen any activity?".
    pub async fn fetch_history_has_activity(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<bool, SpectraBridgeError> {
        let raw = self.fetch_history(&chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_entry_count(raw) > 0)
    }

    /// Fetch history JSON and return the top-level entry count.
    pub async fn fetch_history_entry_count(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<u32, SpectraBridgeError> {
        let raw = self.fetch_history(&chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_entry_count(raw))
    }

    /// Fetch history JSON and return the set of confirmed `txid`s.
    /// Used to reconcile pending transactions with on-chain confirmations.
    pub async fn fetch_history_confirmed_txids(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let raw = self.fetch_history(&chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_confirmed_txids(raw))
    }

    /// Fused Bitcoin HD history page: derive external+change addresses from
    /// `xpub`, concurrently fetch each address's history, and merge into a
    /// deduplicated page truncated to `limit`. Scan window is 20 external +
    /// 10 change.
    pub async fn fetch_bitcoin_hd_history_page(
        &self,
        xpub: String,
        limit: u64,
    ) -> Result<Vec<crate::history::CoreBitcoinHistorySnapshot>, SpectraBridgeError> {
        use futures::stream::{self, StreamExt};
        const RECEIVE_COUNT: u32 = 20;
        const CHANGE_COUNT: u32 = 10;

        let mut addresses = self
            .derive_bitcoin_hd_address_strings(xpub.clone(), 0, 0, RECEIVE_COUNT)
            .await?;
        addresses.extend(
            self.derive_bitcoin_hd_address_strings(xpub, 1, 0, CHANGE_COUNT)
                .await?,
        );

        let fetched: Vec<Vec<crate::history::CoreBitcoinHistorySnapshot>> =
            stream::iter(addresses.clone())
                .map(|address| self.fetch_bitcoin_history_snapshots(address))
                .buffered(4)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

        Ok(crate::history::merge_bitcoin_history_snapshots(
            crate::history::MergeBitcoinHistorySnapshotsRequest {
                snapshots: fetched.into_iter().flatten().collect(),
                owned_addresses: addresses,
                limit,
            },
        ))
    }

    // sign_and_send / sign_and_send_token live in the plain `impl WalletService`
    // block below. UniFFI exports every method of a `#[uniffi::export]` impl
    // block regardless of `pub(crate)` visibility, so chain-dispatch helpers
    // consumed only by execute_send must be outside this block.

    // ── Token balance (ERC-20 / SPL / NEP-141 / TRC-20 / Stellar assets)

    /// Fetch balances for a list of tokens in one call.
    ///
    /// For Solana `contract` is the mint address; for Sui / Aptos it is the
    /// coin type; for TON it is the jetton master address.
    ///
    /// Tokens that fail to fetch are returned with `balance_raw = "0"` so the
    /// caller always gets back the full list.
    pub async fn fetch_token_balances(
        &self,
        chain_id: String,
        address: String,
        tokens: Vec<TokenDescriptor>,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let chain = Chain::from_str_id(&chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_token_balances: unsupported chain_id: {chain_id}"
            ))
        })?;
        let endpoints = self.endpoints_for(chain.str_id()).await;

        macro_rules! coin_token_balances {
            ($Client:ty, $endpoints:expr) => {{
                use futures::future::join_all;
                let client = std::sync::Arc::new(<$Client>::new($endpoints));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let address = address.clone();
                        let coin_type = t.contract.clone();
                        let symbol = t.symbol.clone();
                        let decimals = t.decimals;
                        async move {
                            let raw = client
                                .fetch_coin_balance(&address, &coin_type)
                                .await
                                .unwrap_or(0u64);
                            TokenBalanceResult {
                                contract_address: coin_type,
                                symbol,
                                decimals,
                                balance_raw: raw.to_string(),
                                balance_display: format_decimals(raw as u128, decimals),
                            }
                        }
                    })
                    .collect();
                join_all(futs).await
            }};
        }

        let results: Vec<TokenBalanceResult> = match chain {
            Chain::Tron => {
                use futures::future::join_all;
                let client = std::sync::Arc::new(TronClient::new(endpoints));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let contract = t.contract.clone();
                        let holder = address.clone();
                        let symbol = t.symbol.clone();
                        let decimals = t.decimals;
                        async move {
                            match client.fetch_trc20_balance(&contract, &holder).await {
                                Ok(b) => TokenBalanceResult {
                                    contract_address: contract,
                                    symbol,
                                    decimals,
                                    balance_raw: b.balance_raw,
                                    balance_display: b.balance_display,
                                },
                                Err(_) => TokenBalanceResult {
                                    contract_address: contract,
                                    symbol,
                                    decimals,
                                    balance_raw: "0".to_string(),
                                    balance_display: "0".to_string(),
                                },
                            }
                        }
                    })
                    .collect();
                join_all(futs).await
            }
            Chain::Solana => {
                let client = SolanaClient::new(endpoints);
                let mints: Vec<String> = tokens.iter().map(|t| t.contract.clone()).collect();
                let spl = client
                    .fetch_spl_balances(&address, &mints)
                    .await
                    .unwrap_or_default();
                let by_mint: std::collections::HashMap<
                    &str,
                    &crate::fetch::chains::solana::SplBalance,
                > = spl.iter().map(|b| (b.mint.as_str(), b)).collect();
                tokens
                    .iter()
                    .map(|t| {
                        let b = by_mint.get(t.contract.as_str());
                        TokenBalanceResult {
                            contract_address: t.contract.clone(),
                            symbol: t.symbol.clone(),
                            decimals: t.decimals,
                            balance_raw: b
                                .map(|b| b.balance_raw.clone())
                                .unwrap_or_else(|| "0".to_string()),
                            balance_display: b
                                .map(|b| b.balance_display.clone())
                                .unwrap_or_else(|| "0".to_string()),
                        }
                    })
                    .collect()
            }
            Chain::Near => {
                use futures::future::join_all;
                let client = std::sync::Arc::new(NearClient::new(endpoints));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let contract = t.contract.clone();
                        let holder = address.clone();
                        let symbol = t.symbol.clone();
                        let decimals = t.decimals;
                        async move {
                            let raw = client
                                .fetch_ft_balance_of(&contract, &holder)
                                .await
                                .unwrap_or(0u128);
                            let display = format_decimals(raw, decimals);
                            TokenBalanceResult {
                                contract_address: contract,
                                symbol,
                                decimals,
                                balance_raw: raw.to_string(),
                                balance_display: display,
                            }
                        }
                    })
                    .collect();
                join_all(futs).await
            }
            Chain::Sui => coin_token_balances!(SuiClient, endpoints),
            Chain::Aptos => coin_token_balances!(AptosClient, endpoints),
            Chain::Ton => {
                // TON — jetton balances via TonCenter v3 API. The v3 endpoint
                // lives in the chain's Secondary slot (registered as id + 100 = 116).
                let v3_endpoints = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = TonClient::new(endpoints, api_key).with_v3_endpoints(v3_endpoints);
                let jetton_balances = client
                    .fetch_jetton_balances(&address)
                    .await
                    .unwrap_or_default();

                tokens
                    .iter()
                    .map(|t| {
                        let raw = jetton_balances
                            .iter()
                            .find(|j| j.master_address.eq_ignore_ascii_case(&t.contract))
                            .map(|j| j.balance_raw)
                            .unwrap_or(0u128);
                        let display = format_decimals(raw, t.decimals);
                        TokenBalanceResult {
                            contract_address: t.contract.clone(),
                            symbol: t.symbol.clone(),
                            decimals: t.decimals,
                            balance_raw: raw.to_string(),
                            balance_display: display,
                        }
                    })
                    .collect()
            }
            // The EVM family. This was `fetch_evm_token_balances_batch_typed`,
            // a second method with the *same* signature and the complementary
            // set of chains — so a caller holding a chain had to know which
            // family it was in to pick the right one, which is exactly what
            // the chain id already says.
            c if c.is_evm() => {
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let mut results = Vec::with_capacity(tokens.len());
                for token in &tokens {
                    let contract = token.contract.to_lowercase();
                    if contract.is_empty() {
                        continue;
                    }
                    let raw = client
                        .fetch_erc20_balance_of(&contract, &address)
                        .await
                        .unwrap_or(0);
                    results.push(TokenBalanceResult {
                        contract_address: contract,
                        symbol: token.symbol.clone(),
                        decimals: token.decimals,
                        balance_raw: raw.to_string(),
                        balance_display: format_decimals(raw, token.decimals),
                    });
                }
                results
            }
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "fetch_token_balances: unsupported chain: {c:?}"
                )))
            }
        };

        Ok(results)
    }

    // ── Unified execute_send — collapses derive → payload → sign trampoline

    // `execute_send` lives in `service/send_execution.rs`.

    // ── Bitcoin HD — seed → account xpub derivation

    // ── Bitcoin HD multi-address (xpub / ypub / zpub)

    // `fetch_bitcoin_xpub_balance` lives in the plain-impl block below
    // (JSON shuttle — kept internal, not exported to Swift).

    // ── Price / fiat rate service

    // ── EVM paginated history (native + ERC-20 token transfers)

    /// Fetch one page of EVM transaction history for `address`.
    ///
    /// Runs two requests in parallel against the configured Etherscan-compatible
    /// explorer endpoint:
    ///   1. `txlist` — native ETH/EVM transfers
    ///   2. `tokentx` — ERC-20 token transfers
    ///
    /// `tokens` lists the tracked tokens to include. Only transfers whose
    /// contract matches a tracked token are returned; pass an empty list to
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

        let explorer_base = chain.evm_explorer_api_base().ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "{} does not support Etherscan history",
                chain.chain_display_name()
            ))
        })?;
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
            client.fetch_history(&address, explorer_base, api_key_str, etherscan_chain_id),
            client.fetch_token_transfers(
                &address,
                explorer_base,
                api_key_str,
                etherscan_chain_id,
                page,
                page_size,
            )
        );

        let native_entries = native_result.unwrap_or_default();
        let raw_tokens = token_result.unwrap_or_default();

        // Build a lookup map from contract address (lowercased) → tracked token metadata.
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

    // ── Typed token-array wrappers (no JSON serialization on caller side)

    /// Fetch EVM history for diagnostics and return a fully-built
    /// `EthereumTokenTransferHistoryDiagnostics` record. On network or
    /// chain-support failure the record is seeded with an error description.
    pub async fn fetch_evm_history_diagnostics(
        &self,
        chain_id: String,
        address: String,
    ) -> crate::diagnostics::EthereumTokenTransferHistoryDiagnostics {
        use crate::diagnostics::aggregate::{
            diagnostics_make_evm_error, diagnostics_make_evm_success_record,
        };
        match self
            .fetch_evm_history_page(chain_id, address.clone(), Vec::new(), 1, 50)
            .await
        {
            Ok(page) => diagnostics_make_evm_success_record(address, &page),
            Err(err) => diagnostics_make_evm_error(address, err.to_string()),
        }
    }

    // ── ENS resolution

    /// Resolve an ENS name to an Ethereum address via the ENS Ideas public API.
    /// Returns the resolved address, or `None` if the name has no registered address.
    pub async fn resolve_ens_name_typed(
        &self,
        name: String,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let eps = self.endpoints_for("ethereum").await;
        let client = EvmClient::new(eps, 1);
        let address = client.resolve_ens(&name).await?;
        Ok(address.filter(|a| !a.is_empty()))
    }

    // ── EVM utilities (contract detection, nonce lookup)

    /// Returns true iff `address` has deployed bytecode on the given EVM chain.
    pub async fn fetch_evm_has_contract_code(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<bool, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let code = client.fetch_code(&address).await?;
        Ok(crate::send::flow::core_evm_has_contract_code(code))
    }

    /// Fetch the nonce of a submitted transaction by hash on an EVM chain.
    /// Used to pre-fill the replacement-tx nonce field.
    pub async fn fetch_evm_tx_nonce_typed(
        &self,
        chain_id: String,
        tx_hash: String,
    ) -> Result<u64, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        client.fetch_tx_nonce(&tx_hash).await.map_err(Into::into)
    }

    // `fetch_utxo_fee_preview` and `broadcast_raw` live in the plain-impl
    // block below (JSON shuttles — kept internal, not exported to Swift).

    // ── EVM receipt polling

    /// Fused fetch + classification for an EVM receipt: returns
    /// `Some(classification)` once the receipt has been mined, or `None`
    /// while the transaction is still pending.
    pub async fn fetch_evm_receipt_classification(
        &self,
        chain_id: String,
        tx_hash: String,
    ) -> Result<Option<crate::send::flow::EvmReceiptClassification>, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let Some(receipt) = client.fetch_receipt(&tx_hash).await? else {
            return Ok(None);
        };
        let json = serde_json::to_string(&receipt)?;
        Ok(crate::send::flow::classify_evm_receipt_json(json))
    }

    // `fetch_evm_send_preview` / `fetch_tron_send_preview` /
    // `fetch_simple_chain_send_preview` live in the plain-impl block below
    // (JSON shuttles — kept internal, not exported to Swift). Their typed
    // wrappers below call into those internal helpers.

    // ── Typed send-preview wrappers (fuse fetch + decode in Rust)

    /// Lightweight EVM address probe used for send-flow chain-risk warnings.
    /// Fetches nonce + native balance concurrently and returns both typed,
    /// skipping the fee/gas work of the full preview.
    pub async fn fetch_evm_address_probe(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<EvmAddressProbe, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let (nonce_res, bal_res) =
            tokio::join!(client.fetch_nonce(&address), client.fetch_balance(&address));
        let nonce = nonce_res.unwrap_or(0) as i64;
        let balance_wei: u128 = bal_res
            .map(|b| b.balance_wei.parse::<u128>().unwrap_or(0))
            .unwrap_or(0);
        Ok(EvmAddressProbe {
            nonce,
            balance_eth: balance_wei as f64 / 1e18,
        })
    }

    // ── UTXO tx status

    /// Fetch confirmation status for a UTXO chain transaction.
    /// Returns a typed record so Swift can read `confirmed`/`block_height`/
    /// `confirmations` fields without bouncing through JSON.
    /// Supported chain_ids: 0 (BTC), 3 (DOGE), 5 (LTC), 6 (BCH), 22 (BSV).
    pub async fn fetch_utxo_tx_status_typed(
        &self,
        chain_id: String,
        txid: String,
    ) -> Result<UtxoTxStatus, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_utxo_tx_status: unsupported chain_id: {chain_id}"
            ))
        })?;
        let endpoints = self.endpoints_for(chain.str_id()).await;
        let status: UtxoTxStatus = match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Zcash => {
                let client = ZcashClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::BitcoinGold => {
                let client = BitcoinGoldClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Decred => {
                let client = DecredClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Kaspa => {
                let client = KaspaClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Dash => {
                let client = DashClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "fetch_utxo_tx_status: unsupported chain: {c:?}"
                )))
            }
        };
        Ok(status)
    }
}

impl WalletService {
    /// Fetch Bitcoin history JSON for `address` and decode it into typed
    /// `CoreBitcoinHistorySnapshot` records. Now internal-only — callers go
    /// through `fetch_bitcoin_hd_history_page` for the full HD scan or
    /// `fetch_normalized_history` for single-address paths.
    pub(crate) async fn fetch_bitcoin_history_snapshots(
        &self,
        address: String,
    ) -> Result<Vec<crate::history::CoreBitcoinHistorySnapshot>, SpectraBridgeError> {
        let raw = self.fetch_history(Chain::Bitcoin.str_id(), address).await?;
        Ok(crate::fetch::history_decode::history_decode_bitcoin_raw_snapshots(raw))
    }

    // Internal JSON-returning helpers (not exported to Swift — the typed
    // wrappers above in the exported impl block call these and translate
    // the JSON into UniFFI records at the boundary).

    pub(crate) async fn fetch_balance(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_id(chain_id)?;
        fetch_balance(&address, chain, None, self).await
    }

    /// Typed end-to-end balance fetch used by the refresh engine. Returns a
    /// parsed `NativeBalanceSummary` directly — no JSON-string intermediate.
    ///
    /// For `chain_id == 0` extended-public-key cases we still go through the
    /// xpub balance JSON path — that one's deeply UTXO-aware and not worth
    /// retyping for the marginal saving.
    pub(crate) async fn fetch_native_balance_summary_auto(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        if chain_id == "bitcoin" && is_extended_public_key(&address) {
            // xpub path stays JSON-based; parse once and project into the
            // unified summary shape.
            let json = self.fetch_bitcoin_xpub_balance(address, 20, 20).await?;
            let value: serde_json::Value = serde_json::from_str(&json)?;
            let confirmed_sats = value["confirmed_sats"].as_u64().unwrap_or(0);
            let utxo_count = value["utxo_count"].as_u64().unwrap_or(0) as u32;
            return Ok(NativeBalanceSummary {
                smallest_unit: confirmed_sats.to_string(),
                amount_display: format_smallest_unit_decimal(confirmed_sats as u128, 8),
                utxo_count,
            });
        }
        let chain = chain_for_id(chain_id)?;
        fetch_native_balance_summary(&address, chain, self).await
    }

    pub(crate) async fn fetch_history(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_id(chain_id)?;
        fetch_history(&address, chain, None, self).await
    }

    pub(crate) async fn fetch_bitcoin_xpub_balance(
        &self,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for("bitcoin").await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        let bal = crate::derivation::xpub_walker::fetch_xpub_balance(
            &client,
            &xpub,
            receive_count,
            change_count,
        )
        .await?;
        Ok(serde_json::to_string(&bal)?)
    }
}

// ── Fetch dispatch ────────────────────────────────────────────────────────
// Three free functions replace the old ChainClient enum. Each builds the
// right client inline and runs the fetch — no enum intermediary.
// Adding a chain means one new arm per function.

async fn fetch_balance(
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
                .fetch_balance(address)
                .await?,
        ),
        Chain::BitcoinCash => json_response(
            &BitcoinCashClient::new(endpoints)
                .fetch_balance(address)
                .await?,
        ),
        Chain::BitcoinSV => json_response(
            &BitcoinSvClient::new(endpoints)
                .fetch_balance(address)
                .await?,
        ),
        Chain::Litecoin => json_response(
            &LitecoinClient::new(endpoints)
                .fetch_balance(address)
                .await?,
        ),
        Chain::Dogecoin => json_response(
            &DogecoinClient::new(endpoints)
                .fetch_balance(address)
                .await?,
        ),
        c if c.is_evm() => json_response(
            &EvmClient::new(endpoints, chain.evm_chain_id())
                .fetch_balance(address)
                .await?,
        ),
        Chain::Solana => json_response(&SolanaClient::new(endpoints).fetch_balance(address).await?),
        Chain::Tron => json_response(&TronClient::new(endpoints).fetch_balance(address).await?),
        Chain::Stellar => {
            json_response(&StellarClient::new(endpoints).fetch_balance(address).await?)
        }
        Chain::Xrp => json_response(&XrpClient::new(endpoints).fetch_balance(address).await?),
        Chain::Cardano => {
            let api_key = service
                .api_key_for(chain.str_id())
                .await
                .unwrap_or_default();
            json_response(
                &CardanoClient::new(endpoints, api_key)
                    .fetch_balance(address)
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
                    .fetch_balance(address)
                    .await?,
            )
        }
        Chain::Sui => json_response(&SuiClient::new(endpoints).fetch_balance(address).await?),
        Chain::Aptos => json_response(&AptosClient::new(endpoints).fetch_balance(address).await?),
        Chain::Ton => {
            let api_key = service.api_key_for(chain.str_id()).await;
            json_response(
                &TonClient::new(endpoints, api_key)
                    .fetch_balance(address)
                    .await?,
            )
        }
        Chain::Near => json_response(&NearClient::new(endpoints).fetch_balance(address).await?),
        Chain::Icp => json_response(&IcpClient::new(endpoints).fetch_balance(address).await?),
        Chain::Monero => json_response(&MoneroClient::new(endpoints).fetch_balance(0).await?),
        Chain::Zcash => json_response(&ZcashClient::new(endpoints).fetch_balance(address).await?),
        Chain::BitcoinGold => json_response(
            &BitcoinGoldClient::new(endpoints)
                .fetch_balance(address)
                .await?,
        ),
        Chain::Decred => json_response(&DecredClient::new(endpoints).fetch_balance(address).await?),
        Chain::Kaspa => json_response(&KaspaClient::new(endpoints).fetch_balance(address).await?),
        Chain::Dash => json_response(&DashClient::new(endpoints).fetch_balance(address).await?),
        Chain::Bittensor => {
            let taostats = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                .await;
            let api_key = service.api_key_for(chain.str_id()).await;
            json_response(
                &BittensorClient::new(endpoints, taostats, api_key)
                    .fetch_balance(address)
                    .await?,
            )
        }
        c => Err(SpectraBridgeError::from(format!(
            "unsupported chain: {c:?}"
        ))),
    }
}

async fn fetch_native_balance_summary(
    address: &str,
    chain: Chain,
    service: &WalletService,
) -> Result<NativeBalanceSummary, SpectraBridgeError> {
    let endpoints = service.endpoints_for(chain.str_id()).await;
    let dispatch = chain.mainnet_counterpart();
    match dispatch {
        Chain::Bitcoin => {
            let bal = BitcoinClient::new(HttpClient::shared(), endpoints)
                .fetch_balance(address)
                .await?;
            Ok(NativeBalanceSummary {
                smallest_unit: bal.confirmed_sats.to_string(),
                amount_display: format_smallest_unit_decimal(bal.confirmed_sats as u128, 8),
                utxo_count: bal.utxo_count as u32,
            })
        }
        Chain::BitcoinCash => {
            let bal = BitcoinCashClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::BitcoinSV => {
            let bal = BitcoinSvClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Litecoin => {
            let bal = LitecoinClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Dogecoin => {
            let bal = DogecoinClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_koin.to_string(),
                bal.balance_display,
            ))
        }
        c if c.is_evm() => {
            let bal = EvmClient::new(endpoints, chain.evm_chain_id())
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.balance_wei, bal.balance_display))
        }
        Chain::Solana => {
            let bal = SolanaClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.lamports.to_string(), bal.sol_display))
        }
        Chain::Tron => {
            let bal = TronClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.sun.to_string(), bal.trx_display))
        }
        Chain::Stellar => {
            let bal = StellarClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.stroops.to_string(), bal.xlm_display))
        }
        Chain::Xrp => {
            let bal = XrpClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.drops.to_string(), bal.xrp_display))
        }
        Chain::Cardano => {
            let api_key = service
                .api_key_for(chain.str_id())
                .await
                .unwrap_or_default();
            let bal = CardanoClient::new(endpoints, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.lovelace.to_string(), bal.ada_display))
        }
        Chain::Polkadot => {
            let subscan = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                .await;
            let api_key = service.api_key_for(chain.str_id()).await;
            let bal = PolkadotClient::new(endpoints, subscan, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.planck.to_string(), bal.dot_display))
        }
        Chain::Sui => {
            let bal = SuiClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.mist.to_string(), bal.sui_display))
        }
        Chain::Aptos => {
            let bal = AptosClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.octas.to_string(), bal.apt_display))
        }
        Chain::Ton => {
            let api_key = service.api_key_for(chain.str_id()).await;
            let bal = TonClient::new(endpoints, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.nanotons.to_string(), bal.ton_display))
        }
        Chain::Near => {
            let bal = NearClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.yocto_near, bal.near_display))
        }
        Chain::Icp => {
            let bal = IcpClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.e8s.to_string(), bal.icp_display))
        }
        Chain::Monero => {
            let bal = MoneroClient::new(endpoints).fetch_balance(0).await?;
            Ok(summary_native(bal.piconeros.to_string(), bal.xmr_display))
        }
        Chain::Zcash => {
            let bal = ZcashClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::BitcoinGold => {
            let bal = BitcoinGoldClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Decred => {
            let bal = DecredClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_atoms.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Kaspa => {
            let bal = KaspaClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_sompi.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Dash => {
            let bal = DashClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Bittensor => {
            let taostats = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                .await;
            let api_key = service.api_key_for(chain.str_id()).await;
            let bal = BittensorClient::new(endpoints, taostats, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.rao.to_string(), bal.tao_display))
        }
        c => Err(SpectraBridgeError::from(format!(
            "unsupported chain: {c:?}"
        ))),
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
            let Some(explorer_base) = chain.evm_explorer_api_base() else {
                return json_response(&Vec::<crate::fetch::chains::evm::EvmHistoryEntry>::new());
            };
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
                .fetch_history(address, explorer_base, api_key_str, chain.evm_chain_id())
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

fn summary_native(smallest_unit: String, amount_display: String) -> NativeBalanceSummary {
    NativeBalanceSummary {
        smallest_unit,
        amount_display,
        utxo_count: 0,
    }
}
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Derive the account-level xpub (mainnet, canonical `xpub…` encoding)
    /// from a BIP39 mnemonic phrase.
    ///
    /// `account_path` is the **hardened account path** only, e.g.:
    ///   - `"m/84'/0'/0'"` → native SegWit (BIP84)
    ///   - `"m/49'/0'/0'"` → nested SegWit (BIP49)
    ///   - `"m/44'/0'/0'"` → legacy P2PKH (BIP44)
    ///
    /// `passphrase` is the optional BIP39 passphrase — pass `""` for none.
    pub fn derive_bitcoin_account_xpub_typed(
        &self,
        mnemonic_phrase: String,
        passphrase: String,
        account_path: String,
    ) -> Result<String, SpectraBridgeError> {
        crate::derivation::xpub_walker::derive_account_xpub(
            &mnemonic_phrase,
            &passphrase,
            &account_path,
        )
        .map_err(Into::into)
    }

    /// Derive a contiguous range of child addresses from an account-level
    /// extended public key (xpub/ypub/zpub).
    ///
    /// - `change` — 0 for external/receive, 1 for internal/change.
    /// - `start_index`, `count` — [start, start+count) scan window.
    pub async fn derive_bitcoin_hd_address_strings(
        &self,
        xpub: String,
        change: u32,
        start_index: u32,
        count: u32,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let children =
            crate::derivation::xpub_walker::derive_children(&xpub, change, start_index, count)?;
        Ok(children.into_iter().map(|c| c.address).collect())
    }

    /// Return the first address on the `change` leg (0 = receive, 1 = change)
    /// that has zero confirmed/unconfirmed history, scanning up to
    /// `gap_limit` candidates. Returns the derived address string, or
    /// `None` if every candidate in the `gap_limit` window had activity.
    pub async fn fetch_bitcoin_next_unused_address_typed(
        &self,
        xpub: String,
        change: u32,
        gap_limit: u32,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let endpoints = self.endpoints_for("bitcoin").await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        let next = crate::derivation::xpub_walker::fetch_next_unused_address(
            &client, &xpub, change, gap_limit,
        )
        .await?;
        Ok(next.map(|c| c.address))
    }
    /// Fetch USD spot prices for the supplied coins from `provider`.
    ///
    /// `provider` is the Swift-side display name (e.g. "CoinGecko").
    /// `coins` are the tracked tokens. All providers use their public
    /// endpoints — no API key plumbing.
    pub async fn fetch_prices_typed(
        &self,
        provider: String,
        coins: Vec<crate::price::PriceRequestCoin>,
    ) -> Result<std::collections::HashMap<String, f64>, SpectraBridgeError> {
        tracing::debug!(provider = %provider, coins = coins.len(), "fetch_prices enter");
        let parsed_provider = match crate::price::PriceProvider::from_raw(&provider) {
            Some(p) => p,
            None => {
                tracing::warn!(provider = %provider, "unknown price provider");
                return Err(format!("unknown price provider: {provider}").into());
            }
        };
        match crate::price::fetch_prices(parsed_provider, &coins).await {
            Ok(quotes) => {
                tracing::debug!(provider = %provider, returned = quotes.len(), "fetch_prices ok");
                Ok(quotes)
            }
            Err(e) => {
                tracing::error!(provider = %provider, error = %e, "fetch_prices failed");
                Err(SpectraBridgeError::from(e))
            }
        }
    }

    /// Typed variant — accepts typed currency list and returns typed map directly.
    pub async fn fetch_fiat_rates_typed(
        &self,
        provider: String,
        currencies: Vec<String>,
    ) -> Result<std::collections::HashMap<String, f64>, SpectraBridgeError> {
        tracing::debug!(provider = %provider, currencies = currencies.len(), "fetch_fiat_rates enter");
        let parsed_provider = match crate::price::FiatRateProvider::from_raw(&provider) {
            Some(p) => p,
            None => {
                tracing::warn!(provider = %provider, "unknown fiat rate provider");
                return Err(format!("unknown fiat rate provider: {provider}").into());
            }
        };
        match crate::price::fetch_fiat_rates(parsed_provider, &currencies).await {
            Ok(rates) => {
                tracing::debug!(provider = %provider, returned = rates.len(), "fetch_fiat_rates ok");
                Ok(rates)
            }
            Err(e) => {
                tracing::error!(provider = %provider, error = %e, "fetch_fiat_rates failed");
                Err(SpectraBridgeError::from(e))
            }
        }
    }
}

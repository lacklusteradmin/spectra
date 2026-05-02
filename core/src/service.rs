//! WalletService — the stateful async UniFFI object that Swift / Kotlin talk to.
//!
//! New methods should land in a matching sub-module (`chain_client`,
//! `helpers`, `derivation_methods`, `pricing_methods`, `state_methods`,
//! `standalone`) rather than `mod.rs` itself.
//!
//! ## FFI dichotomy
//!
//! This file owns the **dispatched path** — methods that match on `Chain`
//! and call into per-chain clients. See `ffi.rs` for the **typed path** —
//! free functions exporting via UniFFI Records. New endpoints that don't
//! dispatch on `Chain` belong in `ffi.rs` rather than here. Within this
//! file, the historical `params: serde_json::Value` shape has been
//! replaced with typed `*SendParams` structs in `service::types` for
//! migrated chains; new chain dispatch arms should follow that pattern,
//! not invent fresh ad-hoc JSON shapes.
//!
//! ## Service properties
//!
//! - All chain operations are `async` internally; UniFFI 0.29 with the tokio
//!   feature wraps them into `async fn` on the Swift side automatically.
//! - The service does not own secrets. It receives private key bytes per-call
//!   (Swift reads from Keychain and passes them in).
//! - Endpoint lists are set at construction time and rebuilt via
//!   `update_endpoints_typed`.
//! - Frozen chain-id discriminants live in `crate::registry::Chain`.

// `WalletService` types, helpers, and per-domain `impl` blocks all live
// in this file (sections at the bottom were merged from the old
// `service_*.rs` files for clarity).

pub(crate) use crate::fetch::chains::{
    aptos::AptosClient, bitcoin::BitcoinClient, bitcoin::UtxoTxStatus,
    bitcoin_cash::BitcoinCashClient, bitcoin_gold::BitcoinGoldClient, bitcoin_sv::BitcoinSvClient,
    bittensor::BittensorClient, cardano::CardanoClient, dash::DashClient, decred::DecredClient,
    dogecoin::DogecoinClient, evm::EvmClient, icp::IcpClient, kaspa::KaspaClient,
    litecoin::LitecoinClient, monero::MoneroClient, near::NearClient, polkadot::PolkadotClient,
    solana::SolanaClient, stellar::StellarClient, sui::SuiClient, ton::TonClient,
    tron::TronClient, xrp::XrpClient, zcash::ZcashClient,
};
pub(crate) use crate::fetch::history_store::HistoryPaginationStore;
pub(crate) use crate::http::HttpClient;
pub(crate) use crate::registry::{Chain, EndpointSlot};
pub(crate) use crate::send::chains::bitcoin::{
    sign_and_broadcast as bitcoin_sign_and_broadcast, BitcoinSendParams,
};
pub(crate) use crate::state::{
    reduce_state_in_place, AssetHolding, CoreAppState, StateCommand, WalletSummary,
};
pub(crate) use crate::store::secret_store::SecretStore;
pub(crate) use crate::SpectraBridgeError;

pub(crate) use serde_json::json;
pub(crate) use std::sync::Arc;
pub(crate) use tokio::sync::RwLock;

// ── Endpoint index (internal — pre-indexed for O(1) chain_id lookup) ──────

#[derive(Debug, Clone, Default)]
pub(crate) struct EndpointIndex {
    endpoints: std::collections::HashMap<u32, Arc<Vec<String>>>,
    api_keys: std::collections::HashMap<u32, String>,
}

impl EndpointIndex {
    fn from_list(list: Vec<ChainEndpoints>) -> Self {
        let mut endpoints = std::collections::HashMap::with_capacity(list.len());
        let mut api_keys = std::collections::HashMap::new();
        for entry in list {
            endpoints.insert(entry.chain_id, Arc::new(entry.endpoints));
            if let Some(key) = entry.api_key {
                api_keys.insert(entry.chain_id, key);
            }
        }
        Self { endpoints, api_keys }
    }
}

// ── WalletService — primary UniFFI-exported object ────────────────────────

/// Swift holds one instance for the lifetime of the app session.
#[derive(uniffi::Object)]
pub struct WalletService {
    pub(crate) endpoints: Arc<RwLock<EndpointIndex>>,
    /// Per-wallet history pagination state (cursor / page / exhaustion).
    pub(crate) history_pagination: Arc<HistoryPaginationStore>,
    /// Optional Keychain delegate (set via `set_secret_store`).
    pub(crate) secret_store: Arc<std::sync::RwLock<Option<Arc<dyn SecretStore>>>>,
    /// Canonical in-memory wallet + holdings state.
    pub(crate) wallet_state: Arc<RwLock<CoreAppState>>,
    /// User's Etherscan V2 API key. Shared across all EVM chains: Etherscan v2
    /// dispatches by `chainid` parameter against a single host.
    pub(crate) etherscan_api_key: Arc<std::sync::RwLock<String>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    #[uniffi::constructor]
    pub fn new_typed(endpoints: Vec<ChainEndpoints>) -> Result<Arc<Self>, SpectraBridgeError> {
        Ok(Arc::new(Self {
            endpoints: Arc::new(RwLock::new(EndpointIndex::from_list(endpoints))),
            history_pagination: Arc::new(HistoryPaginationStore::new()),
            secret_store: Arc::new(std::sync::RwLock::new(None)),
            wallet_state: Arc::new(RwLock::new(CoreAppState::default())),
            etherscan_api_key: Arc::new(std::sync::RwLock::new(String::new())),
        }))
    }

    pub async fn update_endpoints_typed(&self, endpoints: Vec<ChainEndpoints>) -> Result<(), SpectraBridgeError> {
        let mut guard = self.endpoints.write().await;
        *guard = EndpointIndex::from_list(endpoints);
        Ok(())
    }

    /// Swift pushes the user's Etherscan V2 API key here. Used for EVM history
    /// fetches across every indexed EVM chain (chainid is passed as a query
    /// param, so one key covers all of them).
    pub fn set_etherscan_api_key(&self, key: String) {
        if let Ok(mut guard) = self.etherscan_api_key.write() {
            *guard = key;
        }
    }

    // `fetch_balance` and `fetch_native_balance_summary_auto` live in the
    // plain-impl block below — internal helpers, not exported to Swift.

    pub async fn fetch_solana_balance_typed(
        &self,
        address: String,
    ) -> Result<crate::fetch::chains::solana::SolanaBalance, SpectraBridgeError> {
        let endpoints = self.endpoints_for(2).await;
        let client = crate::fetch::chains::solana::SolanaClient::new(endpoints);
        client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)
    }

    pub async fn fetch_near_balance_typed(
        &self,
        address: String,
    ) -> Result<crate::fetch::chains::near::NearBalance, SpectraBridgeError> {
        let endpoints = self.endpoints_for(17).await;
        let client = crate::fetch::chains::near::NearClient::new(endpoints);
        client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)
    }

    pub async fn fetch_erc20_balance_typed(
        &self,
        chain_id: u32,
        contract: String,
        holder: String,
    ) -> Result<crate::fetch::chains::evm::Erc20Balance, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!("fetch_token_balance_typed: unsupported chain_id: {chain_id}"))
        })?;
        let endpoints = self.endpoints_for(chain.id()).await;
        let client = crate::fetch::chains::evm::EvmClient::new(endpoints, chain.evm_chain_id());
        client.fetch_erc20_balance(&contract, &holder).await.map_err(SpectraBridgeError::from)
    }

    /// Unified per-chain native balance summary, replacing chain-specific JSON
    /// decoding on the Swift side. Smallest unit is returned as a decimal
    /// string (sats / wei / lamports / yocto-NEAR / ...) so callers can `UInt64`
    /// or `BigInt` parse as appropriate. `amount_display` is the human-readable
    /// native amount as decimal string. `utxo_count` is 0 for non-UTXO chains.
    pub async fn fetch_native_balance_summary(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        let chain = chain_for_id(chain_id)?;
        let client = ChainClient::build(chain, self).await?;
        client.fetch_native_balance_summary(&address).await
    }

    // `fetch_history` lives in the plain-impl block below (JSON shuttle —
    // kept internal, not exported to Swift).

    /// Fetch history for `address` on `chain_id` and normalize the raw
    /// chain-specific shape into a standard `NormalizedHistoryItem` array,
    /// returning typed records directly across the FFI boundary.
    pub async fn fetch_normalized_history(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<Vec<crate::fetch::history_decode::NormalizedHistoryItem>, SpectraBridgeError> {
        let raw = self.fetch_history(chain_id, address).await?;
        let entries = crate::history::normalize_chain_history(chain_id, &raw);
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
        chain_id: u32,
        address: String,
    ) -> Result<bool, SpectraBridgeError> {
        let raw = self.fetch_history(chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_entry_count(raw) > 0)
    }

    /// Fetch history JSON and return the top-level entry count.
    pub async fn fetch_history_entry_count(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<u32, SpectraBridgeError> {
        let raw = self.fetch_history(chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_entry_count(raw))
    }

    /// Fetch history JSON and return the set of confirmed `txid`s.
    /// Used to reconcile pending transactions with on-chain confirmations.
    pub async fn fetch_history_confirmed_txids(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let raw = self.fetch_history(chain_id, address).await?;
        Ok(crate::diagnostics::diagnostics_history_confirmed_txids(raw))
    }

    /// Fused Bitcoin HD history page: derive external+change addresses from
    /// `xpub`, concurrently fetch each address's history, and merge into a
    /// deduplicated page truncated to `limit`. Scan window is 20 external +
    /// 10 change, matching the legacy Swift orchestration this replaces.
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

    // ----------------------------------------------------------------
    // Token balance (ERC-20 / SPL / NEP-141 / TRC-20 / Stellar assets)
    // ----------------------------------------------------------------

    /// Fetch balances for a list of tokens in one call.
    ///
    /// For Solana `contract` is the mint address; for Sui / Aptos it is the
    /// coin type; for TON it is the jetton master address.
    ///
    /// Tokens that fail to fetch are returned with `balance_raw = "0"` so the
    /// caller always gets back the full list.
    pub async fn fetch_token_balances(
        &self,
        chain_id: u32,
        address: String,
        tokens: Vec<TokenDescriptor>,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("fetch_token_balances: unsupported chain_id: {chain_id}")))?;
        let endpoints = self.endpoints_for(chain.id()).await;

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
                let by_mint: std::collections::HashMap<&str, &crate::fetch::chains::solana::SplBalance> =
                    spl.iter().map(|b| (b.mint.as_str(), b)).collect();
                tokens
                    .iter()
                    .map(|t| {
                        let b = by_mint.get(t.contract.as_str());
                        TokenBalanceResult {
                            contract_address: t.contract.clone(),
                            symbol: t.symbol.clone(),
                            decimals: t.decimals,
                            balance_raw: b.map(|b| b.balance_raw.clone()).unwrap_or_else(|| "0".to_string()),
                            balance_display: b.map(|b| b.balance_display.clone()).unwrap_or_else(|| "0".to_string()),
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
                            let display = {
                                let div = 10u128.pow(decimals as u32);
                                let whole = raw / div;
                                let frac = raw % div;
                                if frac == 0 { whole.to_string() }
                                else { format!("{whole}.{frac:0>prec$}", prec = decimals as usize) }
                            };
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
            Chain::Sui => {
                use futures::future::join_all;
                let client = std::sync::Arc::new(SuiClient::new(endpoints));
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
                            let display = format_decimals(raw as u128, decimals);
                            TokenBalanceResult {
                                contract_address: coin_type,
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
            Chain::Aptos => {
                use futures::future::join_all;
                let client = std::sync::Arc::new(AptosClient::new(endpoints));
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
                            let display = format_decimals(raw as u128, decimals);
                            TokenBalanceResult {
                                contract_address: coin_type,
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
            Chain::Ton => {
                // TON — jetton balances via TonCenter v3 API. The v3 endpoint
                // lives in the chain's Secondary slot (registered as id + 100 = 116).
                let v3_endpoints = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = TonClient::new(endpoints, api_key).with_v3_endpoints(v3_endpoints);
                let jetton_balances = client
                    .fetch_jetton_balances(&address)
                    .await
                    .unwrap_or_default();

                tokens.iter().map(|t| {
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
                }).collect()
            }
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "fetch_token_balances: unsupported chain: {c:?}"
                )))
            }
        };

        Ok(results)
    }

    // ----------------------------------------------------------------
    // Unified execute_send — collapses derive → payload → sign trampoline
    // ----------------------------------------------------------------

    /// Derive key material, build the chain-specific payload, sign, and
    /// broadcast in a single call.
    ///
    /// This eliminates the Swift↔Rust trampoline where Swift held a closure
    /// between derivation and signing. Swift now passes the seed phrase (or
    /// raw private key) directly, and Rust handles the entire pipeline.
    pub async fn execute_send(
        &self,
        request: crate::send::SendExecutionRequest,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        // 1. Derive key material (or use provided private key).
        let (priv_hex, pub_hex) = if let Some(ref seed_phrase) = request.seed_phrase {
            let (_addr, priv_h, pub_h) =
                crate::derivation::derive_key_material_for_chain_with_overrides(
                    seed_phrase,
                    &request.chain_name,
                    &request.derivation_path,
                    request.derivation_overrides.as_ref(),
                )?;
            (priv_h, Some(pub_h))
        } else if let Some(ref pk) = request.private_key_hex {
            let normalized = pk.strip_prefix("0x").unwrap_or(pk).to_string();
            (normalized, None)
        } else {
            return Err(SpectraBridgeError::from(
                "execute_send: neither seed_phrase nor private_key_hex provided",
            ));
        };

        // 2. Build payload JSON and route to sign_and_send or sign_and_send_token.
        let is_token = request.contract_address.is_some();
        let params_json = self.build_execute_send_payload(&request, &priv_hex, &pub_hex)?;

        let result_json = if is_token {
            self.sign_and_send_token(request.chain_id, params_json).await?
        } else {
            self.sign_and_send(request.chain_id, params_json).await?
        };

        // 3. Classify broadcast result. `is_token` is intentionally unused —
        // `SendChain` is chain-family granularity, not token/native.
        let _ = is_token;
        let send_chain = Chain::from_id(request.chain_id)
            .map(Chain::send_chain)
            .unwrap_or(crate::send::payload::SendChain::Bitcoin);
        let outcome =
            crate::send::payload::classify_send_broadcast_result(send_chain, result_json.clone());

        // 4. For EVM chains, decode the typed result here so Swift doesn't
        // have to round-trip through `decode_evm_send_result(json:)`.
        let evm = if Chain::from_id(request.chain_id).is_some_and(Chain::is_evm) {
            let fallback_nonce = request
                .evm_overrides
                .as_ref()
                .and_then(|o| o.nonce)
                .unwrap_or(0);
            Some(crate::send::ethereum::decode_evm_send_result_internal(
                &result_json,
                fallback_nonce,
            ))
        } else {
            None
        };

        Ok(crate::send::SendExecutionResult {
            result_json,
            transaction_hash: outcome.transaction_hash,
            payload_format: outcome.payload_format,
            evm,
        })
    }

    // `build_execute_send_payload` lives in the plain-impl block below
    // (internal helper for `execute_send` — not exported to Swift).

    // ----------------------------------------------------------------
    // Bitcoin HD — seed → account xpub derivation
    // ----------------------------------------------------------------


    // ----------------------------------------------------------------
    // Bitcoin HD multi-address (xpub / ypub / zpub)
    // ----------------------------------------------------------------


    // `fetch_bitcoin_xpub_balance` lives in the plain-impl block below
    // (JSON shuttle — kept internal, not exported to Swift).


    // ----------------------------------------------------------------
    // Price / fiat rate service
    // ----------------------------------------------------------------



    // ----------------------------------------------------------------
    // EVM paginated history (native + ERC-20 token transfers)
    // ----------------------------------------------------------------

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
        chain_id: u32,
        address: String,
        tokens: Vec<TokenDescriptor>,
        page: u32,
        page_size: u32,
    ) -> Result<crate::fetch::history_decode::EvmHistoryPageDecoded, SpectraBridgeError> {
        use crate::fetch::history_decode::{
            EvmHistoryPageDecoded, EvmNativeTransferItem, EvmTokenTransferItem,
        };

        // Only EVM chains are supported.
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_evm_history_page: chain_id {chain_id} not supported"
            ))
        })?;

        let eps = self.endpoints_for(chain.id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());

        // Etherscan V2 is a unified multichain API: one host, chainid query
        // parameter dispatches. Per-chain subdomains (arbiscan.io, etc.) do not
        // host a V2 path — everything routes through api.etherscan.io.
        let explorer_base = "https://api.etherscan.io";
        let etherscan_chain_id = chain.evm_chain_id();
        let api_key_owned: String = self
            .etherscan_api_key
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        let api_key_str = if api_key_owned.is_empty() { None } else { Some(api_key_owned.as_str()) };

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

    // ----------------------------------------------------------------
    // Typed token-array wrappers (no JSON serialization on caller side)
    // ----------------------------------------------------------------

    pub async fn fetch_evm_token_balances_batch_typed(
        &self,
        chain_id: u32,
        address: String,
        tokens: Vec<TokenDescriptor>,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_evm_token_balances_batch: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let mut results = Vec::with_capacity(tokens.len());
        for t in &tokens {
            let contract = t.contract.to_lowercase();
            if contract.is_empty() { continue; }
            let raw = client.fetch_erc20_balance_of(&contract, &address)
                .await
                .unwrap_or(0);
            let decimals = t.decimals;
            let balance_display = {
                let divisor = 10u128.pow(decimals as u32);
                let whole = raw / divisor;
                let frac = raw % divisor;
                if frac == 0 {
                    whole.to_string()
                } else {
                    let frac_s = format!("{:0>width$}", frac, width = decimals as usize);
                    let trimmed = frac_s.trim_end_matches('0');
                    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
                    format!("{}.{}", whole, capped)
                }
            };
            results.push(TokenBalanceResult {
                contract_address: contract,
                symbol: t.symbol.clone(),
                decimals,
                balance_raw: raw.to_string(),
                balance_display,
            });
        }
        Ok(results)
    }

    /// Fetch EVM history for diagnostics and return a fully-built
    /// `EthereumTokenTransferHistoryDiagnostics` record. On network or
    /// chain-support failure the record is seeded with an error description.
    pub async fn fetch_evm_history_diagnostics(
        &self,
        chain_id: u32,
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

    // ----------------------------------------------------------------
    // ENS resolution
    // ----------------------------------------------------------------

    /// Resolve an ENS name to an Ethereum address via the ENS Ideas public API.
    /// Returns the resolved address, or `None` if the name has no registered address.
    pub async fn resolve_ens_name_typed(
        &self,
        name: String,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let eps = self.endpoints_for(1).await;
        let client = EvmClient::new(eps, 1);
        let address = client
            .resolve_ens(&name)
            .await
            .map_err(SpectraBridgeError::from)?;
        Ok(address.filter(|a| !a.is_empty()))
    }

    // ----------------------------------------------------------------
    // EVM utilities (contract detection, nonce lookup)
    // ----------------------------------------------------------------

    /// Returns true iff `address` has deployed bytecode on the given EVM chain.
    pub async fn fetch_evm_has_contract_code(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<bool, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_evm_has_contract_code: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let code = client.fetch_code(&address).await.map_err(SpectraBridgeError::from)?;
        Ok(crate::send::flow::core_evm_has_contract_code(code))
    }

    /// Fetch the nonce of a submitted transaction by hash on an EVM chain.
    /// Used to pre-fill the replacement-tx nonce field.
    pub async fn fetch_evm_tx_nonce_typed(
        &self,
        chain_id: u32,
        tx_hash: String,
    ) -> Result<u64, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_evm_tx_nonce: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        client.fetch_tx_nonce(&tx_hash).await.map_err(SpectraBridgeError::from)
    }

    // `fetch_utxo_fee_preview` and `broadcast_raw` live in the plain-impl
    // block below (JSON shuttles — kept internal, not exported to Swift).

    /// Typed wrapper around `broadcast_raw`: runs the broadcast then extracts
    /// the named field (typically `"txid"` or `"digest"`) from the result JSON.
    /// Returns the field value as a string (empty string when missing —
    /// matches the prior `rustField(...)` semantics on the Swift side).
    pub async fn broadcast_raw_extract(
        &self,
        chain_id: u32,
        payload: String,
        result_field: String,
    ) -> Result<String, SpectraBridgeError> {
        let json = self.broadcast_raw(chain_id, payload).await?;
        Ok(crate::send::preview_decode::extract_json_string_field(
            json,
            result_field,
        ))
    }

    // ----------------------------------------------------------------
    // EVM receipt polling
    // ----------------------------------------------------------------

    /// Fused fetch + classification for an EVM receipt: returns
    /// `Some(classification)` once the receipt has been mined, or `None`
    /// while the transaction is still pending.
    pub async fn fetch_evm_receipt_classification(
        &self,
        chain_id: u32,
        tx_hash: String,
    ) -> Result<Option<crate::send::flow::EvmReceiptClassification>, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_evm_receipt_classification: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let Some(receipt) = client.fetch_receipt(&tx_hash).await.map_err(SpectraBridgeError::from)? else {
            return Ok(None);
        };
        let json = serde_json::to_string(&receipt)?;
        Ok(crate::send::flow::classify_evm_receipt_json(json))
    }

    // `fetch_evm_send_preview` / `fetch_tron_send_preview` /
    // `fetch_simple_chain_send_preview` live in the plain-impl block below
    // (JSON shuttles — kept internal, not exported to Swift). Their typed
    // wrappers below call into those internal helpers.

    // ----------------------------------------------------------------
    // Typed send-preview wrappers (fuse fetch + decode in Rust)
    // ----------------------------------------------------------------

    /// Typed EVM send preview: fetches the raw preview JSON then decodes it
    /// into `EthereumSendPreview` with the caller-supplied nonce / fee
    /// overrides applied. Returns `None` when the decoder rejects the payload.
    pub async fn fetch_evm_send_preview_typed(
        &self,
        chain_id: u32,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::ethereum_send::EvmCustomFeeConfiguration>,
    ) -> Result<Option<crate::wallet_core::EthereumSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_evm_send_preview(chain_id, from, to, value_wei, data_hex)
            .await?;
        Ok(crate::send::preview_decode::build_evm_send_preview_record(
            crate::ethereum_send::EvmPreviewDecodeInput {
                raw_json: raw,
                explicit_nonce,
                custom_fees,
            },
        ))
    }

    /// Lightweight EVM address probe used for send-flow chain-risk warnings.
    /// Fetches nonce + native balance concurrently and returns both typed,
    /// skipping the fee/gas work of the full preview.
    pub async fn fetch_evm_address_probe(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<EvmAddressProbe, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_evm_address_probe: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let (nonce_res, bal_res) = tokio::join!(client.fetch_nonce(&address), client.fetch_balance(&address));
        let nonce = nonce_res.unwrap_or(0) as i64;
        let balance_wei: u128 = bal_res
            .map(|b| b.balance_wei.parse::<u128>().unwrap_or(0))
            .unwrap_or(0);
        Ok(EvmAddressProbe {
            nonce,
            balance_eth: balance_wei as f64 / 1e18,
        })
    }

    /// Typed Tron send preview wrapper around `fetch_tron_send_preview` +
    /// `build_tron_send_preview_record`.
    pub async fn fetch_tron_send_preview_typed(
        &self,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<Option<crate::wallet_core::TronSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_tron_send_preview(address, symbol, contract_address)
            .await?;
        Ok(crate::send::preview_decode::build_tron_send_preview_record(raw))
    }

    /// Typed UTXO fee preview wrapper (BTC / LTC / BCH / BSV single-address
    /// flow). Fuses `fetch_utxo_fee_preview` + `build_utxo_send_preview_record`.
    pub async fn fetch_utxo_fee_preview_typed(
        &self,
        chain_id: u32,
        address: String,
        fee_rate_svb: u64,
    ) -> Result<Option<crate::wallet_core::BitcoinSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_utxo_fee_preview(chain_id, address, fee_rate_svb)
            .await?;
        Ok(crate::send::preview_decode::build_utxo_send_preview_record(raw))
    }

    /// Typed Dogecoin send preview: runs the UTXO fee-preview fetch on the
    /// Dogecoin chain then decodes with the requested amount + fee priority.
    pub async fn fetch_dogecoin_send_preview_typed(
        &self,
        address: String,
        requested_amount: f64,
        fee_priority: String,
    ) -> Result<Option<crate::wallet_core::DogecoinSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_utxo_fee_preview(Chain::Dogecoin.id(), address, 0)
            .await?;
        Ok(crate::send::preview_decode::build_dogecoin_send_preview_record(
            raw,
            requested_amount,
            fee_priority,
        ))
    }

    /// Typed Bitcoin HD send preview: concurrently fetches the xpub balance
    /// and the Bitcoin fee estimate then decodes into `BitcoinSendPreview`.
    pub async fn fetch_bitcoin_hd_send_preview_typed(
        &self,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<Option<crate::wallet_core::BitcoinSendPreview>, SpectraBridgeError> {
        let (balance_json, fee_json) = tokio::try_join!(
            self.fetch_bitcoin_xpub_balance(xpub, receive_count, change_count),
            self.fetch_fee_estimate(Chain::Bitcoin.id()),
        )?;
        Ok(crate::send::preview_decode::build_bitcoin_hd_send_preview_record(
            balance_json,
            fee_json,
        ))
    }

    /// Typed simple-chain send preview: fuses `fetch_simple_chain_send_preview`
    /// + `build_simple_chain_preview` so Swift never sees the intermediate JSON.
    pub async fn fetch_simple_chain_send_preview_typed(
        &self,
        chain_id: u32,
        address: String,
        chain: crate::send::preview_decode::SimpleChain,
    ) -> Result<crate::send::preview_decode::SimpleChainPreview, SpectraBridgeError> {
        let raw = self
            .fetch_simple_chain_send_preview(chain_id, address)
            .await?;
        Ok(crate::send::preview_decode::build_simple_chain_preview(
            raw, chain,
        ))
    }

    // ----------------------------------------------------------------
    // Phase 2 — SQLite state persistence
    // ----------------------------------------------------------------



    // ----------------------------------------------------------------
    // Phase 2.1 — WalletStore CRUD (SQLite-backed snapshot)
    // ----------------------------------------------------------------



    // ----------------------------------------------------------------
    // Phase 2.1 — Relational wallet state (keypool + owned addresses)
    //
    // These replace UserDefaults JSON blobs on the Swift side.
    // All calls run in spawn_blocking because rusqlite is not async.
    // ----------------------------------------------------------------










    // ----------------------------------------------------------------
    // Phase 2.8 — Transaction history persistence (SQLite-backed)
    // ----------------------------------------------------------------






    // ----------------------------------------------------------------
    // Phase 3 — typed wallet state (no JSON round-trip)
    // ----------------------------------------------------------------



    // ----------------------------------------------------------------
    // Phase 2.3 — History pagination state
    // ----------------------------------------------------------------












    // ----------------------------------------------------------------
    // Phase 2.7 — SecretStore (Keychain delegate)
    // ----------------------------------------------------------------

    /// Register the platform Keychain implementation. Must be called once at
    /// app start before any code path that reads or writes secrets. Rust code
    /// that needs secret I/O calls the delegate directly via `self.secret_store`;
    /// there are deliberately no pass-through FFI wrappers — all secret traffic
    /// is driven by Rust.
    pub fn set_secret_store(&self, store: Arc<dyn SecretStore>) {
        if let Ok(mut guard) = self.secret_store.write() {
            *guard = Some(store);
        }
    }

    // ----------------------------------------------------------------
    // UTXO tx status
    // ----------------------------------------------------------------

    /// Fetch confirmation status for a UTXO chain transaction.
    /// Returns a typed record so Swift can read `confirmed`/`block_height`/
    /// `confirmations` fields without bouncing through JSON.
    /// Supported chain_ids: 0 (BTC), 3 (DOGE), 5 (LTC), 6 (BCH), 22 (BSV).
    pub async fn fetch_utxo_tx_status_typed(
        &self,
        chain_id: u32,
        txid: String,
    ) -> Result<UtxoTxStatus, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("fetch_utxo_tx_status: unsupported chain_id: {chain_id}")))?;
        let endpoints = self.endpoints_for(chain.id()).await;
        let status: UtxoTxStatus = match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::Zcash => {
                let client = ZcashClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::BitcoinGold => {
                let client = BitcoinGoldClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::Decred => {
                let client = DecredClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::Kaspa => {
                let client = KaspaClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            Chain::Dash => {
                let client = DashClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            c => return Err(SpectraBridgeError::from(format!(
                "fetch_utxo_tx_status: unsupported chain: {c:?}"
            ))),
        };
        Ok(status)
    }
}

impl WalletService {
    pub(crate) async fn endpoints_for(&self, chain_id: u32) -> Arc<Vec<String>> {
        // Cached `Arc<Vec<String>>` returned directly — no clone of inner Vec.
        let guard = self.endpoints.read().await;
        guard
            .endpoints
            .get(&chain_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(Vec::new()))
    }

    pub(crate) async fn api_key_for(&self, chain_id: u32) -> Option<String> {
        let guard = self.endpoints.read().await;
        guard.api_keys.get(&chain_id).cloned()
    }

    /// Internal fee-estimate helper used by `estimate_send_fee_*` and the
    /// broadcast pipelines. Previously also exported to Swift as
    /// `fetchFeeEstimateJSON` but that wrapper had no callers — un-exported
    /// 2026-04-19 to remove a dead JSON boundary.
    ///
    /// BTC returns the serialized `FeeRate` struct; EVM returns the serialized
    /// `EvmFeeEstimate` struct; all other chains return a unified
    /// `{chain_id, native_fee_raw, native_fee_display, unit, source}` JSON.
    /// `source` is `"rpc"` for live values and `"static"` for hardcoded defaults.
    pub(crate) async fn fetch_fee_estimate(
        &self,
        chain_id: u32,
    ) -> Result<String, SpectraBridgeError> {
        let Some(chain) = Chain::from_id(chain_id) else {
            return Ok(json!({"note": "fee estimation not supported for this chain"}).to_string());
        };
        let endpoints = self.endpoints_for(chain.id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                let fee = client.fetch_fee_rate(6).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&fee)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let fee = client.fetch_fee_estimate().await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&fee)?)
            }
            // Chains with live RPC fee fetches.
            Chain::Xrp => {
                let client = XrpClient::new(endpoints);
                let drops = client.fetch_fee().await.map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, drops as u128, chain.native_decimals(), chain.coin_symbol(), "rpc"))
            }
            Chain::Stellar => {
                let client = StellarClient::new(endpoints);
                let stroops = client.fetch_base_fee().await.map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, stroops as u128, chain.native_decimals(), chain.coin_symbol(), "rpc"))
            }
            Chain::Aptos => {
                let client = AptosClient::new(endpoints);
                let price = client.fetch_gas_price().await.map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, price as u128, chain.native_decimals(), chain.coin_symbol(), "rpc"))
            }
            // NEAR's static fee overflows u128 — fall back to the string variant.
            Chain::Near => Ok(fee_preview_str(
                chain_id, "1000000000000000000000", "0.001", chain.coin_symbol(), "static",
            )),
            // Every remaining supported chain returns a flat static fee from
            // `Chain::static_fee_units`. One arm replaces 18 near-identical ones.
            other => match other.static_fee_units() {
                Some(units) => Ok(fee_preview(
                    chain_id, units, chain.native_decimals(), chain.coin_symbol(), "static",
                )),
                None => Ok(json!({"note": "fee estimation not supported for this chain"}).to_string()),
            },
        }
    }

    // ----------------------------------------------------------------
    // Internal helpers — non-FFI. UniFFI exports every method of an
    // `#[uniffi::export]` impl block regardless of `pub(crate)`, so helpers
    // only called from other Rust methods live here to stay off the FFI
    // surface.
    // ----------------------------------------------------------------

    /// Fetch Bitcoin history JSON for `address` and decode it into typed
    /// `CoreBitcoinHistorySnapshot` records. Now internal-only — callers go
    /// through `fetch_bitcoin_hd_history_page` for the full HD scan or
    /// `fetch_normalized_history` for single-address paths.
    pub(crate) async fn fetch_bitcoin_history_snapshots(
        &self,
        address: String,
    ) -> Result<Vec<crate::history::CoreBitcoinHistorySnapshot>, SpectraBridgeError> {
        let raw = self.fetch_history(Chain::Bitcoin.id(), address).await?;
        Ok(crate::fetch::history_decode::history_decode_bitcoin_raw_snapshots(raw))
    }

    /// Sign and broadcast a transaction.
    ///
    /// `params_json` is chain-specific JSON containing addresses, amounts,
    /// and private keys (read from Keychain by Swift before calling).
    pub(crate) async fn sign_and_send(
        &self,
        chain_id: u32,
        params: serde_json::Value,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("sign_and_send: unsupported chain_id: {chain_id}")))?;
        let endpoints = self.endpoints_for(chain.id()).await;

        match chain {
            Chain::Bitcoin => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sats = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_rate_svb = params["fee_rate_svb"].as_f64().unwrap_or(10.0);
                let priv_hex = str_field(&params, "private_key_hex")?;
                let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                let send_params = BitcoinSendParams {
                    from_address: from.to_string(),
                    private_key_hex: priv_hex.to_string(),
                    to_address: to.to_string(),
                    amount_sats,
                    fee_rate: crate::fetch::chains::bitcoin::FeeRate { sats_per_vbyte: fee_rate_svb },
                    available_utxos: vec![],
                    network_mode: "mainnet".to_string(),
                    enable_rbf: true,
                };
                let r = bitcoin_sign_and_broadcast(&client, send_params)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            c if c.is_evm() => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let value_wei: u128 = params["value_wei"].as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("missing value_wei")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let overrides = read_evm_overrides(&params);
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let r = client
                    .sign_and_broadcast_with_overrides(from, to, value_wei, &priv_bytes, overrides)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Solana => {
                let from_bytes = hex_field(&params, "from_pubkey_hex")?;
                let from_arr: [u8; 32] = from_bytes.try_into().map_err(|_| "from pubkey wrong length")?;
                let to = str_field(&params, "to")?;
                let lamports = params["lamports"].as_u64().ok_or("missing lamports")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let priv_arr: [u8; 64] = priv_bytes.try_into().map_err(|_| "privkey wrong length")?;
                let client = SolanaClient::new(endpoints);
                let r = client.sign_and_broadcast(&from_arr, to, lamports, &priv_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Xrp => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let drops = params["drops"].as_u64().ok_or("missing drops")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                // public_key_hex is optional: derive compressed secp256k1 pubkey when absent.
                let derived_pub: String;
                let pub_hex: &str = match params["public_key_hex"].as_str().filter(|s| !s.is_empty()) {
                    Some(s) => s,
                    None => {
                        use secp256k1::{PublicKey as SecpPubKey, Secp256k1, SecretKey};
                        let secp = Secp256k1::new();
                        let secret = SecretKey::from_slice(&priv_bytes)
                            .map_err(|e| format!("bad privkey: {e}"))?;
                        derived_pub = hex::encode(SecpPubKey::from_secret_key(&secp, &secret).serialize());
                        &derived_pub
                    }
                };
                let client = XrpClient::new(endpoints);
                let r = client.sign_and_submit(from, to, drops, &priv_bytes, pub_hex)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Tron => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sun = params["amount_sun"].as_u64().ok_or("missing amount_sun")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = TronClient::new(endpoints);
                let r = client.sign_and_broadcast(from, to, amount_sun, &priv_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Sui => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let mist = params["mist"].as_u64().ok_or("missing mist")?;
                let gas_budget = params["gas_budget"].as_u64().unwrap_or(10_000_000);
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let client = SuiClient::new(endpoints);
                let r = client.sign_and_send(from, to, mist, gas_budget, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Aptos => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let octas = params["octas"].as_u64().ok_or("missing octas")?;
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let client = AptosClient::new(endpoints);
                let r = client.sign_and_submit(from, to, octas, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Near => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let yocto: u128 = params["yocto_near"].as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("missing yocto_near")?;
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let client = NearClient::new(endpoints);
                let r = client.sign_and_broadcast(from, to, yocto, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Dogecoin => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sat = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sat = params["fee_sat"].as_u64().unwrap_or(200_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = DogecoinClient::new(endpoints);
                let r = client.sign_and_broadcast(from, to, amount_sat, fee_sat, &priv_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Litecoin => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sat = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sat = params["fee_sat"].as_u64().unwrap_or(10_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = LitecoinClient::new(endpoints);
                let r = client.sign_and_broadcast(from, to, amount_sat, fee_sat, &priv_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::BitcoinCash => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sat = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sat = params["fee_sat"].as_u64().unwrap_or(1_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = BitcoinCashClient::new(endpoints);
                let r = client.sign_and_broadcast(from, to, amount_sat, fee_sat, &priv_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::BitcoinSV => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sat = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sat = params["fee_sat"].as_u64().unwrap_or(1_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = BitcoinSvClient::new(endpoints);
                let r = client.sign_and_broadcast(from, to, amount_sat, fee_sat, &priv_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Zcash => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sat = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sat = params["fee_sat"].as_u64().unwrap_or(1_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = ZcashClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(from, to, amount_sat, fee_sat, &priv_bytes)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::BitcoinGold => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sat = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sat = params["fee_sat"].as_u64().unwrap_or(1_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = BitcoinGoldClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(from, to, amount_sat, fee_sat, &priv_bytes)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Decred => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_atoms = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_atoms = params["fee_sat"].as_u64().unwrap_or(2_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = DecredClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(from, to, amount_atoms, fee_atoms, &priv_bytes)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Kaspa => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sompi = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sompi = params["fee_sat"].as_u64().unwrap_or(1_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = KaspaClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(from, to, amount_sompi, fee_sompi, &priv_bytes)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Dash => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sat = params["amount_sat"].as_u64().ok_or("missing amount_sat")?;
                let fee_sat = params["fee_sat"].as_u64().unwrap_or(2_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = DashClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(from, to, amount_sat, fee_sat, &priv_bytes)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Stellar => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let stroops = params["stroops"].as_i64().ok_or("missing stroops")?;
                // Accept 32-byte seed (raw import) or 64-byte expanded key (derived).
                let priv_raw = hex_field(&params, "private_key_hex")?;
                let priv_arr: [u8; 64] = if priv_raw.len() == 32 {
                    let mut expanded = [0u8; 64];
                    expanded[..32].copy_from_slice(&priv_raw);
                    expanded
                } else {
                    priv_raw.try_into().map_err(|_| "privkey must be 32 or 64 bytes")?
                };
                // public_key_hex is optional: derive ed25519 verifying key when absent.
                let pub_arr: [u8; 32] = match params["public_key_hex"].as_str().filter(|s| !s.is_empty()) {
                    Some(s) => hex::decode(s).map_err(|e| format!("pubkey hex: {e}"))?
                        .try_into().map_err(|_| "pubkey wrong length")?,
                    None => {
                        use ed25519_dalek::SigningKey;
                        let seed: [u8; 32] = priv_arr[..32].try_into()
                            .map_err(|_| "privkey seed too short")?;
                        SigningKey::from_bytes(&seed).verifying_key().to_bytes()
                    }
                };
                let client = StellarClient::new(endpoints);
                let r = client.sign_and_submit(from, to, stroops, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Cardano => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_lovelace = params["amount_lovelace"].as_u64().ok_or("missing amount_lovelace")?;
                let fee_lovelace = params["fee_lovelace"].as_u64().unwrap_or(170_000);
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let api_key = self.api_key_for(chain.id()).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let r = client.sign_and_broadcast(from, to, amount_lovelace, fee_lovelace, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Polkadot => {
                let p: PolkadotSendParams = parse_params(&params)?;
                let priv_arr: [u8; 32] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let subscan = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let r = client.sign_and_submit(&p.from, &p.to, p.planck, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                json_response(&r)
            }
            Chain::Bittensor => {
                let p: BittensorSendParams = parse_params(&params)?;
                let priv_arr: [u8; 32] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let taostats = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = BittensorClient::new(endpoints, taostats, api_key);
                let r = client.sign_and_submit(&p.from, &p.to, p.rao, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                json_response(&r)
            }
            Chain::Ton => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let nanotons = params["nanotons"].as_u64().ok_or("missing nanotons")?;
                let comment = params["comment"].as_str();
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let api_key = self.api_key_for(chain.id()).await;
                let client = TonClient::new(endpoints, api_key);
                let seqno = client.fetch_seqno(from).await?;
                let r = client.sign_and_send(to, nanotons, seqno, comment, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Icp => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let e8s = params["e8s"].as_u64().ok_or("missing e8s")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                // public_key_hex is optional: derive compressed secp256k1 pubkey when absent.
                let derived_pub: Vec<u8>;
                let pub_bytes: &[u8] = match params["public_key_hex"].as_str().filter(|s| !s.is_empty()) {
                    Some(s) => {
                        derived_pub = hex::decode(s).map_err(|e| format!("pubkey hex: {e}"))?;
                        &derived_pub
                    }
                    None => {
                        use secp256k1::{PublicKey as SecpPubKey, Secp256k1, SecretKey};
                        let secp = Secp256k1::new();
                        let secret = SecretKey::from_slice(&priv_bytes)
                            .map_err(|e| format!("bad privkey: {e}"))?;
                        derived_pub = SecpPubKey::from_secret_key(&secp, &secret).serialize().to_vec();
                        &derived_pub
                    }
                };
                let client = IcpClient::new(endpoints);
                let r = client.sign_and_submit(from, to, e8s, &priv_bytes, pub_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Monero => {
                let to = str_field(&params, "to")?;
                let piconeros = params["piconeros"].as_u64().ok_or("missing piconeros")?;
                let priority = params["priority"].as_u64().unwrap_or(2) as u32;
                let client = MoneroClient::new(endpoints);
                let r = client.send(to, piconeros, 0, priority).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            c => Err(SpectraBridgeError::from(format!("sign_and_send: unsupported chain: {c:?}"))),
        }
    }

    /// Sign and broadcast a token transfer on the given chain.
    ///
    /// `params_json` schema:
    ///   - EVM chains (1,11,12,13,20,21): `{"from": "0x…", "contract": "0x…",
    ///     "to": "0x…", "amount_raw": "<decimal string>", "private_key_hex": "…"}`
    ///     (`amount_raw` is scaled by token decimals on the Swift side)
    ///   - Tron (7): same EVM shape plus optional `"fee_limit_sun"` (default
    ///     100 TRX). Addresses are base58 (`T…`).
    ///   - Stellar (8): `{"from": "G…", "to": "G…", "stroops": <int>,
    ///     "asset_code": "USDC", "asset_issuer": "G…",
    ///     "private_key_hex": "<64-byte>", "public_key_hex": "<32-byte>"}`
    ///   - NEAR (17): `{"from": "alice.near", "contract": "token.near",
    ///     "to": "bob.near", "amount_raw": "<decimal>",
    ///     "private_key_hex": "<64-byte>", "public_key_hex": "<32-byte>"}`
    ///   - Solana (2): `{"from_pubkey_hex": "<32 hex>", "to": "<b58 wallet>",
    ///     "mint": "<b58 mint>", "amount_raw": "<decimal>",
    ///     "decimals": <u8>, "private_key_hex": "<64-byte>"}`
    pub(crate) async fn sign_and_send_token(
        &self,
        chain_id: u32,
        params: serde_json::Value,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("sign_and_send_token: unsupported chain_id: {chain_id}")))?;
        let endpoints = self.endpoints_for(chain.id()).await;

        match chain {
            c if c.is_evm() => {
                let from = str_field(&params, "from")?;
                let contract = str_field(&params, "contract")?;
                let to = str_field(&params, "to")?;
                let amount_raw: u128 = params["amount_raw"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| params["amount_raw"].as_u64().map(|n| n as u128))
                    .ok_or("missing amount_raw")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let overrides = read_evm_overrides(&params);
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let r = client
                    .sign_and_broadcast_erc20_with_overrides(
                        from, contract, to, amount_raw, &priv_bytes, overrides,
                    )
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Tron => {
                // Tron — TRC-20. Addresses are base58, amount is in token units,
                // `fee_limit_sun` defaults to 100 TRX (100_000_000 sun) which
                // covers typical USDT transfers (roughly 13-25 TRX actual cost).
                let from = str_field(&params, "from")?;
                let contract = str_field(&params, "contract")?;
                let to = str_field(&params, "to")?;
                let amount_raw: u128 = params["amount_raw"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| params["amount_raw"].as_u64().map(|n| n as u128))
                    .ok_or("missing amount_raw")?;
                let fee_limit_sun = params["fee_limit_sun"].as_u64().unwrap_or(100_000_000);
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = TronClient::new(endpoints);
                let r = client
                    .sign_and_broadcast_trc20(from, contract, to, amount_raw, fee_limit_sun, &priv_bytes)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Stellar => {
                // Stellar — custom issued asset payment. Uses same keypair
                // shape as native XLM sends (64-byte priv, 32-byte pub).
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let stroops = params["stroops"].as_i64().ok_or("missing stroops")?;
                let asset_code = str_field(&params, "asset_code")?;
                let asset_issuer = str_field(&params, "asset_issuer")?;
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into()
                    .map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into()
                    .map_err(|_| "pubkey wrong length")?;
                let client = StellarClient::new(endpoints);
                let r = client
                    .sign_and_submit_asset(
                        from,
                        to,
                        stroops,
                        asset_code,
                        asset_issuer,
                        &priv_arr,
                        &pub_arr,
                    )
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Near => {
                // NEAR — NEP-141 fungible token transfer (ft_transfer).
                let from = str_field(&params, "from")?;
                let contract = str_field(&params, "contract")?;
                let to = str_field(&params, "to")?;
                let amount_raw: u128 = params["amount_raw"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| params["amount_raw"].as_u64().map(|n| n as u128))
                    .ok_or("missing amount_raw")?;
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into()
                    .map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into()
                    .map_err(|_| "pubkey wrong length")?;
                let client = NearClient::new(endpoints);
                let r = client
                    .sign_and_broadcast_ft_transfer(
                        from,
                        contract,
                        to,
                        amount_raw,
                        &priv_arr,
                        &pub_arr,
                    )
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            Chain::Solana => {
                // Solana — SPL token transfer with idempotent ATA create.
                let from_arr: [u8; 32] = hex_field(&params, "from_pubkey_hex")?
                    .try_into()
                    .map_err(|_| "from pubkey wrong length")?;
                let to = str_field(&params, "to")?;
                let mint = str_field(&params, "mint")?;
                let amount_raw: u64 = params["amount_raw"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| params["amount_raw"].as_u64())
                    .ok_or("missing amount_raw")?;
                let decimals: u8 = params["decimals"]
                    .as_u64()
                    .map(|n| n as u8)
                    .ok_or("missing decimals")?;
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into()
                    .map_err(|_| "privkey wrong length")?;
                let client = SolanaClient::new(endpoints);
                let r = client
                    .sign_and_broadcast_spl(
                        &from_arr,
                        to,
                        mint,
                        amount_raw,
                        decimals,
                        &priv_arr,
                    )
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            c => Err(SpectraBridgeError::from(format!(
                "sign_and_send_token: unsupported chain: {c:?}"
            ))),
        }
    }

    // ----------------------------------------------------------------
    // Internal JSON-returning helpers (not exported to Swift — the typed
    // wrappers above in the exported impl block call these and translate
    // the JSON into UniFFI records at the boundary).
    // ----------------------------------------------------------------

    pub(crate) async fn fetch_balance(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_id(chain_id)?;
        let client = ChainClient::build(chain, self).await?;
        client.fetch_balance_json(&address).await
    }

    /// Typed end-to-end balance fetch used by the refresh engine. Returns a
    /// parsed `NativeBalanceSummary` directly — no JSON-string intermediate.
    ///
    /// For `chain_id == 0` extended-public-key cases we still go through the
    /// xpub balance JSON path — that one's deeply UTXO-aware and not worth
    /// retyping for the marginal saving.
    pub(crate) async fn fetch_native_balance_summary_auto(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        if chain_id == 0 && is_extended_public_key(&address) {
            // xpub path stays JSON-based; parse once and project into the
            // unified summary shape.
            let json = self.fetch_bitcoin_xpub_balance(address, 20, 20).await?;
            let value: serde_json::Value =
                serde_json::from_str(&json).map_err(SpectraBridgeError::from)?;
            let confirmed_sats = value["confirmed_sats"].as_u64().unwrap_or(0);
            let utxo_count = value["utxo_count"].as_u64().unwrap_or(0) as u32;
            return Ok(NativeBalanceSummary {
                smallest_unit: confirmed_sats.to_string(),
                amount_display: format_smallest_unit_decimal(confirmed_sats as u128, 8),
                utxo_count,
            });
        }
        let chain = chain_for_id(chain_id)?;
        let client = ChainClient::build(chain, self).await?;
        client.fetch_native_balance_summary(&address).await
    }

    pub(crate) async fn fetch_history(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_id(chain_id)?;
        let client = ChainClient::build(chain, self).await?;
        client.fetch_history_json(&address, self, chain).await
    }

    pub(crate) async fn fetch_bitcoin_xpub_balance(
        &self,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(0).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        let bal = crate::derivation::utxo_hd::fetch_xpub_balance(
            &client,
            &xpub,
            receive_count,
            change_count,
        )
        .await
        .map_err(SpectraBridgeError::from)?;
        Ok(serde_json::to_string(&bal)?)
    }

    pub(crate) async fn fetch_utxo_fee_preview(
        &self,
        chain_id: u32,
        address: String,
        fee_rate_svb: u64,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("fetch_utxo_fee_preview: unsupported chain_id: {chain_id}")))?;
        let eps = self.endpoints_for(chain.id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 {
                    fee_rate_svb
                } else {
                    client.fetch_fee_rate(3).await
                        .map(|r| r.sats_per_vbyte.ceil() as u64)
                        .unwrap_or(5)
                };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_koin).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { client.fetch_fee_rate(3).await };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { client.fetch_fee_rate(3).await };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            c => Err(SpectraBridgeError::from(format!(
                "fetch_utxo_fee_preview: unsupported chain: {c:?}"
            ))),
        }
    }

    pub(crate) async fn broadcast_raw(
        &self,
        chain_id: u32,
        payload: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("broadcast_raw: chain {chain_id} not supported")))?;
        let eps = self.endpoints_for(chain.id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let txid = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(json!({ "txid": txid }).to_string())
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Solana => {
                let client = SolanaClient::new(eps);
                let res = client
                    .broadcast_raw(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Tron => {
                let client = TronClient::new(eps);
                let res = client
                    .broadcast_raw(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(eps, c.evm_chain_id());
                let res = client
                    .broadcast_raw(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Xrp => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let blob = val["tx_blob_hex"].as_str()
                    .ok_or("broadcast_raw xrp: missing tx_blob_hex")?
                    .to_string();
                let client = XrpClient::new(eps);
                let res = client
                    .submit_signed_blob(&blob)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Stellar => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let xdr = val["signed_xdr_b64"].as_str()
                    .ok_or("broadcast_raw stellar: missing signed_xdr_b64")?
                    .to_string();
                let client = StellarClient::new(eps);
                let res = client
                    .submit_envelope_b64(&xdr)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Cardano => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let cbor = val["cbor_hex"].as_str()
                    .ok_or("broadcast_raw cardano: missing cbor_hex")?
                    .to_string();
                let api_key = self.api_key_for(chain.id()).await.unwrap_or_default();
                let client = CardanoClient::new(eps, api_key);
                let res = client
                    .submit_tx(&cbor)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Polkadot => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let ext_hex = val["extrinsic_hex"].as_str()
                    .ok_or("broadcast_raw polkadot: missing extrinsic_hex")?
                    .to_string();
                let subscan = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = PolkadotClient::new(eps, subscan, api_key);
                let res = client
                    .submit_extrinsic_hex(&ext_hex)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Sui => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_bytes = val["tx_bytes_b64"].as_str()
                    .ok_or("broadcast_raw sui: missing tx_bytes_b64")?
                    .to_string();
                let sig = val["sig_b64"].as_str()
                    .ok_or("broadcast_raw sui: missing sig_b64")?
                    .to_string();
                let client = SuiClient::new(eps);
                let res = client
                    .execute_signed_tx(&tx_bytes, &sig)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Aptos => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let body_json = val["signed_body_json"].as_str()
                    .ok_or("broadcast_raw aptos: missing signed_body_json")?
                    .to_string();
                let client = AptosClient::new(eps);
                let res = client
                    .submit_signed_body(&body_json)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Ton => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let boc = val["boc_b64"].as_str()
                    .ok_or("broadcast_raw ton: missing boc_b64")?
                    .to_string();
                let api_key = self.api_key_for(chain.id()).await;
                let client = TonClient::new(eps, api_key);
                let res = client
                    .send_boc(&boc)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Near => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_b64 = val["signed_tx_b64"].as_str()
                    .ok_or("broadcast_raw near: missing signed_tx_b64")?
                    .to_string();
                let client = NearClient::new(eps);
                let res = client
                    .broadcast_signed_tx_b64(&tx_b64)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Icp => Err(SpectraBridgeError::from(
                "ICP rebroadcast is not supported".to_string()
            )),
            c => Err(SpectraBridgeError::from(format!(
                "broadcast_raw: chain {c:?} not supported"
            ))),
        }
    }

    pub(crate) async fn fetch_evm_send_preview(
        &self,
        chain_id: u32,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_evm_send_preview: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());

        let value_u128: u128 = value_wei.parse().unwrap_or(0);
        let data_opt: Option<&str> = if data_hex == "0x" || data_hex.is_empty() {
            None
        } else {
            Some(&data_hex)
        };

        let (nonce_res, fee_res, gas_res, bal_res) = tokio::join!(
            client.fetch_nonce(&from),
            client.fetch_fee_estimate(),
            client.estimate_gas(&from, &to, value_u128, data_opt),
            client.fetch_balance(&from)
        );

        let nonce = nonce_res.unwrap_or(0);
        let fee = fee_res.unwrap_or(crate::fetch::chains::evm::EvmFeeEstimate {
            base_fee_wei: 0,
            priority_fee_wei: 1_000_000_000,
            max_fee_per_gas_wei: 2_000_000_000,
            estimated_fee_wei: 42_000_000_000,
        });
        let gas_limit = gas_res.unwrap_or(21_000);
        let balance_wei_val: u128 = bal_res
            .map(|b| b.balance_wei.parse::<u128>().unwrap_or(0))
            .unwrap_or(0);

        let estimated_fee_wei: u128 = (gas_limit as u128)
            .saturating_mul(fee.max_fee_per_gas_wei);
        let max_fee_gwei = fee.max_fee_per_gas_wei as f64 / 1_000_000_000.0;
        let priority_fee_gwei = fee.priority_fee_wei as f64 / 1_000_000_000.0;
        let balance_eth = balance_wei_val as f64 / 1e18;
        let estimated_fee_eth = estimated_fee_wei as f64 / 1e18;
        let spendable_eth = (balance_wei_val.saturating_sub(estimated_fee_wei)) as f64 / 1e18;

        Ok(json!({
            "nonce": nonce,
            "gas_limit": gas_limit,
            "max_fee_per_gas_gwei": max_fee_gwei,
            "max_priority_fee_per_gas_gwei": priority_fee_gwei,
            "estimated_fee_eth": estimated_fee_eth,
            "balance_eth": balance_eth,
            "spendable_eth": spendable_eth,
            "fee_rate_description": format!("Max {:.2} gwei / Priority {:.2} gwei",
                max_fee_gwei, priority_fee_gwei),
        }).to_string())
    }

    pub(crate) async fn fetch_tron_send_preview(
        &self,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for(7).await;
        let client = TronClient::new(eps);

        let trx_balance = client.fetch_balance(&address).await
            .map(|b| b.sun as f64 / 1_000_000.0)
            .unwrap_or(0.0);

        if symbol == "TRX" || contract_address.is_empty() {
            let fee_trx = 1.0_f64;
            let spendable = (trx_balance - fee_trx).max(0.0);
            return Ok(json!({
                "estimated_fee_trx": fee_trx,
                "fee_limit_sun": 0_i64,
                "spendable_balance": spendable,
                "max_sendable": spendable,
                "fee_rate_description": "Static bandwidth estimate",
            }).to_string());
        }

        let token_balance = client.fetch_trc20_balance_of(&contract_address, &address).await
            .map(|raw| raw as f64 / 1_000_000.0)
            .unwrap_or(0.0);

        let fee_trx = 15.0_f64;
        let fee_limit_sun: i64 = 15_000_000;
        Ok(json!({
            "estimated_fee_trx": fee_trx,
            "fee_limit_sun": fee_limit_sun,
            "spendable_balance": token_balance,
            "max_sendable": token_balance,
            "fee_rate_description": "Static energy estimate",
        }).to_string())
    }

    pub(crate) async fn fetch_simple_chain_send_preview(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let (fee_json, balance_json) = tokio::try_join!(
            self.fetch_fee_estimate(chain_id),
            self.fetch_balance(chain_id, address),
        )?;

        let fee_obj: serde_json::Value = serde_json::from_str(&fee_json)?;
        let bal_obj: serde_json::Value = serde_json::from_str(&balance_json)?;

        let fee_display = fee_obj["native_fee_display"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| fee_obj["native_fee_display"].as_f64())
            .unwrap_or(0.0);
        let fee_raw = fee_obj["native_fee_raw"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| fee_obj["native_fee_raw"].as_u64().map(|n| n.to_string()))
            .unwrap_or_default();
        let fee_rate_description = fee_obj["source"]
            .as_str()
            .unwrap_or("static")
            .to_string();

        let balance_display = simple_chain_balance_display(chain_id, &bal_obj);
        let max_sendable = (balance_display - fee_display).max(0.0);

        Ok(json!({
            "fee_display":          fee_display,
            "fee_raw":              fee_raw,
            "fee_rate_description": fee_rate_description,
            "balance_display":      balance_display,
            "max_sendable":         max_sendable,
        })
        .to_string())
    }

    fn build_execute_send_payload(
        &self,
        req: &crate::send::SendExecutionRequest,
        priv_hex: &str,
        pub_hex: &Option<String>,
    ) -> Result<serde_json::Value, SpectraBridgeError> {
        use crate::send::payload::*;
        use crate::send::preview_decode::build_utxo_sat_send_payload;

        let from = &req.from_address;
        let to = &req.to_address;
        let amount = req.amount;
        let priv_str = priv_hex.to_string();

        let chain = Chain::from_id(req.chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!("execute_send: unsupported chain_id: {}", req.chain_id))
        })?;

        // Each chain's `build_*_send_payload` still produces a JSON String
        // internally — we parse that to `Value` on exit so `sign_and_send`
        // doesn't have to re-parse on the receiving side.
        let json_string: String = if let Some(ref contract) = req.contract_address {
            let decimals = req.token_decimals.unwrap_or(6);
            match chain {
                c if c.is_evm() => {
                    let amount_raw = crate::send::preview_decode::amount_to_raw_units_string(amount, decimals);
                    let overrides = crate::send::ethereum::render_evm_overrides_fragment(
                        req.evm_overrides.as_ref(),
                    );
                    crate::send::ethereum::build_evm_token_send_payload(
                        from.clone(), contract.clone(), to.clone(),
                        amount_raw, priv_str, overrides,
                    )
                }
                Chain::Tron => build_tron_token_send_payload(
                    from.clone(), contract.clone(), to.clone(), amount, decimals, priv_str,
                ),
                Chain::Solana => build_solana_token_send_payload(
                    pub_hex.clone().unwrap_or_default(),
                    contract.clone(), to.clone(), amount, decimals, priv_str,
                ),
                Chain::Near => build_near_token_send_payload(
                    from.clone(), contract.clone(), to.clone(), amount, decimals,
                    priv_str, pub_hex.clone().unwrap_or_default(),
                ),
                c => return Err(SpectraBridgeError::from(format!(
                    "execute_send: unsupported token chain: {c:?}"
                ))),
            }
        } else {
            match chain {
            Chain::Bitcoin => build_btc_send_payload(
                from.clone(), to.clone(), amount,
                req.fee_rate_svb.unwrap_or(10.0), priv_str,
            ),
            c if c.is_evm() => {
                let value_wei = crate::send::preview_decode::amount_to_raw_units_string(amount, 18);
                let overrides = crate::send::ethereum::render_evm_overrides_fragment(
                    req.evm_overrides.as_ref(),
                );
                crate::send::ethereum::build_evm_native_send_payload(
                    from.clone(), to.clone(), value_wei, priv_str, overrides,
                )
            }
            Chain::Solana => build_solana_native_send_payload(
                pub_hex.clone().unwrap_or_default(), to.clone(), amount, priv_str,
            ),
            Chain::Dogecoin => build_doge_send_payload(
                from.clone(), to.clone(), amount,
                req.fee_rate_svb.unwrap_or(0.01), priv_str,
            ),
            Chain::Xrp => build_xrp_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone(),
            ),
            Chain::Litecoin | Chain::BitcoinCash => {
                let amount_sat = (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                let fee_sat = req.fee_sat.unwrap_or(if chain == Chain::Litecoin { 10_000 } else { 1_000 });
                build_utxo_sat_send_payload(from.clone(), to.clone(), amount_sat, fee_sat, priv_str)
            }
            Chain::Tron => build_tron_native_send_payload(from.clone(), to.clone(), amount, priv_str),
            Chain::Stellar => build_stellar_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone(),
            ),
            Chain::Cardano => build_cardano_send_payload(
                from.clone(), to.clone(), amount,
                req.fee_amount.unwrap_or(0.17), priv_str,
                pub_hex.clone().unwrap_or_default(),
            ),
            Chain::Polkadot => build_polkadot_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone().unwrap_or_default(),
            ),
            Chain::Bittensor => crate::send::payload::build_bittensor_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone().unwrap_or_default(),
            ),
            Chain::Sui => build_sui_send_payload(
                from.clone(), to.clone(), amount,
                req.gas_budget.unwrap_or(0.01), priv_str,
                pub_hex.clone().unwrap_or_default(),
            ),
            Chain::Aptos => build_aptos_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone().unwrap_or_default(),
            ),
            Chain::Ton => build_ton_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone().unwrap_or_default(),
            ),
            Chain::Near => build_near_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone().unwrap_or_default(),
            ),
            Chain::Icp => build_icp_send_payload(
                from.clone(), to.clone(), amount, priv_str,
                pub_hex.clone(),
            ),
            Chain::Monero => build_monero_send_payload(
                to.clone(), amount, req.monero_priority.unwrap_or(2),
            ),
            Chain::BitcoinSV => {
                let amount_sat = (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                let fee_sat = req.fee_sat.unwrap_or(1_000);
                build_utxo_sat_send_payload(from.clone(), to.clone(), amount_sat, fee_sat, priv_str)
            }
            Chain::Zcash => {
                let amount_sat = (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                let fee_sat = req.fee_sat.unwrap_or(1_000);
                build_utxo_sat_send_payload(from.clone(), to.clone(), amount_sat, fee_sat, priv_str)
            }
            Chain::BitcoinGold => {
                let amount_sat = (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                let fee_sat = req.fee_sat.unwrap_or(1_000);
                build_utxo_sat_send_payload(from.clone(), to.clone(), amount_sat, fee_sat, priv_str)
            }
            Chain::Decred => {
                let amount_atoms = (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                let fee_atoms = req.fee_sat.unwrap_or(2_000);
                build_utxo_sat_send_payload(from.clone(), to.clone(), amount_atoms, fee_atoms, priv_str)
            }
            Chain::Kaspa => {
                let amount_sompi = (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                let fee_sompi = req.fee_sat.unwrap_or(1_000);
                build_utxo_sat_send_payload(from.clone(), to.clone(), amount_sompi, fee_sompi, priv_str)
            }
            Chain::Dash => {
                let amount_sat = (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                let fee_sat = req.fee_sat.unwrap_or(2_000);
                build_utxo_sat_send_payload(from.clone(), to.clone(), amount_sat, fee_sat, priv_str)
            }
            c => return Err(SpectraBridgeError::from(format!(
                "execute_send: unsupported chain: {c:?}"
            ))),
            }
        };
        serde_json::from_str(&json_string).map_err(SpectraBridgeError::from)
    }

}



// ── Merged from service_helpers.rs ──────────────────────

// Internal helper functions used by the WalletService impl blocks.
// None of these are UniFFI-exported.
//
// ## Error message convention
//
// Errors raised here use the format `"<context>: <reason>"`, where the
// context names the offending field (`private_key_hex`, `planck`, `chain_id`)
// and the reason names the failure (`hex decode: …`, `wrong length: …`,
// `invalid params: …`). This puts the field name first so the most
// diagnostic information appears in any truncated log line. New helpers in
// this file should follow the same shape; downstream chain dispatch arms
// that still hand-format errors are migration candidates.

// (Chain, AssetHolding, json are already imported at the top of this file)
use serde::Serialize;

// ── Chain ID lookup ───────────────────────────────────────────────────────

/// `Chain::from_id` with a uniform error message. Used by every WalletService
/// method that takes a `chain_id: u32` from Swift.
///
/// Reader note: the call pattern `let chain = chain_for_id(chain_id)?;` is
/// the first line of ~30 dispatch methods in `service::mod`. That repetition
/// is a sign the receiver shape is wrong — a future refactor should accept
/// `Chain` directly via a typed UniFFI Record (or a thin newtype that does
/// the lookup once at FFI entry), eliminating the per-method conversion.
/// New methods should accept `Chain` as a parameter where possible rather
/// than `chain_id: u32`, with `chain_for_id` only at the FFI boundary.
pub(super) fn chain_for_id(chain_id: u32) -> Result<Chain, SpectraBridgeError> {
    Chain::from_id(chain_id)
        .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain_id: {chain_id}")))
}

/// Serialize a value to JSON, returning the bridge error type directly.
/// Used by chain dispatch arms whose FFI signature is
/// `Result<String, SpectraBridgeError>`. New endpoints should return a
/// typed `#[derive(uniffi::Record)]` value directly rather than going
/// through this helper.
pub(super) fn json_response<T: Serialize>(value: &T) -> Result<String, SpectraBridgeError> {
    serde_json::to_string(value).map_err(SpectraBridgeError::from)
}

// ── Param extraction ──────────────────────────────────────────────────────

pub(super) fn str_field<'a>(
    params: &'a serde_json::Value,
    key: &str,
) -> Result<&'a str, SpectraBridgeError> {
    params[key]
        .as_str()
        .ok_or_else(|| SpectraBridgeError::from(format!("missing field: {key}")))
}

pub(super) fn hex_field(
    params: &serde_json::Value,
    key: &str,
) -> Result<Vec<u8>, SpectraBridgeError> {
    let s = str_field(params, key)?;
    hex::decode(s).map_err(|e| SpectraBridgeError::from(format!("{key} hex decode: {e}")))
}

/// Parse `params` as a typed payload. Lets a chain dispatch arm collapse
/// six lines of ad-hoc `params["x"].as_y()` extraction into one decode step:
///
/// ```ignore
/// let p: PolkadotSendParams = parse_params(&params)?;
/// ```
///
/// Serde populates the error message with the offending field name, so each
/// missing-field error is "<chain>: missing field `planck` at line N column M"
/// instead of an inscrutable `"missing planck"`.
pub(super) fn parse_params<T: for<'de> serde::Deserialize<'de>>(
    params: &serde_json::Value,
) -> Result<T, SpectraBridgeError> {
    serde_json::from_value(params.clone())
        .map_err(|e| SpectraBridgeError::from(format!("invalid params: {e}")))
}

/// Decode a hex string of an exact byte length. Replaces the
/// `hex_field(..)?.try_into().map_err(|_| "X wrong length")?` pair that
/// recurred across every chain arm. Includes the field name in both the
/// "not hex" and "wrong length" error variants.
pub(super) fn decode_hex_array<const N: usize>(
    hex_str: &str,
    field_name: &str,
) -> Result<[u8; N], SpectraBridgeError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| SpectraBridgeError::from(format!("{field_name} hex decode: {e}")))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| {
            SpectraBridgeError::from(format!(
                "{field_name} wrong length: expected {N} bytes, got {}",
                v.len()
            ))
        })
}

// ── Decimal scaling ───────────────────────────────────────────────────────

/// Format a smallest-unit `u128` amount as a fixed-decimal string. Used for
/// chains whose typed balance struct doesn't already provide a `_display` field.
pub(super) fn format_smallest_unit_decimal(amount: u128, decimals: u32) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let scale = 10u128.pow(decimals);
    let whole = amount / scale;
    let frac = amount % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    if trimmed.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{trimmed}")
    }
}

/// Scale a raw integer by `10^decimals` into a human-readable decimal string
/// with up to 6 fractional digits of precision.
pub(super) fn format_decimals(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let divisor: u128 = 10u128.pow(decimals as u32);
    let whole = raw / divisor;
    let frac = raw % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

// ── Balance JSON parsing ──────────────────────────────────────────────────

/// Extract the normalised native balance (in display units) from a balance
/// JSON value. Returns 0.0 for unknown / unsupported chains.
pub(super) fn simple_chain_balance_display(chain_id: u32, obj: &serde_json::Value) -> f64 {
    let u64_field = |key: &str| -> f64 {
        obj[key]
            .as_u64()
            .map(|n| n as f64)
            .or_else(|| obj[key].as_str().and_then(|s| s.parse::<u64>().ok()).map(|n| n as f64))
            .unwrap_or(0.0)
    };
    let i64_field = |key: &str| -> f64 {
        obj[key]
            .as_i64()
            .map(|n| n as f64)
            .or_else(|| obj[key].as_str().and_then(|s| s.parse::<i64>().ok()).map(|n| n as f64))
            .unwrap_or(0.0)
    };
    let Some(chain) = Chain::from_id(chain_id) else {
        return 0.0;
    };
    let factor = 10f64.powi(chain.native_decimals() as i32);
    match chain {
        Chain::Stellar => i64_field("stroops") / factor,
        Chain::Polkadot => obj["planck"]
            .as_u64()
            .map(|n| n as f64)
            .or_else(|| obj["planck"].as_str().and_then(|s| s.parse::<f64>().ok()))
            .unwrap_or(0.0)
            / factor,
        Chain::Near => {
            if let Some(s) = obj["near_display"].as_str() {
                s.parse::<f64>().unwrap_or(0.0)
            } else {
                obj["yocto_near"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|y| y / factor)
                    .unwrap_or(0.0)
            }
        }
        Chain::Solana
        | Chain::Xrp
        | Chain::Cardano
        | Chain::Sui
        | Chain::Aptos
        | Chain::Ton
        | Chain::Icp
        | Chain::Monero => match chain.native_balance_field() {
            Some(field) => u64_field(field) / factor,
            None => 0.0,
        },
        _ => 0.0,
    }
}

// ── Fee preview JSON shapes ───────────────────────────────────────────────

/// Flat struct for direct serialization — avoids the intermediate
/// `serde_json::Value` + `Map` heap allocation that `json!()` would produce.
#[derive(Serialize)]
struct FeePreview<'a> {
    chain_id: u32,
    native_fee_raw: &'a str,
    native_fee_display: &'a str,
    unit: &'a str,
    source: &'a str,
}

/// Build a `fee_preview` JSON string from an integer raw amount plus decimals.
/// Scales the raw amount down for a human-readable display field.
pub(super) fn fee_preview(chain_id: u32, raw: u128, decimals: u8, unit: &str, source: &str) -> String {
    let display = format_decimals(raw, decimals);
    let raw_str = raw.to_string();
    serde_json::to_string(&FeePreview {
        chain_id,
        native_fee_raw: &raw_str,
        native_fee_display: &display,
        unit,
        source,
    })
    .unwrap()
}

/// Variant that accepts pre-computed raw/display strings. Used when the raw
/// amount doesn't fit in `u128` (e.g. NEAR's 10^21 yoctoNEAR).
pub(super) fn fee_preview_str(
    chain_id: u32,
    raw: &str,
    display: &str,
    unit: &str,
    source: &str,
) -> String {
    serde_json::to_string(&FeePreview {
        chain_id,
        native_fee_raw: raw,
        native_fee_display: display,
        unit,
        source,
    })
    .unwrap()
}

/// Compute a UTXO capacity fee preview using P2PKH sizing (148 B/input,
/// 34 B/output, 10 B overhead). Assumes all confirmed UTXOs above the
/// 546-satoshi dust threshold are selected, single-output (max-send) tx.
pub(super) fn utxo_fee_preview_json(utxo_values: Vec<u64>, fee_rate: u64) -> String {
    const INPUT_BYTES: u64 = 148;
    const OUTPUT_BYTES: u64 = 34;
    const OVERHEAD: u64 = 10;
    const DUST: u64 = 546;

    let spendable: Vec<u64> = utxo_values.into_iter().filter(|&v| v >= DUST).collect();
    let n = spendable.len() as u64;
    let total: u64 = spendable.iter().sum();

    if n == 0 || total == 0 {
        return json!({
            "fee_rate_svb": fee_rate,
            "estimated_fee_sat": 0_u64,
            "estimated_tx_bytes": 0_u64,
            "selected_input_count": 0_u64,
            "uses_change_output": false,
            "spendable_balance_sat": 0_u64,
            "max_sendable_sat": 0_u64,
        })
        .to_string();
    }

    let tx_bytes = OVERHEAD + n * INPUT_BYTES + OUTPUT_BYTES;
    let fee = tx_bytes * fee_rate;
    let max_sendable = total.saturating_sub(fee);

    json!({
        "fee_rate_svb": fee_rate,
        "estimated_fee_sat": fee,
        "estimated_tx_bytes": tx_bytes,
        "selected_input_count": n,
        "uses_change_output": false,
        "spendable_balance_sat": total,
        "max_sendable_sat": max_sendable,
    })
    .to_string()
}

// ── EVM overrides parser ──────────────────────────────────────────────────

/// Parse optional EVM transaction overrides from a `sign_and_send` params blob.
/// All fields default to `None` — the Rust client then falls back to its
/// standard pending-nonce / recommended-fee / estimated-gas behavior.
pub(super) fn read_evm_overrides(
    params: &serde_json::Value,
) -> crate::send::chains::evm::EvmSendOverrides {
    let nonce = params["nonce"]
        .as_u64()
        .or_else(|| params["nonce"].as_str().and_then(|s| s.parse().ok()));
    let gas_limit = params["gas_limit"]
        .as_u64()
        .or_else(|| params["gas_limit"].as_str().and_then(|s| s.parse().ok()));
    let max_fee_per_gas_wei = params["max_fee_per_gas_wei"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| params["max_fee_per_gas_wei"].as_u64().map(|n| n as u128));
    let max_priority_fee_per_gas_wei = params["max_priority_fee_per_gas_wei"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| params["max_priority_fee_per_gas_wei"].as_u64().map(|n| n as u128));
    crate::send::chains::evm::EvmSendOverrides {
        nonce,
        max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei,
        gas_limit,
    }
}

// ── SQLite blocking helpers ───────────────────────────────────────────────
//
// Key/value `state` table backing AppState persistence (wallets, settings,
// fiat rates, live prices, etc.). Mirrors the `with_conn` pool already in
// `store/wallet_db.rs` — re-uses a single `Connection` per `db_path` instead
// of opening + running DDL + closing on every load/save. With ~5–10 persists
// per refresh cycle, the previous open-per-call cost was meaningful.
//
// PRAGMAs applied once per connection:
//   - `journal_mode = WAL`     concurrent reads while a write is in flight
//   - `synchronous  = NORMAL`  fsync only at checkpoint, ~5× faster writes
//                              (still durable; only loses ms on power loss)
//   - `temp_store   = MEMORY`  query temp tables don't hit disk

use parking_lot::Mutex as PlMutex;
use std::collections::HashMap;

static SQLITE_POOL: std::sync::LazyLock<PlMutex<HashMap<String, rusqlite::Connection>>> =
    std::sync::LazyLock::new(|| PlMutex::new(HashMap::new()));

fn with_state_conn<T>(
    db_path: &str,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    let mut pool = SQLITE_POOL.lock();
    if !pool.contains_key(db_path) {
        pool.insert(db_path.to_string(), open_state_conn(db_path)?);
    }
    f(pool.get(db_path).unwrap())
}

fn open_state_conn(db_path: &str) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| format!("sqlite open {db_path}: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         CREATE TABLE IF NOT EXISTS state (
             key      TEXT    PRIMARY KEY,
             value    TEXT    NOT NULL,
             saved_at INTEGER NOT NULL
         );",
    )
    .map_err(|e| format!("sqlite init: {e}"))?;
    Ok(conn)
}

pub(super) fn sqlite_load(db_path: &str, key: &str) -> Result<String, String> {
    with_state_conn(db_path, |conn| {
        let result: rusqlite::Result<String> = conn.query_row(
            "SELECT value FROM state WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        );
        match result {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok("{}".to_string()),
            Err(e) => Err(format!("sqlite load: {e}")),
        }
    })
}

pub(super) fn sqlite_save(db_path: &str, key: &str, value: &str) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    with_state_conn(db_path, |conn| {
        conn.execute(
            "INSERT INTO state (key, value, saved_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, saved_at = excluded.saved_at",
            rusqlite::params![key, value, now],
        )
        .map_err(|e| format!("sqlite save: {e}"))?;
        Ok(())
    })
}

// ── State helpers ─────────────────────────────────────────────────────────

/// Return a zero-amount AssetHolding template for the native coin of each
/// chain. Used as the default when the holding doesn't exist yet.
pub(crate) fn native_coin_template(chain_id: u32) -> Option<AssetHolding> {
    let chain = Chain::from_id(chain_id)?;
    Some(AssetHolding {
        name: chain.coin_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        coin_gecko_id: chain.coin_gecko_id().to_string(),
        chain_name: chain.chain_display_name().to_string(),
        token_standard: "Native".to_string(),
        contract_address: None,
        amount: 0.0,
        price_usd: 0.0,
    })
}


/// Returns `true` when `s` starts with a BIP-32 extended public key prefix.
pub(super) fn is_extended_public_key(s: &str) -> bool {
    matches!(
        s.get(..4),
        Some("xpub") | Some("ypub") | Some("zpub") | Some("Ypub") | Some("Zpub")
    )
}


// ── FFI: misc utility surface (relocated from ffi.rs) ────────────────────

/// Trim + lowercase + strip leading `0x` from a private-key hex string.
#[uniffi::export]
pub fn core_private_key_hex_normalized(raw_value: String) -> String {
    let trimmed = raw_value.trim().to_lowercase();
    match trimmed.strip_prefix("0x") {
        Some(stripped) => stripped.to_string(),
        None => trimmed,
    }
}

/// Heuristic check for a 32-byte hex private key.
#[uniffi::export]
pub fn core_private_key_hex_is_likely(raw_value: String) -> bool {
    let normalized = core_private_key_hex_normalized(raw_value);
    normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct LargeMovementEvaluation {
    pub should_alert: bool,
    pub absolute_delta: f64,
    pub ratio: f64,
    pub direction_up: bool,
}

/// Evaluate whether a portfolio-total swing crosses both an absolute USD
/// threshold and a percent-change threshold (large-movement notifications).
#[uniffi::export]
pub fn core_evaluate_large_movement(
    previous_total_usd: f64,
    current_total_usd: f64,
    usd_threshold: f64,
    percent_threshold: f64,
) -> LargeMovementEvaluation {
    if previous_total_usd <= 0.0 {
        return LargeMovementEvaluation {
            should_alert: false,
            absolute_delta: 0.0,
            ratio: 0.0,
            direction_up: true,
        };
    }
    let delta = current_total_usd - previous_total_usd;
    let absolute_delta = delta.abs();
    let ratio = absolute_delta / previous_total_usd;
    let should_alert =
        absolute_delta >= usd_threshold && ratio >= (percent_threshold / 100.0);
    LargeMovementEvaluation {
        should_alert,
        absolute_delta,
        ratio,
        direction_up: delta >= 0.0,
    }
}

// ── Merged from service_types.rs ──────────────────────

use serde::Deserialize;

/// Typed app-settings record for UniFFI — Rust handles JSON serialization to/from SQLite.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAppSettings {
    pub pricing_provider: String,
    pub selected_fiat_currency: String,
    pub fiat_rate_provider: String,
    #[serde(rename = "ethereumRPCEndpoint")]
    pub ethereum_rpc_endpoint: String,
    pub ethereum_network_mode: String,
    #[serde(rename = "etherscanAPIKey")]
    pub etherscan_api_key: String,
    #[serde(rename = "moneroBackendBaseURL")]
    pub monero_backend_base_url: String,
    #[serde(rename = "moneroBackendAPIKey")]
    pub monero_backend_api_key: String,
    pub bitcoin_network_mode: String,
    pub dogecoin_network_mode: String,
    pub bitcoin_esplora_endpoints: String,
    pub bitcoin_stop_gap: i32,
    pub bitcoin_fee_priority: String,
    pub dogecoin_fee_priority: String,
    pub hide_balances: bool,
    #[serde(rename = "useFaceID")]
    pub use_face_id: bool,
    pub use_auto_lock: bool,
    #[serde(rename = "useStrictRPCOnly")]
    pub use_strict_rpc_only: bool,
    pub require_biometric_for_send_actions: bool,
    pub use_price_alerts: bool,
    pub use_transaction_status_notifications: bool,
    pub use_large_movement_notifications: bool,
    pub automatic_refresh_frequency_minutes: i32,
    pub background_sync_profile: String,
    pub large_movement_alert_percent_threshold: f64,
    #[serde(rename = "largeMovementAlertUSDThreshold")]
    pub large_movement_alert_usd_threshold: f64,
    pub pinned_dashboard_asset_symbols: Vec<String>,
}

/// Token descriptor passed across UniFFI without JSON-shuttle marshalling.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenDescriptor {
    pub contract: String,
    pub symbol: String,
    pub decimals: u8,
    pub name: Option<String>,
}

/// Typed token-balance result returned via UniFFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenBalanceResult {
    pub contract_address: String,
    pub symbol: String,
    pub decimals: u8,
    pub balance_raw: String,
    pub balance_display: String,
}

/// Unified per-chain native balance projection used by `fetch_native_balance_summary`.
/// `smallest_unit` is a base-10 integer string (sats, lamports, wei, yocto-NEAR, …);
/// `amount_display` is the chain's human-readable native amount.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NativeBalanceSummary {
    pub smallest_unit: String,
    pub amount_display: String,
    pub utxo_count: u32,
}

/// EVM-address probe output. Used by chain-risk warnings to decide whether a
/// destination looks "fresh" (zero balance + zero nonce).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmAddressProbe {
    pub nonce: i64,
    pub balance_eth: f64,
}

/// Endpoint configuration passed in from Swift at construction time and
/// rebuilt via `update_endpoints_typed`.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct ChainEndpoints {
    pub chain_id: u32,
    pub endpoints: Vec<String>,
    /// Optional API key for services that require one (Blockfrost, Subscan, etc.).
    pub api_key: Option<String>,
}

// ── Per-chain `sign_and_send` parameter shapes ────────────────────────────
//
// Each chain's `sign_and_send` arm in `service::mod` historically read its
// inputs by pulling individual fields out of a `serde_json::Value` with
// inline `.as_str()` / `.as_u64()` / `try_into()` chains. That style hides
// the contract — a reader can't see at a glance what shape the Polkadot
// endpoint expects without scanning the full arm body.
//
// Defining a typed struct per chain reverses that: the type doc *is* the
// API contract, serde gives field-name-aware error messages for free, and
// the dispatch arm collapses to one `parse_params` call.
//
// These structs accept the same JSON shape Swift already produces so this
// migration is internal — no FFI signature changes.

/// `Chain::Polkadot` send parameters. `planck` is the smallest unit
/// (10⁻¹⁰ DOT). The 32-byte `private_key_hex` is the sr25519 mini-secret
/// produced by `derive_polkadot`, *not* a 64-byte ed25519 secret.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PolkadotSendParams {
    pub from: String,
    pub to: String,
    /// Accepts either a JSON string ("12500000000") or a JSON number for
    /// backward compatibility with Swift call sites that emitted both forms.
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub planck: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

/// `Chain::Bittensor` send parameters. `rao` is the smallest unit
/// (10⁻⁹ TAO). Same sr25519 32-byte mini-secret rules as Polkadot.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BittensorSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub rao: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

/// Accepts JSON `"12345"` or `12345` for u128 fields. Swift sends planck
/// values as strings (since u128 doesn't round-trip safely through JSON
/// numbers) but legacy call sites emitted them as `as_u64`-able numbers.
fn deserialize_u128_from_string_or_number<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(s) = value.as_str() {
        return s.parse::<u128>().map_err(D::Error::custom);
    }
    if let Some(n) = value.as_u64() {
        return Ok(n as u128);
    }
    Err(D::Error::custom("expected u128 as string or number"))
}

// ── Merged from service_standalone.rs ──────────────────────

// Synchronous, stateless UniFFI exports — token catalog + BIP-39 mnemonic
// utilities. Kept separate from the WalletService impl blocks because they
// perform no network I/O and don't need the service's state.

use crate::tokens;

/// Return the built-in token catalog filtered to one chain. Synchronous
/// so Swift can call from a `static let`. For "all chains, please" use
/// [`list_all_builtin_tokens`] — that's the named entry point, not a
/// sentinel value.
#[uniffi::export]
pub fn list_builtin_tokens(chain_id: u32) -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(chain_id)
}

/// Return the entire built-in token catalog across every registered
/// chain. Replaces the `list_builtin_tokens(chain_id: u32::MAX)`
/// sentinel pattern — the "all chains" call site now reads as exactly
/// what it means instead of forcing the reader to know the magic value.
#[uniffi::export]
pub fn list_all_builtin_tokens() -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(u32::MAX)
}

/// Generate a new random BIP-39 mnemonic with the requested word count.
///
/// `word_count` must be 12, 15, 18, 21, or 24. Any other value falls back
/// silently to 12 words. Returns the space-joined mnemonic phrase.
#[uniffi::export]
pub fn generate_mnemonic(word_count: u32) -> String {
    use bip39::{Language, Mnemonic};
    use rand::RngCore;

    // BIP-39 entropy bytes: 128/160/192/224/256 bits → 12/15/18/21/24 words.
    let entropy_bytes: usize = match word_count {
        15 => 20,
        18 => 24,
        21 => 28,
        24 => 32,
        _ => 16, // default: 12 words
    };
    let mut entropy = vec![0u8; entropy_bytes];
    rand::thread_rng().fill_bytes(&mut entropy);
    Mnemonic::from_entropy_in(Language::English, &entropy)
        .expect("valid entropy length")
        .to_string()
}

/// Validate a BIP-39 mnemonic phrase. Returns `true` only for a valid
/// English BIP-39 mnemonic with correct word count + checksum.
#[uniffi::export]
pub fn validate_mnemonic(phrase: String) -> bool {
    use bip39::{Language, Mnemonic};
    phrase.trim().parse::<Mnemonic>().is_ok()
        || Mnemonic::parse_in(Language::English, phrase.trim()).is_ok()
}

/// Return the full BIP-39 English word list as a newline-delimited string
/// (2048 words, alphabetically sorted).
#[uniffi::export]
pub fn bip39_english_wordlist() -> String {
    static WORDLIST: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| bip39::Language::English.word_list().join("\n"));
    WORDLIST.clone()
}

// ── Merged from chain_client.rs ──────────────────────

// Per-chain client dispatch — collapses the 17-arm `match chain` blocks that
// were repeated across `fetch_balance`, `fetch_history`, and
// `fetch_native_balance_summary` into one place.
//
// Construction lives in `ChainClient::build`. Each method on the enum
// dispatches via `match self`. New chains add one variant + one arm per
// method, instead of editing 3+ separate match tables in `service/mod.rs`.

// (chain_client imports merged with the top-of-file block)

/// One constructed client per chain family. Variants that need extra
/// per-instance context beyond the client itself (Tron's tronscan URL,
/// Near's indexer URL) carry it inline.
pub(super) enum ChainClient {
    Bitcoin(BitcoinClient),
    BitcoinCash(BitcoinCashClient),
    BitcoinSv(BitcoinSvClient),
    Litecoin(LitecoinClient),
    Dogecoin(DogecoinClient),
    Evm(EvmClient),
    Solana(SolanaClient),
    Tron { client: TronClient, tronscan: String },
    Stellar(StellarClient),
    Xrp(XrpClient),
    Cardano(CardanoClient),
    Polkadot(PolkadotClient),
    Sui(SuiClient),
    Aptos(AptosClient),
    Ton(TonClient),
    Near { client: NearClient, indexer: String },
    Icp(IcpClient),
    Monero(MoneroClient),
    Zcash(ZcashClient),
    BitcoinGold(BitcoinGoldClient),
    Decred(DecredClient),
    Kaspa(KaspaClient),
    Dash(DashClient),
    Bittensor(BittensorClient),
}

impl ChainClient {
    /// Build the right client for `chain`, looking up endpoints / api keys /
    /// secondary endpoints from the service. The single source of truth for
    /// per-chain construction.
    pub(super) async fn build(
        chain: Chain,
        service: &WalletService,
    ) -> Result<Self, SpectraBridgeError> {
        // Each chain (mainnet OR testnet) has its own endpoint slot keyed
        // by its frozen `Chain::id()`, so users can configure independent
        // RPC URLs. EVM testnets share the EvmClient — its chainid is read
        // from `chain.evm_chain_id()` which already returns testnet ids
        // (Sepolia=11155111, Hoodi=560048, etc.).
        let endpoints = service.endpoints_for(chain.id()).await;
        // For non-EVM testnets we dispatch on `mainnet_counterpart()` so a
        // single set of per-chain client types covers both flavors. The
        // Bitcoin / Litecoin / Cash / SV / Dogecoin clients don't carry
        // network state internally — the network parameter is per-call.
        let dispatch = chain.mainnet_counterpart();
        Ok(match dispatch {
            Chain::Bitcoin => ChainClient::Bitcoin(BitcoinClient::new(HttpClient::shared(), endpoints)),
            Chain::BitcoinCash => ChainClient::BitcoinCash(BitcoinCashClient::new(endpoints)),
            Chain::BitcoinSV => ChainClient::BitcoinSv(BitcoinSvClient::new(endpoints)),
            Chain::Litecoin => ChainClient::Litecoin(LitecoinClient::new(endpoints)),
            Chain::Dogecoin => ChainClient::Dogecoin(DogecoinClient::new(endpoints)),
            c if c.is_evm() => ChainClient::Evm(EvmClient::new(endpoints, chain.evm_chain_id())),
            Chain::Solana => ChainClient::Solana(SolanaClient::new(endpoints)),
            Chain::Tron => {
                // Use the testnet/mainnet's own explorer slot so testnet
                // calls don't accidentally hit the mainnet Tronscan.
                let tronscan = service
                    .endpoints_for(chain.endpoint_id(EndpointSlot::Explorer))
                    .await
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "https://apilist.tronscan.org".to_string());
                ChainClient::Tron { client: TronClient::new(endpoints), tronscan }
            }
            Chain::Stellar => ChainClient::Stellar(StellarClient::new(endpoints)),
            Chain::Xrp => ChainClient::Xrp(XrpClient::new(endpoints)),
            Chain::Cardano => {
                let api_key = service.api_key_for(chain.id()).await.unwrap_or_default();
                ChainClient::Cardano(CardanoClient::new(endpoints, api_key))
            }
            Chain::Polkadot => {
                let subscan = service
                    .endpoints_for(chain.endpoint_id(EndpointSlot::Secondary))
                    .await;
                let api_key = service.api_key_for(chain.id()).await;
                ChainClient::Polkadot(PolkadotClient::new(endpoints, subscan, api_key))
            }
            Chain::Sui => ChainClient::Sui(SuiClient::new(endpoints)),
            Chain::Aptos => ChainClient::Aptos(AptosClient::new(endpoints)),
            Chain::Ton => {
                let api_key = service.api_key_for(chain.id()).await;
                ChainClient::Ton(TonClient::new(endpoints, api_key))
            }
            Chain::Near => {
                let indexer = service
                    .endpoints_for(chain.endpoint_id(EndpointSlot::Explorer))
                    .await
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "https://api.kitwallet.app".to_string());
                ChainClient::Near { client: NearClient::new(endpoints), indexer }
            }
            Chain::Icp => ChainClient::Icp(IcpClient::new(endpoints)),
            Chain::Monero => ChainClient::Monero(MoneroClient::new(endpoints)),
            Chain::Zcash => ChainClient::Zcash(ZcashClient::new(endpoints)),
            Chain::BitcoinGold => ChainClient::BitcoinGold(BitcoinGoldClient::new(endpoints)),
            Chain::Decred => ChainClient::Decred(DecredClient::new(endpoints)),
            Chain::Kaspa => ChainClient::Kaspa(KaspaClient::new(endpoints)),
            Chain::Dash => ChainClient::Dash(DashClient::new(endpoints)),
            Chain::Bittensor => {
                let taostats = service
                    .endpoints_for(chain.endpoint_id(EndpointSlot::Secondary))
                    .await;
                let api_key = service.api_key_for(chain.id()).await;
                ChainClient::Bittensor(BittensorClient::new(endpoints, taostats, api_key))
            }
            c => {
                return Err(SpectraBridgeError::from(format!("unsupported chain: {c:?}")))
            }
        })
    }

    /// Fetch native balance and serialize to a JSON string. Used by the
    /// generic `WalletService::fetch_balance` FFI export.
    pub(super) async fn fetch_balance_json(&self, address: &str) -> Result<String, SpectraBridgeError> {
        match self {
            ChainClient::Bitcoin(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::BitcoinCash(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::BitcoinSv(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Litecoin(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Dogecoin(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Evm(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Solana(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Tron { client, .. } => json_response(&client.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Stellar(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Xrp(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Cardano(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Polkadot(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Sui(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Aptos(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Ton(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Near { client, .. } => json_response(&client.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Icp(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Monero(c) => json_response(&c.fetch_balance(0).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Zcash(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::BitcoinGold(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Decred(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Kaspa(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Dash(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Bittensor(c) => json_response(&c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?),
        }
    }

    /// Fetch native balance and project into the unified `NativeBalanceSummary`.
    pub(super) async fn fetch_native_balance_summary(
        &self,
        address: &str,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        match self {
            ChainClient::Bitcoin(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.confirmed_sats.to_string(),
                    amount_display: format_smallest_unit_decimal(bal.confirmed_sats as u128, 8),
                    utxo_count: bal.utxo_count as u32,
                })
            }
            ChainClient::BitcoinCash(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_sat.to_string(), bal.balance_display))
            }
            ChainClient::BitcoinSv(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_sat.to_string(), bal.balance_display))
            }
            ChainClient::Litecoin(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_sat.to_string(), bal.balance_display))
            }
            ChainClient::Dogecoin(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_koin.to_string(), bal.balance_display))
            }
            ChainClient::Evm(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_wei, bal.balance_display))
            }
            ChainClient::Solana(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.lamports.to_string(), bal.sol_display))
            }
            ChainClient::Tron { client, .. } => {
                let bal = client.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.sun.to_string(), bal.trx_display))
            }
            ChainClient::Stellar(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.stroops.to_string(), bal.xlm_display))
            }
            ChainClient::Xrp(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.drops.to_string(), bal.xrp_display))
            }
            ChainClient::Cardano(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.lovelace.to_string(), bal.ada_display))
            }
            ChainClient::Polkadot(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.planck.to_string(), bal.dot_display))
            }
            ChainClient::Sui(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.mist.to_string(), bal.sui_display))
            }
            ChainClient::Aptos(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.octas.to_string(), bal.apt_display))
            }
            ChainClient::Ton(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.nanotons.to_string(), bal.ton_display))
            }
            ChainClient::Near { client, .. } => {
                let bal = client.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.yocto_near, bal.near_display))
            }
            ChainClient::Icp(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.e8s.to_string(), bal.icp_display))
            }
            ChainClient::Monero(c) => {
                let bal = c.fetch_balance(0).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.piconeros.to_string(), bal.xmr_display))
            }
            ChainClient::Zcash(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_sat.to_string(), bal.balance_display))
            }
            ChainClient::BitcoinGold(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_sat.to_string(), bal.balance_display))
            }
            ChainClient::Decred(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_atoms.to_string(), bal.balance_display))
            }
            ChainClient::Kaspa(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_sompi.to_string(), bal.balance_display))
            }
            ChainClient::Dash(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.balance_sat.to_string(), bal.balance_display))
            }
            ChainClient::Bittensor(c) => {
                let bal = c.fetch_balance(address).await.map_err(SpectraBridgeError::from)?;
                Ok(summary_native(bal.rao.to_string(), bal.tao_display))
            }
        }
    }

    /// Fetch transaction history for `address` and return JSON. The few chains
    /// that take extra context (Tron tronscan, Near indexer, EVM Etherscan)
    /// pull it from the variant or from `service`.
    pub(super) async fn fetch_history_json(
        &self,
        address: &str,
        service: &WalletService,
        chain: Chain,
    ) -> Result<String, SpectraBridgeError> {
        match self {
            ChainClient::Bitcoin(c) => json_response(&c.fetch_history(address, None).await.map_err(SpectraBridgeError::from)?),
            ChainClient::BitcoinCash(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::BitcoinSv(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Litecoin(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Dogecoin(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Evm(c) => {
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
                let h = c
                    .fetch_history(address, "https://api.etherscan.io", api_key_str, chain.evm_chain_id())
                    .await
                    .map_err(SpectraBridgeError::from)?;
                json_response(&h)
            }
            ChainClient::Solana(c) => json_response(&c.fetch_unified_history(address, 50).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Tron { client, tronscan } => json_response(&client.fetch_unified_history(address, tronscan, 50).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Stellar(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Xrp(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Cardano(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Polkadot(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Sui(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Aptos(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Ton(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Near { client, indexer } => json_response(&client.fetch_history(address, indexer).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Icp(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Monero(c) => json_response(&c.fetch_history(0).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Zcash(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::BitcoinGold(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Decred(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Kaspa(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Dash(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
            ChainClient::Bittensor(c) => json_response(&c.fetch_history(address).await.map_err(SpectraBridgeError::from)?),
        }
    }
}

fn summary_native(smallest_unit: String, amount_display: String) -> NativeBalanceSummary {
    NativeBalanceSummary {
        smallest_unit,
        amount_display,
        utxo_count: 0,
    }
}


// ── Merged from service_derivation_methods.rs ──────────────────────

// WalletService — Bitcoin xpub / HD address derivation.
//
// Sliced out of `service/mod.rs`. The `WalletService` type itself stays in
// `mod.rs`; methods here live in a separate `impl` block (Rust permits
// multiple impl blocks per type, and UniFFI exports them as if they were one).

// (inner allow stripped during merge)

// (self-import removed during merge)

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
        crate::derivation::utxo_hd::derive_account_xpub(
            &mnemonic_phrase,
            &passphrase,
            &account_path,
        )
        .map_err(SpectraBridgeError::from)
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
            crate::derivation::utxo_hd::derive_children(&xpub, change, start_index, count)
                .map_err(SpectraBridgeError::from)?;
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
        let endpoints = self.endpoints_for(0).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        let next = crate::derivation::utxo_hd::fetch_next_unused_address(
            &client,
            &xpub,
            change,
            gap_limit,
        )
        .await
        .map_err(SpectraBridgeError::from)?;
        Ok(next.map(|c| c.address))
    }
}

// ── Merged from service_pricing_methods.rs ──────────────────────

// WalletService — live price + fiat-rate fetch.
//
// Sliced out of `service/mod.rs`. The `WalletService` type itself stays in
// `mod.rs`; methods here live in a separate `impl` block (Rust permits
// multiple impl blocks per type, and UniFFI exports them as if they were one).

// (inner allow stripped during merge)

// (self-import removed during merge)

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
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
        eprintln!("[spectra:prices] enter provider={provider} coins={}", coins.len());
        let parsed_provider = match crate::price::PriceProvider::from_raw(&provider) {
            Some(p) => p,
            None => {
                eprintln!("[spectra:prices] UNKNOWN provider={provider}");
                return Err(format!("unknown price provider: {provider}").into());
            }
        };
        match crate::price::fetch_prices(parsed_provider, &coins).await {
            Ok(quotes) => {
                eprintln!("[spectra:prices] ok provider={provider} returned={}", quotes.len());
                Ok(quotes)
            }
            Err(e) => {
                eprintln!("[spectra:prices] FAIL provider={provider}: {e}");
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
        eprintln!("[spectra:fiat] enter provider={provider} currencies={}", currencies.len());
        let parsed_provider = match crate::price::FiatRateProvider::from_raw(&provider) {
            Some(p) => p,
            None => {
                eprintln!("[spectra:fiat] UNKNOWN provider={provider}");
                return Err(format!("unknown fiat rate provider: {provider}").into());
            }
        };
        match crate::price::fetch_fiat_rates(parsed_provider, &currencies).await {
            Ok(rates) => {
                eprintln!("[spectra:fiat] ok provider={provider} returned={}", rates.len());
                Ok(rates)
            }
            Err(e) => {
                eprintln!("[spectra:fiat] FAIL provider={provider}: {e}");
                Err(SpectraBridgeError::from(e))
            }
        }
    }
}

// ── Merged from service_state_methods.rs ──────────────────────

// WalletService — state persistence (SQLite-backed load/save/delete).
//
// Sliced out of `service/mod.rs`. The `WalletService` type itself stays in
// `mod.rs`; methods here live in a separate `impl` block (Rust permits
// multiple impl blocks per type, and UniFFI exports them as if they were one).

// (inner allow stripped during merge)

// (self-import removed during merge)

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Load the JSON state blob stored under `key` in the SQLite database at
    /// `db_path`. Returns an empty JSON object `"{}"` when no value has been
    /// saved yet. Thread-safe: rusqlite is called in `spawn_blocking`.
    pub async fn load_state(
        &self,
        db_path: String,
        key: String,
    ) -> Result<String, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            sqlite_load(&db_path, &key)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Persist the JSON state blob under `key` in the SQLite database at
    /// `db_path`. Creates the file (and the `state` table) on first use.
    pub async fn save_state(
        &self,
        db_path: String,
        key: String,
        state_json: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            sqlite_save(&db_path, &key, &state_json)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn save_app_settings_typed(
        &self,
        db_path: String,
        settings: PersistedAppSettings,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let json = serde_json::to_string(&settings)
                .map_err(|e| format!("save_app_settings_typed serialize: {e}"))?;
            sqlite_save(&db_path, "app.settings.v1", &json)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn load_app_settings_typed(
        &self,
        db_path: String,
    ) -> Result<Option<PersistedAppSettings>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let json = sqlite_load(&db_path, "app.settings.v1")?;
            if json == "{}" {
                return Ok(None);
            }
            serde_json::from_str::<PersistedAppSettings>(&json)
                .map(Some)
                .map_err(|e| format!("load_app_settings_typed deserialize: {e}"))
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Persist keypool state using typed record (no JSON intermediate).
    pub async fn save_keypool_state_typed(
        &self,
        db_path: String,
        wallet_id: String,
        chain_name: String,
        state: crate::wallet_db::KeypoolState,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_save(&db_path, &wallet_id, &chain_name, &state)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn load_all_keypool_state_typed(
        &self,
        db_path: String,
    ) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, crate::wallet_db::KeypoolState>>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::keypool_load_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Remove all keypool state for a wallet (called when a wallet is deleted).
    pub async fn delete_keypool_for_wallet(
        &self,
        db_path: String,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_delete_for_wallet(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Remove all keypool state for a chain (called when the user switches network modes,
    /// triggering a rescan).
    pub async fn delete_keypool_for_chain(
        &self,
        db_path: String,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_delete_for_chain(&db_path, &chain_name)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Upsert a single owned address record.
    pub async fn save_owned_address_typed(
        &self,
        db_path: String,
        record: crate::wallet_db::OwnedAddressRecord,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::address_save(&db_path, &record)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn load_all_owned_addresses_typed(
        &self,
        db_path: String,
    ) -> Result<Vec<crate::wallet_db::OwnedAddressRecord>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::address_load_all_chains(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Remove all owned address records for a deleted wallet.
    pub async fn delete_owned_addresses_for_wallet(
        &self,
        db_path: String,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::address_delete_for_wallet(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Remove all owned address records for a chain (called after a full rescan).
    pub async fn delete_owned_addresses_for_chain(
        &self,
        db_path: String,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::address_delete_for_chain(&db_path, &chain_name)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Remove all relational wallet state (keypool + addresses) for a deleted wallet.
    /// This is the single call to make when a wallet is removed.
    pub async fn delete_wallet_relational_data(
        &self,
        db_path: String,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::delete_wallet_data(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Upsert a batch of transaction history records. `records[*].payload`
    /// is the typed `CorePersistedTransactionRecord`; Rust serializes to JSON
    /// for the SQLite TEXT column internally — no JSON crosses the FFI.
    pub async fn upsert_history_records(
        &self,
        db_path: String,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_upsert_batch(&db_path, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn fetch_all_history_records_typed(
        &self,
        db_path: String,
    ) -> Result<Vec<crate::wallet_db::HistoryRecord>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Delete history records by ID.
    pub async fn delete_history_records(
        &self,
        db_path: String,
        ids: Vec<String>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::history_delete(&db_path, &ids))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Atomically replace ALL history records with the provided batch.
    pub async fn replace_all_history_records(
        &self,
        db_path: String,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_replace_all(&db_path, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Delete all history records (hard reset).
    pub async fn clear_all_history_records(
        &self,
        db_path: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_clear(&db_path)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Seed the in-memory wallet list from typed `WalletSummary` records.
    pub async fn init_wallet_state_direct(
        &self,
        wallets: Vec<WalletSummary>,
    ) -> Result<(), SpectraBridgeError> {
        let mut state = self.wallet_state.write().await;
        state.wallets = wallets;
        Ok(())
    }

    /// Add or replace a wallet from a typed `WalletSummary` record.
    pub async fn upsert_wallet_direct(
        &self,
        wallet: WalletSummary,
    ) -> Result<(), SpectraBridgeError> {
        let mut state = self.wallet_state.write().await;
        reduce_state_in_place(&mut state, StateCommand::UpsertWallet { wallet });
        Ok(())
    }

    /// Current cursor for the next history fetch, or `None` if no fetch has
    /// been done yet. Pass the returned value as the starting point for the
    /// next page request.
    pub fn history_next_cursor(&self, chain_id: u32, wallet_id: String) -> Option<String> {
        self.history_pagination.cursor(chain_id, &wallet_id)
    }

    /// Current zero-based page index for page-numbered chains (EVM, etc.).
    pub fn history_next_page(&self, chain_id: u32, wallet_id: String) -> u32 {
        self.history_pagination.page(chain_id, &wallet_id)
    }

    /// Returns `true` when all history pages have been fetched and no more
    /// pages are available. Swift should not attempt another fetch until
    /// `reset_history` is called.
    pub fn is_history_exhausted(&self, chain_id: u32, wallet_id: String) -> bool {
        self.history_pagination.is_exhausted(chain_id, &wallet_id)
    }

    /// Record the cursor returned after a successful cursor-based fetch (UTXO
    /// chains). Pass `None` when the chain confirms there are no more pages —
    /// this marks the chain as exhausted.
    pub fn advance_history_cursor(
        &self,
        chain_id: u32,
        wallet_id: String,
        next_cursor: Option<String>,
    ) {
        self.history_pagination
            .advance_cursor(chain_id, &wallet_id, next_cursor);
    }

    /// Increment the page counter after a successful page-based fetch (EVM,
    /// etc.). Pass `is_last = true` when the returned page was empty or the
    /// chain indicated no next page.
    pub fn advance_history_page(&self, chain_id: u32, wallet_id: String, is_last: bool) {
        self.history_pagination
            .advance_page(chain_id, &wallet_id, is_last);
    }

    /// Directly set the page counter to `page`. For page-based chains (EVM)
    /// where Swift tracks absolute page numbers (1-indexed). Swift sets the
    /// page to 1 on reset and stores the page that was just fetched after each
    /// successful request.
    pub fn set_history_page(&self, chain_id: u32, wallet_id: String, page: u32) {
        self.history_pagination.set_page(chain_id, &wallet_id, page);
    }

    /// Explicitly mark a (chain, wallet) pair as exhausted or not. Used when
    /// Swift detects an empty page without going through `advance_history_*`.
    pub fn set_history_exhausted(&self, chain_id: u32, wallet_id: String, exhausted: bool) {
        self.history_pagination
            .set_exhausted(chain_id, &wallet_id, exhausted);
    }

    /// Reset pagination state for one (chain, wallet) pair — clears cursor,
    /// page, and exhaustion flag. Call after the user pulls-to-refresh or
    /// after a send confirmation.
    pub fn reset_history(&self, chain_id: u32, wallet_id: String) {
        self.history_pagination.reset(chain_id, &wallet_id);
    }

    /// Reset pagination for all chains of one wallet (e.g. wallet deleted or
    /// user triggers a full history refresh for that wallet).
    pub fn reset_history_for_wallet(&self, wallet_id: String) {
        self.history_pagination.reset_all_for_wallet(&wallet_id);
    }

    /// Reset pagination for all wallets on one chain (e.g. chain re-org or
    /// endpoint switch).
    pub fn reset_history_for_chain(&self, chain_id: u32) {
        self.history_pagination.reset_chain(chain_id);
    }

    /// Clear all history pagination state. Used on full account wipe / logout.
    pub fn reset_all_history(&self) {
        self.history_pagination.reset_all();
    }

    // ── Typed persistence: 3 stores that previously did Swift→Rust JSON-shuttle ────
    //
    // The `*_state` JSON shuttle (Swift loads JSON → calls decode FFI → gets
    // typed struct) is replaced with single typed methods that do load+decode
    // and encode+save inside Rust. Halves FFI traffic on every persist op and
    // removes a full JSON parse cost per load.

    /// Load the persisted price-alert store. Returns `None` if no value has
    /// been saved yet or if the on-disk shape can't be decoded.
    pub async fn load_price_alert_store(
        &self,
        db_path: String,
        key: String,
    ) -> Result<Option<crate::store::persistence_models::CorePersistedPriceAlertStore>, SpectraBridgeError> {
        let json = tokio::task::spawn_blocking(move || sqlite_load(&db_path, &key))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)?;
        if json == "{}" {
            return Ok(None);
        }
        Ok(serde_json::from_str(&json).ok())
    }

    /// Persist the price-alert store typed.
    pub async fn save_price_alert_store(
        &self,
        db_path: String,
        key: String,
        value: crate::store::persistence_models::CorePersistedPriceAlertStore,
    ) -> Result<(), SpectraBridgeError> {
        let json = serde_json::to_string(&value).map_err(SpectraBridgeError::from)?;
        tokio::task::spawn_blocking(move || sqlite_save(&db_path, &key, &json))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Load the persisted address-book store.
    pub async fn load_address_book_store(
        &self,
        db_path: String,
        key: String,
    ) -> Result<Option<crate::store::persistence_models::CorePersistedAddressBookStore>, SpectraBridgeError> {
        let json = tokio::task::spawn_blocking(move || sqlite_load(&db_path, &key))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)?;
        if json == "{}" {
            return Ok(None);
        }
        Ok(serde_json::from_str(&json).ok())
    }

    /// Persist the address-book store typed.
    pub async fn save_address_book_store(
        &self,
        db_path: String,
        key: String,
        value: crate::store::persistence_models::CorePersistedAddressBookStore,
    ) -> Result<(), SpectraBridgeError> {
        let json = serde_json::to_string(&value).map_err(SpectraBridgeError::from)?;
        tokio::task::spawn_blocking(move || sqlite_save(&db_path, &key, &json))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }
}

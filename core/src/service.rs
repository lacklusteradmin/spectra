//! WalletService — the stateful async UniFFI object that Swift / Kotlin talk to.
//!
//! ## Design
//!
//! - All chain operations are `async` internally; UniFFI 0.29 with the tokio
//!   feature wraps them into `async fn` on the Swift side automatically.
//! - The service does not own secrets. It receives private key bytes per-call
//!   (Swift reads from Keychain and passes them in). This keeps the Rust layer
//!   stateless with respect to secrets.
//! - Endpoint lists are set at construction time and can be rebuilt by calling
//!   `update_endpoints`.
//! - All public methods return `Result<String, SpectraBridgeError>` — the
//!   `String` is a JSON-encoded response that Swift deserializes on its side.
//!
//! ## Chain IDs (frozen — must not change)
//!
//! | ID | Chain               |
//! |----|---------------------|
//! |  0 | Bitcoin             |
//! |  1 | Ethereum            |
//! |  2 | Solana              |
//! |  3 | Dogecoin            |
//! |  4 | XRP                 |
//! |  5 | Litecoin            |
//! |  6 | Bitcoin Cash        |
//! |  7 | Tron                |
//! |  8 | Stellar             |
//! |  9 | Cardano             |
//! | 10 | Polkadot            |
//! | 11 | Arbitrum            |
//! | 12 | Optimism            |
//! | 13 | Avalanche           |
//! | 14 | Sui                 |
//! | 15 | Aptos               |
//! | 16 | TON                 |
//! | 17 | NEAR                |
//! | 18 | ICP                 |
//! | 19 | Monero              |
//! | 20 | Base                |
//! | 21 | Ethereum Classic    |
//! | 22 | Bitcoin SV          |
//! | 23 | BNB Chain           |

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use rusqlite;

use crate::chains::bitcoin::UtxoTxStatus;
use crate::chains::registry::{Chain, EndpointSlot};
use crate::tokens;
use crate::chains::{
    aptos::AptosClient,
    bitcoin::{BitcoinClient, BitcoinSendParams, sign_and_broadcast as bitcoin_sign_and_broadcast},
    bitcoin_cash::BitcoinCashClient,
    bitcoin_sv::BitcoinSvClient,
    cardano::CardanoClient,
    dogecoin::DogecoinClient,
    evm::EvmClient,
    icp::IcpClient,
    litecoin::LitecoinClient,
    monero::MoneroClient,
    near::NearClient,
    polkadot::PolkadotClient,
    solana::SolanaClient,
    stellar::StellarClient,
    sui::SuiClient,
    ton::TonClient,
    tron::TronClient,
    xrp::XrpClient,
};
use crate::history_store::HistoryPaginationStore;
use crate::http::HttpClient;
use crate::secret_store::SecretStore;
use crate::state::{AssetHolding, CoreAppState, StateCommand, WalletSummary, reduce_state_in_place};
use crate::SpectraBridgeError;

// ----------------------------------------------------------------
// Endpoint configuration (passed in from Swift)
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct ChainEndpoints {
    pub chain_id: u32,
    pub endpoints: Vec<String>,
    /// Optional API key for services that require one (Blockfrost, Subscan, etc.)
    pub api_key: Option<String>,
}

/// Pre-indexed endpoint store: O(1) lookup by chain_id, Arc-wrapped endpoints
/// to avoid cloning Vec<String> on every network call.
#[derive(Debug, Clone, Default)]
struct EndpointIndex {
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

// ----------------------------------------------------------------
// WalletService
// ----------------------------------------------------------------

/// The primary UniFFI-exported object. Swift holds one of these for the
/// lifetime of the app session.
#[derive(uniffi::Object)]
pub struct WalletService {
    endpoints: Arc<RwLock<EndpointIndex>>,
    /// Phase 2.3 — per-wallet history pagination state (cursor/page/exhaustion).
    history_pagination: Arc<HistoryPaginationStore>,
    /// Phase 2.7 — optional Keychain delegate (set via `set_secret_store`).
    secret_store: Arc<std::sync::RwLock<Option<Arc<dyn SecretStore>>>>,
    /// Phase 2.1 — canonical in-memory wallet + holdings state.
    wallet_state: Arc<RwLock<CoreAppState>>,
    /// User's Etherscan V2 API key. Shared across all EVM chains because
    /// Etherscan v2 dispatches by `chainid` parameter against a single host.
    etherscan_api_key: Arc<std::sync::RwLock<String>>,
}

/// Typed app settings record for UniFFI — Rust handles JSON serialization to/from SQLite.
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

/// Typed token descriptor for passing token arrays across UniFFI without JSON.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenDescriptor {
    pub contract: String,
    pub symbol: String,
    pub decimals: u8,
    pub name: Option<String>,
}

/// Typed result for token balance queries — returned directly via UniFFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenBalanceResult {
    pub contract_address: String,
    pub symbol: String,
    pub decimals: u8,
    pub balance_raw: String,
    pub balance_display: String,
}

/// Unified per-chain native balance projection used by `fetch_native_balance_summary`.
/// `smallest_unit` is a base-10 integer string (sats, lamports, wei, yocto-NEAR, ...);
/// `amount_display` is the chain's human-readable native amount (e.g. "0.005").
#[derive(Debug, Clone, uniffi::Record)]
pub struct NativeBalanceSummary {
    pub smallest_unit: String,
    pub amount_display: String,
    pub utxo_count: u32,
}

/// Light-weight EVM address probe output. Used by chain-risk warnings to
/// decide whether a destination looks "fresh" (zero balance + zero nonce).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmAddressProbe {
    pub nonce: i64,
    pub balance_eth: f64,
}

/// Format a smallest-unit u128 amount as a fixed-decimal string for chains
/// whose typed balance struct doesn't already provide a `_display` field.
fn format_smallest_unit_decimal(amount: u128, decimals: u32) -> String {
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

    // `fetch_balance` / `fetch_balance_auto` live in the plain-impl block
    // below (JSON shuttle — kept internal, not exported to Swift).

    pub async fn fetch_solana_balance_typed(
        &self,
        address: String,
    ) -> Result<crate::chains::solana::SolanaBalance, SpectraBridgeError> {
        let endpoints = self.endpoints_for(2).await;
        let client = crate::chains::solana::SolanaClient::new(endpoints);
        client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)
    }

    pub async fn fetch_near_balance_typed(
        &self,
        address: String,
    ) -> Result<crate::chains::near::NearBalance, SpectraBridgeError> {
        let endpoints = self.endpoints_for(17).await;
        let client = crate::chains::near::NearClient::new(endpoints);
        client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)
    }

    pub async fn fetch_erc20_balance_typed(
        &self,
        chain_id: u32,
        contract: String,
        holder: String,
    ) -> Result<crate::chains::evm::Erc20Balance, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id).filter(|c| c.is_evm()).ok_or_else(|| {
            SpectraBridgeError::from(format!("fetch_token_balance_typed: unsupported chain_id: {chain_id}"))
        })?;
        let endpoints = self.endpoints_for(chain.id()).await;
        let client = crate::chains::evm::EvmClient::new(endpoints, chain.evm_chain_id());
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
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain_id: {chain_id}")))?;
        let endpoints = self.endpoints_for(chain.id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.confirmed_sats.to_string(),
                    amount_display: format_smallest_unit_decimal(bal.confirmed_sats as u128, 8),
                    utxo_count: bal.utxo_count as u32,
                })
            }
            c if c.is_evm() => {
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.balance_wei,
                    amount_display: bal.balance_display,
                    utxo_count: 0,
                })
            }
            Chain::Solana => {
                let client = SolanaClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.lamports.to_string(),
                    amount_display: bal.sol_display,
                    utxo_count: 0,
                })
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.balance_koin.to_string(),
                    amount_display: bal.balance_display,
                    utxo_count: 0,
                })
            }
            Chain::Xrp => {
                let client = XrpClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.drops.to_string(),
                    amount_display: bal.xrp_display,
                    utxo_count: 0,
                })
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.balance_sat.to_string(),
                    amount_display: bal.balance_display,
                    utxo_count: 0,
                })
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.balance_sat.to_string(),
                    amount_display: bal.balance_display,
                    utxo_count: 0,
                })
            }
            Chain::Tron => {
                let client = TronClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.sun.to_string(),
                    amount_display: bal.trx_display,
                    utxo_count: 0,
                })
            }
            Chain::Stellar => {
                let client = StellarClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.stroops.to_string(),
                    amount_display: bal.xlm_display,
                    utxo_count: 0,
                })
            }
            Chain::Cardano => {
                let api_key = self.api_key_for(chain.id()).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.lovelace.to_string(),
                    amount_display: bal.ada_display,
                    utxo_count: 0,
                })
            }
            Chain::Polkadot => {
                let subscan = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.planck.to_string(),
                    amount_display: bal.dot_display,
                    utxo_count: 0,
                })
            }
            Chain::Sui => {
                let client = SuiClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.mist.to_string(),
                    amount_display: bal.sui_display,
                    utxo_count: 0,
                })
            }
            Chain::Aptos => {
                let client = AptosClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.octas.to_string(),
                    amount_display: bal.apt_display,
                    utxo_count: 0,
                })
            }
            Chain::Ton => {
                let api_key = self.api_key_for(chain.id()).await;
                let client = TonClient::new(endpoints, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.nanotons.to_string(),
                    amount_display: bal.ton_display,
                    utxo_count: 0,
                })
            }
            Chain::Near => {
                let client = NearClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.yocto_near,
                    amount_display: bal.near_display,
                    utxo_count: 0,
                })
            }
            Chain::Icp => {
                let ic_endpoints = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.e8s.to_string(),
                    amount_display: bal.icp_display,
                    utxo_count: 0,
                })
            }
            Chain::Monero => {
                let client = MoneroClient::new(endpoints);
                let bal = client.fetch_balance(0).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.piconeros.to_string(),
                    amount_display: bal.xmr_display,
                    utxo_count: 0,
                })
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(NativeBalanceSummary {
                    smallest_unit: bal.balance_sat.to_string(),
                    amount_display: bal.balance_display,
                    utxo_count: 0,
                })
            }
            c => Err(SpectraBridgeError::from(format!("unsupported chain: {c:?}"))),
        }
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
                let by_mint: std::collections::HashMap<&str, &crate::chains::solana::SplBalance> =
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
            let normalized = if pk.starts_with("0x") { pk[2..].to_string() } else { pk.clone() };
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

        Ok(crate::send::SendExecutionResult {
            result_json,
            transaction_hash: outcome.transaction_hash,
            payload_format: outcome.payload_format,
        })
    }

    // `build_execute_send_payload` lives in the plain-impl block below
    // (internal helper for `execute_send` — not exported to Swift).

    // ----------------------------------------------------------------
    // Bitcoin HD — seed → account xpub derivation
    // ----------------------------------------------------------------

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

    // ----------------------------------------------------------------
    // Bitcoin HD multi-address (xpub / ypub / zpub)
    // ----------------------------------------------------------------

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

    // `fetch_bitcoin_xpub_balance` lives in the plain-impl block below
    // (JSON shuttle — kept internal, not exported to Swift).

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
        let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
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

    // ----------------------------------------------------------------
    // Price / fiat rate service
    // ----------------------------------------------------------------

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
        let parsed_provider = match crate::price::PriceProvider::from_str(&provider) {
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
        let parsed_provider = match crate::price::FiatRateProvider::from_str(&provider) {
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
                        crate::chains::evm::format_evm_decimals(&entry.amount_raw, dec);
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
        Ok(crate::send::flow_helpers::core_evm_has_contract_code(code))
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

    // ----------------------------------------------------------------
    // Phase 2.1 — WalletStore CRUD (SQLite-backed snapshot)
    // ----------------------------------------------------------------

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

    // ----------------------------------------------------------------
    // Phase 2.1 — Relational wallet state (keypool + owned addresses)
    //
    // These replace UserDefaults JSON blobs on the Swift side.
    // All calls run in spawn_blocking because rusqlite is not async.
    // ----------------------------------------------------------------

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

    // ----------------------------------------------------------------
    // Phase 2.8 — Transaction history persistence (SQLite-backed)
    // ----------------------------------------------------------------

    /// Upsert a batch of transaction history records. `records[*].payload_json`
    /// is the raw PersistedTransactionRecord JSON; Rust base64-encodes it into
    /// the stored `payload` column to match the `HistoryRecord` schema.
    pub async fn upsert_history_records(
        &self,
        db_path: String,
        records: Vec<crate::ffi::HistoryRecordEncodeInput>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let records = history_records_from_encode_inputs(records);
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
        records: Vec<crate::ffi::HistoryRecordEncodeInput>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let records = history_records_from_encode_inputs(records);
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

    // ----------------------------------------------------------------
    // Phase 3 — typed wallet state (no JSON round-trip)
    // ----------------------------------------------------------------

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

    // ----------------------------------------------------------------
    // Phase 2.3 — History pagination state
    // ----------------------------------------------------------------

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
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
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
            c => return Err(SpectraBridgeError::from(format!(
                "fetch_utxo_tx_status: unsupported chain: {c:?}"
            ))),
        };
        Ok(status)
    }
}

// ----------------------------------------------------------------
// Token catalog (synchronous free function — no network I/O)
// ----------------------------------------------------------------

/// Return the built-in token catalog as a typed list.
/// Pass `chain_id = 4294967295` (u32::MAX) to get all chains.
/// This is a synchronous free function so Swift can call it from a `static let`.
#[uniffi::export]
pub fn list_builtin_tokens(chain_id: u32) -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(chain_id)
}

// ----------------------------------------------------------------
// BIP39 mnemonic utilities (free functions — no network I/O)
// ----------------------------------------------------------------

/// Generate a new random BIP-39 mnemonic with the requested word count.
///
/// `word_count` must be 12, 15, 18, 21, or 24. Any other value falls
/// back silently to 12 words. Returns the space-joined mnemonic phrase.
#[uniffi::export]
pub fn generate_mnemonic(word_count: u32) -> String {
    use bip39::{Mnemonic, Language};
    use rand::RngCore;

    // BIP-39 entropy bytes: 128/160/192/224/256 bits → 12/15/18/21/24 words.
    let entropy_bytes: usize = match word_count {
        15 => 20,
        18 => 24,
        21 => 28,
        24 => 32,
        _  => 16, // default: 12 words
    };
    let mut entropy = vec![0u8; entropy_bytes];
    rand::thread_rng().fill_bytes(&mut entropy);
    // bip39 v2: Mnemonic::from_entropy returns an error only if the entropy
    // length is invalid — our lengths are always valid so unwrap is safe.
    Mnemonic::from_entropy_in(Language::English, &entropy)
        .expect("valid entropy length")
        .to_string()
}

/// Validate a BIP-39 mnemonic phrase.
///
/// Returns `true` if `phrase` is a valid English BIP-39 mnemonic (correct
/// word count, all words in the word list, and correct checksum). Returns
/// `false` for any other input.
#[uniffi::export]
pub fn validate_mnemonic(phrase: String) -> bool {
    use bip39::{Mnemonic, Language};
    phrase.trim().parse::<Mnemonic>().is_ok()
        || Mnemonic::parse_in(Language::English, phrase.trim()).is_ok()
}

/// Return the full BIP-39 English word list as a newline-delimited string
/// (2048 words, one per line, alphabetically sorted).
#[uniffi::export]
pub fn bip39_english_wordlist() -> String {
    static WORDLIST: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        bip39::Language::English.word_list().join("\n")
    });
    WORDLIST.clone()
}

// ----------------------------------------------------------------
// Internal helpers (not exported)
// ----------------------------------------------------------------

/// Convert typed history encode inputs from Swift into the base64-payload
/// `HistoryRecord` shape expected by the SQLite layer.
fn history_records_from_encode_inputs(
    records: Vec<crate::ffi::HistoryRecordEncodeInput>,
) -> Vec<crate::wallet_db::HistoryRecord> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    records
        .into_iter()
        .map(|r| crate::wallet_db::HistoryRecord {
            id: r.id,
            wallet_id: r.wallet_id,
            chain_name: r.chain_name,
            tx_hash: r.tx_hash,
            created_at: r.created_at,
            payload: engine.encode(r.payload_json.as_bytes()),
        })
        .collect()
}

impl WalletService {
    async fn endpoints_for(&self, chain_id: u32) -> Vec<String> {
        let guard = self.endpoints.read().await;
        guard.endpoints
            .get(&chain_id)
            .map(|arc| (**arc).clone())
            .unwrap_or_default()
    }

    async fn api_key_for(&self, chain_id: u32) -> Option<String> {
        let guard = self.endpoints.read().await;
        guard.api_keys.get(&chain_id).cloned()
    }

    /// Internal native-balance apply helper used by the refresh engine's
    /// tokio cycle (previously also exported to Swift as
    /// `updateNativeBalanceTyped` but that path was a JSON shuttle — Swift
    /// received JSON from the observer and passed it straight back; since
    /// 2026-04-19 the refresh engine applies internally and the observer
    /// callback delivers a typed `WalletSummary`).
    pub(crate) async fn update_native_balance_typed(
        &self,
        wallet_id: String,
        chain_id: u32,
        balance_json: String,
    ) -> Result<Option<WalletSummary>, SpectraBridgeError> {
        let amount = match native_amount_from_balance_json(chain_id, &balance_json) {
            Some(a) => a,
            None => return Ok(None),
        };
        self.apply_native_amount_typed(wallet_id, chain_id, amount).await
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
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let fee = client.fetch_fee_rate(6).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&fee)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let fee = client.fetch_fee_estimate().await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&fee)?)
            }
            Chain::Solana => Ok(fee_preview(chain_id, 5_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Xrp => {
                let client = XrpClient::new(endpoints);
                let drops = client.fetch_fee().await.map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, drops as u128, chain.native_decimals(), chain.coin_symbol(), "rpc"))
            }
            Chain::Tron => Ok(fee_preview(chain_id, 1_000_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Stellar => {
                let client = StellarClient::new(endpoints);
                let stroops = client
                    .fetch_base_fee()
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, stroops as u128, chain.native_decimals(), chain.coin_symbol(), "rpc"))
            }
            Chain::Cardano => Ok(fee_preview(chain_id, 170_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Polkadot => Ok(fee_preview(chain_id, 160_000_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Sui => Ok(fee_preview(chain_id, 1_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Aptos => {
                let client = AptosClient::new(endpoints);
                let price = client
                    .fetch_gas_price()
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, price as u128, chain.native_decimals(), chain.coin_symbol(), "rpc"))
            }
            Chain::Ton => Ok(fee_preview(chain_id, 7_000_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Near => Ok(fee_preview_str(
                chain_id,
                "1000000000000000000000",
                "0.001",
                chain.coin_symbol(),
                "static",
            )),
            Chain::Icp => Ok(fee_preview(chain_id, 10_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Monero => Ok(fee_preview(chain_id, 500_000_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Dogecoin => Ok(fee_preview(chain_id, 1_000_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::Litecoin => Ok(fee_preview(chain_id, 10_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::BitcoinCash => Ok(fee_preview(chain_id, 2_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            Chain::BitcoinSV => Ok(fee_preview(chain_id, 1_000, chain.native_decimals(), chain.coin_symbol(), "static")),
            _ => Ok(json!({"note": "fee estimation not supported for this chain"}).to_string()),
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
        params_json: String,
    ) -> Result<String, SpectraBridgeError> {
        let params: serde_json::Value = serde_json::from_str(&params_json)?;
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
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let send_params = BitcoinSendParams {
                    from_address: from.to_string(),
                    private_key_hex: priv_hex.to_string(),
                    to_address: to.to_string(),
                    amount_sats,
                    fee_rate: crate::chains::bitcoin::FeeRate { sats_per_vbyte: fee_rate_svb },
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
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let planck: u128 = params["planck"].as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| params["planck"].as_u64().map(|n| n as u128))
                    .ok_or("missing planck")?;
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let subscan = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let r = client.sign_and_submit(from, to, planck, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
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
                let ic_endpoints = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
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
        params_json: String,
    ) -> Result<String, SpectraBridgeError> {
        let params: serde_json::Value = serde_json::from_str(&params_json)?;
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
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain_id: {chain_id}")))?;
        let endpoints = self.endpoints_for(chain.id()).await;

        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Solana => {
                let client = SolanaClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Xrp => {
                let client = XrpClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Tron => {
                let client = TronClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Stellar => {
                let client = StellarClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Cardano => {
                let api_key = self.api_key_for(chain.id()).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Polkadot => {
                let subscan = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Sui => {
                let client = SuiClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Aptos => {
                let client = AptosClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Ton => {
                let api_key = self.api_key_for(chain.id()).await;
                let client = TonClient::new(endpoints, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Near => {
                let client = NearClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Icp => {
                let ic_endpoints = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::Monero => {
                let client = MoneroClient::new(endpoints);
                let bal = client.fetch_balance(0).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            c => Err(SpectraBridgeError::from(format!("unsupported chain: {c:?}"))),
        }
    }

    pub(crate) async fn fetch_balance_auto(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        if chain_id == 0 && is_extended_public_key(&address) {
            self.fetch_bitcoin_xpub_balance(address, 20, 20).await
        } else {
            self.fetch_balance(chain_id, address).await
        }
    }

    pub(crate) async fn fetch_history(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain_id: {chain_id}")))?;
        let endpoints = self.endpoints_for(chain.id()).await;

        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let h = client.fetch_history(&address, None).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let api_key_owned = self
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
                let h = client
                    .fetch_history(
                        &address,
                        "https://api.etherscan.io",
                        api_key_str,
                        c.evm_chain_id(),
                    )
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Solana => {
                let client = SolanaClient::new(endpoints);
                let h = client.fetch_unified_history(&address, 50).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Xrp => {
                let client = XrpClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Tron => {
                let client = TronClient::new(endpoints);
                let tronscan = self.endpoints_for(chain.endpoint_id(EndpointSlot::Explorer)).await
                    .into_iter().next()
                    .unwrap_or_else(|| "https://apilist.tronscan.org".to_string());
                let h = client.fetch_unified_history(&address, &tronscan, 50)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Stellar => {
                let client = StellarClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Cardano => {
                let api_key = self.api_key_for(chain.id()).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Polkadot => {
                let subscan = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let api_key = self.api_key_for(chain.id()).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Sui => {
                let client = SuiClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Aptos => {
                let client = AptosClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Ton => {
                let api_key = self.api_key_for(chain.id()).await;
                let client = TonClient::new(endpoints, api_key);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Near => {
                let client = NearClient::new(endpoints);
                let indexer = self.endpoints_for(chain.endpoint_id(EndpointSlot::Explorer)).await
                    .into_iter().next()
                    .unwrap_or_else(|| "https://api.kitwallet.app".to_string());
                let h = client.fetch_history(&address, &indexer)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Icp => {
                let ic_endpoints = self.endpoints_for(chain.endpoint_id(EndpointSlot::Secondary)).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            Chain::Monero => {
                let client = MoneroClient::new(endpoints);
                let h = client.fetch_history(0).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            c => Err(SpectraBridgeError::from(format!("unsupported chain: {c:?}"))),
        }
    }

    pub(crate) async fn fetch_bitcoin_xpub_balance(
        &self,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(0).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
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
                let client = BitcoinClient::new(HttpClient::shared(), eps, "mainnet");
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
                let client = BitcoinClient::new(HttpClient::shared(), eps, "mainnet");
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
        let fee = fee_res.unwrap_or(crate::chains::evm::EvmFeeEstimate {
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
    ) -> Result<String, SpectraBridgeError> {
        use crate::send::payload::*;
        use crate::send::preview_decode::build_utxo_sat_send_payload;

        let from = &req.from_address;
        let to = &req.to_address;
        let amount = req.amount;
        let priv_str = priv_hex.to_string();

        let chain = Chain::from_id(req.chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!("execute_send: unsupported chain_id: {}", req.chain_id))
        })?;

        if let Some(ref contract) = req.contract_address {
            let decimals = req.token_decimals.unwrap_or(6);
            return match chain {
                c if c.is_evm() => {
                    let amount_raw = crate::send::preview_decode::amount_to_raw_units_string(amount, decimals);
                    let overrides = req.evm_overrides_fragment.as_deref().unwrap_or("");
                    Ok(crate::send::ethereum::build_evm_token_send_payload(
                        from.clone(), contract.clone(), to.clone(),
                        amount_raw, priv_str, overrides.to_string(),
                    ))
                }
                Chain::Tron => Ok(build_tron_token_send_payload(
                    from.clone(), contract.clone(), to.clone(), amount, decimals, priv_str,
                )),
                Chain::Solana => Ok(build_solana_token_send_payload(
                    pub_hex.clone().unwrap_or_default(),
                    contract.clone(), to.clone(), amount, decimals, priv_str,
                )),
                Chain::Near => Ok(build_near_token_send_payload(
                    from.clone(), contract.clone(), to.clone(), amount, decimals,
                    priv_str, pub_hex.clone().unwrap_or_default(),
                )),
                c => Err(SpectraBridgeError::from(format!(
                    "execute_send: unsupported token chain: {c:?}"
                ))),
            };
        }

        Ok(match chain {
            Chain::Bitcoin => build_btc_send_payload(
                from.clone(), to.clone(), amount,
                req.fee_rate_svb.unwrap_or(10.0), priv_str,
            ),
            c if c.is_evm() => {
                let value_wei = crate::send::preview_decode::amount_to_raw_units_string(amount, 18);
                let overrides = req.evm_overrides_fragment.as_deref().unwrap_or("");
                crate::send::ethereum::build_evm_native_send_payload(
                    from.clone(), to.clone(), value_wei, priv_str, overrides.to_string(),
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
            c => return Err(SpectraBridgeError::from(format!(
                "execute_send: unsupported chain: {c:?}"
            ))),
        })
    }

    pub(crate) async fn apply_native_amount_typed(
        &self,
        wallet_id: String,
        chain_id: u32,
        amount: f64,
    ) -> Result<Option<WalletSummary>, SpectraBridgeError> {
        let template = match native_coin_template(chain_id) {
            Some(t) => t,
            None => return Ok(None),
        };
        let holding = AssetHolding { amount, ..template };
        let mut state = self.wallet_state.write().await;
        if let Some(wallet) = state.wallets.iter_mut().find(|w| w.id == wallet_id) {
            upsert_asset_holding(&mut wallet.holdings, holding);
            return Ok(Some(wallet.clone()));
        }
        Ok(None)
    }
}

// ----------------------------------------------------------------
// Param extraction helpers
// ----------------------------------------------------------------

fn str_field<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, SpectraBridgeError> {
    params[key]
        .as_str()
        .ok_or_else(|| SpectraBridgeError::from(format!("missing field: {key}")))
}

fn hex_field(params: &serde_json::Value, key: &str) -> Result<Vec<u8>, SpectraBridgeError> {
    let s = str_field(params, key)?;
    hex::decode(s).map_err(|e| SpectraBridgeError::from(format!("{key} hex decode: {e}")))
}

/// Extract the normalised native balance (in display units, i.e. divided by
/// the chain's smallest-unit factor) from a balance JSON value.
///
/// Supported chain_ids: 2, 4, 8, 9, 10, 14, 15, 16, 17, 18, 19.
/// Returns 0.0 for unknown / unsupported chains.
fn simple_chain_balance_display(chain_id: u32, obj: &serde_json::Value) -> f64 {
    let u64_field = |key: &str| -> f64 {
        obj[key].as_u64().map(|n| n as f64)
            .or_else(|| obj[key].as_str().and_then(|s| s.parse::<u64>().ok()).map(|n| n as f64))
            .unwrap_or(0.0)
    };
    let i64_field = |key: &str| -> f64 {
        obj[key].as_i64().map(|n| n as f64)
            .or_else(|| obj[key].as_str().and_then(|s| s.parse::<i64>().ok()).map(|n| n as f64))
            .unwrap_or(0.0)
    };
    let Some(chain) = Chain::from_id(chain_id) else { return 0.0 };
    let factor = 10f64.powi(chain.native_decimals() as i32);
    match chain {
        // Outliers: signed, string-encoded big ints, or multi-field balance shapes.
        Chain::Stellar => i64_field("stroops") / factor,
        Chain::Polkadot => {
            obj["planck"].as_u64().map(|n| n as f64)
                .or_else(|| obj["planck"].as_str().and_then(|s| s.parse::<f64>().ok()))
                .unwrap_or(0.0) / factor
        }
        Chain::Near => {
            if let Some(s) = obj["near_display"].as_str() {
                s.parse::<f64>().unwrap_or(0.0)
            } else {
                obj["yocto_near"].as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|y| y / factor)
                    .unwrap_or(0.0)
            }
        }
        // Standard shape: single u64 field divided by 10^decimals.
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

/// Flat struct for direct serialization — avoids the intermediate
/// `serde_json::Value` + `Map` heap allocation that `json!()` produces.
#[derive(Serialize)]
struct FeePreview<'a> {
    chain_id: u32,
    native_fee_raw: &'a str,
    native_fee_display: &'a str,
    unit: &'a str,
    source: &'a str,
}

/// Build a `fee_preview` JSON string from an integer raw amount plus
/// decimals. Scales the raw amount down for a human-readable display field.
fn fee_preview(chain_id: u32, raw: u128, decimals: u8, unit: &str, source: &str) -> String {
    let display = format_decimals(raw, decimals);
    let raw_str = raw.to_string();
    serde_json::to_string(&FeePreview {
        chain_id, native_fee_raw: &raw_str, native_fee_display: &display, unit, source,
    }).unwrap()
}

/// Variant that accepts pre-computed raw/display strings. Used when the raw
/// amount doesn't fit in `u128` (e.g. NEAR's 10^21 yoctoNEAR).
fn fee_preview_str(
    chain_id: u32,
    raw: &str,
    display: &str,
    unit: &str,
    source: &str,
) -> String {
    serde_json::to_string(&FeePreview {
        chain_id, native_fee_raw: raw, native_fee_display: display, unit, source,
    }).unwrap()
}

/// Compute a UTXO capacity fee preview.
///
/// Uses P2PKH sizing (148 bytes/input, 34 bytes/output, 10 bytes overhead).
/// The preview assumes all confirmed UTXOs above the 546-satoshi dust threshold
/// are selected, and computes the fee for a single-output (max-send) transaction.
fn utxo_fee_preview_json(utxo_values: Vec<u64>, fee_rate: u64) -> String {
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

    // Single-output tx (send everything, no change).
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

/// Scale a raw integer by `10^decimals` into a human-readable decimal
/// string with up to 6 fractional digits of precision.
fn format_decimals(raw: u128, decimals: u8) -> String {
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

/// Parse optional EVM transaction overrides from a `sign_and_send` params
/// blob. All fields default to `None` — the Rust client then falls back to
/// its standard pending-nonce / recommended-fee / estimated-gas behavior.
///
/// Accepted fields (all optional):
///   - `"nonce"`: decimal integer or string
///   - `"gas_limit"`: decimal integer or string
///   - `"max_fee_per_gas_wei"`: decimal string
///   - `"max_priority_fee_per_gas_wei"`: decimal string
fn read_evm_overrides(params: &serde_json::Value) -> crate::chains::evm::EvmSendOverrides {
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
    crate::chains::evm::EvmSendOverrides {
        nonce,
        max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei,
        gas_limit,
    }
}

// ----------------------------------------------------------------
// SQLite helpers (blocking — must be called via spawn_blocking)
// ----------------------------------------------------------------

fn sqlite_open(db_path: &str) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| format!("sqlite open {db_path}: {e}"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS state (
            key      TEXT    PRIMARY KEY,
            value    TEXT    NOT NULL,
            saved_at INTEGER NOT NULL
        );",
    )
    .map_err(|e| format!("sqlite create table: {e}"))?;
    Ok(conn)
}

fn sqlite_load(db_path: &str, key: &str) -> Result<String, String> {
    let conn = sqlite_open(db_path)?;
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
}

// ----------------------------------------------------------------
// Phase 2.1 helpers — not UniFFI-exported
// ----------------------------------------------------------------

/// Parse a chain-specific balance JSON blob and return the native coin amount
/// as a plain f64.  Mirrors the RustBalanceDecoder logic on the Swift side.
fn native_amount_from_balance_json(chain_id: u32, json: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let chain = Chain::from_id(chain_id)?;
    let factor = 10f64.powi(chain.native_decimals() as i32);
    match chain {
        // EVM: prefer pre-computed display; fall back to `balance_wei` string.
        c if c.is_evm() => v["balance_display"].as_str().and_then(|s| s.parse().ok())
            .or_else(|| v["balance_wei"].as_str().and_then(|s| s.parse::<f64>().ok()).map(|w| w / factor))
            .or_else(|| v["balance_wei"].as_f64().map(|w| w / factor)),
        // NEAR: prefer pre-computed display; fall back to yoctoNEAR string.
        Chain::Near => v["near_display"].as_str().and_then(|s| s.parse().ok())
            .or_else(|| v["yocto_near"].as_str().and_then(|s| s.parse::<f64>().ok()).map(|y| y / factor))
            .or_else(|| v["yocto_near"].as_f64().map(|y| y / factor)),
        // Stellar stroops is i64; take magnitude.
        Chain::Stellar => {
            let field = chain.native_balance_field()?;
            v[field].as_i64().map(|s| s.unsigned_abs() as f64 / factor)
        }
        // Polkadot planck is a big integer stored as a string.
        Chain::Polkadot => {
            let field = chain.native_balance_field()?;
            v[field].as_str().and_then(|s| s.parse::<f64>().ok())
                .or_else(|| v[field].as_f64())
                .map(|p| p / factor)
        }
        // Standard shape: single u64 field divided by 10^decimals.
        c => {
            let field = c.native_balance_field()?;
            v[field].as_u64().map(|n| n as f64 / factor)
        }
    }
}

/// Return a zero-amount AssetHolding template for the native coin of each
/// chain.  Used as the default when the holding doesn't exist yet.
fn native_coin_template(chain_id: u32) -> Option<AssetHolding> {
    let chain = Chain::from_id(chain_id)?;
    Some(AssetHolding {
        name: chain.coin_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        market_data_id: chain.market_data_id().to_string(),
        coin_gecko_id: chain.coin_gecko_id().to_string(),
        chain_name: chain.chain_display_name().to_string(),
        token_standard: "Native".to_string(),
        contract_address: None,
        amount: 0.0,
        price_usd: 0.0,
    })
}

/// Upsert a holding by (chain_name, contract_address) for tokens or
/// (chain_name, symbol) for native coins.  Replaces the full holding when
/// found, appends otherwise.
fn upsert_asset_holding(holdings: &mut Vec<AssetHolding>, new: AssetHolding) {
    let pos = match &new.contract_address {
        Some(contract) => holdings
            .iter()
            .position(|h| h.chain_name == new.chain_name && h.contract_address.as_deref() == Some(contract.as_str())),
        None => holdings
            .iter()
            .position(|h| h.chain_name == new.chain_name && h.symbol == new.symbol && h.contract_address.is_none()),
    };
    if let Some(idx) = pos {
        holdings[idx] = new;
    } else {
        holdings.push(new);
    }
}

/// Returns `true` when `s` starts with a BIP-32 extended public key prefix.
fn is_extended_public_key(s: &str) -> bool {
    matches!(
        s.get(..4),
        Some("xpub") | Some("ypub") | Some("zpub") | Some("Ypub") | Some("Zpub")
    )
}

fn sqlite_save(db_path: &str, key: &str, value: &str) -> Result<(), String> {
    let conn = sqlite_open(db_path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO state (key, value, saved_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, saved_at = excluded.saved_at",
        rusqlite::params![key, value, now],
    )
    .map_err(|e| format!("sqlite save: {e}"))?;
    Ok(())
}


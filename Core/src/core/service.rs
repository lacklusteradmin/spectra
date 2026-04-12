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

use crate::core::chains::bitcoin::UtxoTxStatus;
use crate::core::tokens;
use crate::core::chains::{
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
use crate::core::balance_cache::BalanceCache;
use crate::core::history_cache::HistoryCache;
use crate::core::history_store::HistoryPaginationStore;
use crate::core::http::HttpClient;
use crate::core::secret_store::SecretStore;
use crate::SpectraBridgeError;

// ----------------------------------------------------------------
// Endpoint configuration (passed in from Swift)
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainEndpoints {
    pub chain_id: u32,
    pub endpoints: Vec<String>,
    /// Optional API key for services that require one (Blockfrost, Subscan, etc.)
    pub api_key: Option<String>,
}

// ----------------------------------------------------------------
// WalletService
// ----------------------------------------------------------------

/// The primary UniFFI-exported object. Swift holds one of these for the
/// lifetime of the app session.
#[derive(uniffi::Object)]
pub struct WalletService {
    endpoints: Arc<RwLock<Vec<ChainEndpoints>>>,
    /// Phase 2.2 — in-memory balance cache (default TTL: 30 s).
    balance_cache: Arc<BalanceCache>,
    /// Phase 2.3 — in-memory history cache (default TTL: 5 min).
    history_cache: Arc<HistoryCache>,
    /// Phase 2.3 — per-wallet history pagination state (cursor/page/exhaustion).
    history_pagination: Arc<HistoryPaginationStore>,
    /// Phase 2.7 — optional Keychain delegate (set via `set_secret_store`).
    secret_store: Arc<std::sync::RwLock<Option<Arc<dyn SecretStore>>>>,
}

#[uniffi::export]
impl WalletService {
    #[uniffi::constructor]
    pub fn new(endpoints_json: String) -> Result<Arc<Self>, SpectraBridgeError> {
        let endpoints: Vec<ChainEndpoints> = serde_json::from_str(&endpoints_json)?;
        Ok(Arc::new(Self {
            endpoints: Arc::new(RwLock::new(endpoints)),
            balance_cache: Arc::new(BalanceCache::new(30)),
            history_cache: Arc::new(HistoryCache::new(300)), // 5-minute TTL
            history_pagination: Arc::new(HistoryPaginationStore::new()),
            secret_store: Arc::new(std::sync::RwLock::new(None)),
        }))
    }

    pub async fn update_endpoints(&self, endpoints_json: String) -> Result<(), SpectraBridgeError> {
        let new_endpoints: Vec<ChainEndpoints> = serde_json::from_str(&endpoints_json)?;
        let mut guard = self.endpoints.write().await;
        *guard = new_endpoints;
        Ok(())
    }

    // ----------------------------------------------------------------
    // Balance fetch
    // ----------------------------------------------------------------

    pub async fn fetch_balance(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain_id).await;

        match chain_id {
            0 => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            2 => {
                let client = SolanaClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            3 => {
                let client = DogecoinClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            4 => {
                let client = XrpClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            5 => {
                let client = LitecoinClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            6 => {
                let client = BitcoinCashClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            7 => {
                let client = TronClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            8 => {
                let client = StellarClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            9 => {
                let api_key = self.api_key_for(chain_id).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            10 => {
                let subscan = self.endpoints_for(SUBSCAN_OFFSET + chain_id).await;
                let api_key = self.api_key_for(chain_id).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            14 => {
                let client = SuiClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            15 => {
                let client = AptosClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            16 => {
                let api_key = self.api_key_for(chain_id).await;
                let client = TonClient::new(endpoints, api_key);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            17 => {
                let client = NearClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            18 => {
                let ic_endpoints = self.endpoints_for(IC_OFFSET + chain_id).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            19 => {
                let client = MoneroClient::new(endpoints);
                let bal = client.fetch_balance(0).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            22 => {
                let client = BitcoinSvClient::new(endpoints);
                let bal = client.fetch_balance(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            _ => Err(SpectraBridgeError::from(format!("unknown chain_id: {chain_id}"))),
        }
    }

    /// Fetch balance, auto-detecting Bitcoin HD paths.
    ///
    /// For chain_id=0 (Bitcoin) with an extended public key (xpub/ypub/zpub),
    /// delegates to `fetch_bitcoin_xpub_balance`; otherwise calls `fetch_balance`.
    /// Used internally by `BalanceRefreshEngine`; also callable from Swift.
    pub async fn fetch_balance_auto(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        if chain_id == 0 && is_extended_public_key(&address) {
            // 20 receive + 20 change — matches the WalletServiceBridge Swift defaults.
            self.fetch_bitcoin_xpub_balance(address, 20, 20).await
        } else {
            self.fetch_balance(chain_id, address).await
        }
    }

    // ----------------------------------------------------------------
    // History fetch
    // ----------------------------------------------------------------

    pub async fn fetch_history(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain_id).await;

        match chain_id {
            0 => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let h = client.fetch_history(&address, None).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let explorer = self.endpoints_for(EXPLORER_OFFSET + chain_id).await
                    .into_iter().next().unwrap_or_default();
                let api_key = self.api_key_for(chain_id).await;
                let h = client.fetch_history(&address, &explorer, api_key.as_deref())
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            2 => {
                let client = SolanaClient::new(endpoints);
                let h = client.fetch_unified_history(&address, 50).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            3 => {
                let client = DogecoinClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            4 => {
                let client = XrpClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            5 => {
                let client = LitecoinClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            6 => {
                let client = BitcoinCashClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            22 => {
                let client = BitcoinSvClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            7 => {
                let client = TronClient::new(endpoints);
                let tronscan = self.endpoints_for(EXPLORER_OFFSET + chain_id).await
                    .into_iter().next()
                    .unwrap_or_else(|| "https://apilist.tronscan.org".to_string());
                let h = client.fetch_unified_history(&address, &tronscan, 50)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            8 => {
                let client = StellarClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            9 => {
                let api_key = self.api_key_for(chain_id).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            10 => {
                let subscan = self.endpoints_for(SUBSCAN_OFFSET + chain_id).await;
                let api_key = self.api_key_for(chain_id).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            14 => {
                let client = SuiClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            15 => {
                let client = AptosClient::new(endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            16 => {
                let api_key = self.api_key_for(chain_id).await;
                let client = TonClient::new(endpoints, api_key);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            17 => {
                let client = NearClient::new(endpoints);
                let indexer = self.endpoints_for(EXPLORER_OFFSET + chain_id).await
                    .into_iter().next()
                    .unwrap_or_else(|| "https://api.kitwallet.app".to_string());
                let h = client.fetch_history(&address, &indexer)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            18 => {
                let ic_endpoints = self.endpoints_for(IC_OFFSET + chain_id).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
                let h = client.fetch_history(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            19 => {
                let client = MoneroClient::new(endpoints);
                let h = client.fetch_history(0).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&h)?)
            }
            _ => Err(SpectraBridgeError::from(format!("unknown chain_id: {chain_id}"))),
        }
    }

    // ----------------------------------------------------------------
    // Sign and send
    // ----------------------------------------------------------------

    /// Sign and broadcast a transaction.
    ///
    /// `params_json` is chain-specific JSON containing addresses, amounts,
    /// and private keys (read from Keychain by Swift before calling).
    pub async fn sign_and_send(
        &self,
        chain_id: u32,
        params_json: String,
    ) -> Result<String, SpectraBridgeError> {
        let params: serde_json::Value = serde_json::from_str(&params_json)?;
        let endpoints = self.endpoints_for(chain_id).await;

        match chain_id {
            0 => {
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
                    fee_rate: crate::core::chains::bitcoin::FeeRate { sats_per_vbyte: fee_rate_svb },
                    available_utxos: vec![],
                    network_mode: "mainnet".to_string(),
                    enable_rbf: true,
                };
                let r = bitcoin_sign_and_broadcast(&client, send_params)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let value_wei: u128 = params["value_wei"].as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("missing value_wei")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let overrides = read_evm_overrides(&params);
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let r = client
                    .sign_and_broadcast_with_overrides(from, to, value_wei, &priv_bytes, overrides)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            2 => {
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
            4 => {
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
            7 => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_sun = params["amount_sun"].as_u64().ok_or("missing amount_sun")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = TronClient::new(endpoints);
                let r = client.sign_and_broadcast(from, to, amount_sun, &priv_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            14 => {
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
            15 => {
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
            17 => {
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
            3 => {
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
            5 => {
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
            6 => {
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
            22 => {
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
            8 => {
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
            9 => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let amount_lovelace = params["amount_lovelace"].as_u64().ok_or("missing amount_lovelace")?;
                let fee_lovelace = params["fee_lovelace"].as_u64().unwrap_or(170_000);
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let api_key = self.api_key_for(chain_id).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let r = client.sign_and_broadcast(from, to, amount_lovelace, fee_lovelace, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            10 => {
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
                let subscan = self.endpoints_for(SUBSCAN_OFFSET + chain_id).await;
                let api_key = self.api_key_for(chain_id).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let r = client.sign_and_submit(from, to, planck, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            16 => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let nanotons = params["nanotons"].as_u64().ok_or("missing nanotons")?;
                let comment = params["comment"].as_str();
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
                let api_key = self.api_key_for(chain_id).await;
                let client = TonClient::new(endpoints, api_key);
                let seqno = client.fetch_seqno(from).await?;
                let r = client.sign_and_send(to, nanotons, seqno, comment, &priv_arr, &pub_arr)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            18 => {
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
                let ic_endpoints = self.endpoints_for(IC_OFFSET + chain_id).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
                let r = client.sign_and_submit(from, to, e8s, &priv_bytes, pub_bytes)
                    .await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            19 => {
                let to = str_field(&params, "to")?;
                let piconeros = params["piconeros"].as_u64().ok_or("missing piconeros")?;
                let priority = params["priority"].as_u64().unwrap_or(2) as u32;
                let client = MoneroClient::new(endpoints);
                let r = client.send(to, piconeros, 0, priority).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            _ => Err(SpectraBridgeError::from(format!("sign_and_send: unsupported chain_id: {chain_id}"))),
        }
    }

    // ----------------------------------------------------------------
    // Token balance (ERC-20 / SPL / NEP-141 / TRC-20 / Stellar assets)
    // ----------------------------------------------------------------

    /// Fetch a single token balance for the given chain.
    ///
    /// `params_json` schema (chain-specific):
    ///   - EVM chains (1,11,12,13,20,21): `{"contract": "0x…", "holder": "0x…"}`
    ///   - Tron (7): `{"contract": "T…", "holder": "T…"}`
    ///   - Stellar (8): `{"holder": "G…", "asset_code": "USDC", "asset_issuer": "G…"}`
    ///   - NEAR (17): `{"contract": "token.near", "holder": "account.near"}`
    ///   - Solana (2): `{"mint": "<base58>", "owner": "<base58>"}`
    ///
    /// Returns a JSON string Swift can decode directly.
    pub async fn fetch_token_balance(
        &self,
        chain_id: u32,
        params_json: String,
    ) -> Result<String, SpectraBridgeError> {
        let params: serde_json::Value = serde_json::from_str(&params_json)?;
        let endpoints = self.endpoints_for(chain_id).await;

        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let contract = str_field(&params, "contract")?;
                let holder = str_field(&params, "holder")?;
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let bal = client
                    .fetch_erc20_balance(contract, holder)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            7 => {
                // Tron — TRC-20 tokens. Addresses are base58 (`T…`).
                let contract = str_field(&params, "contract")?;
                let holder = str_field(&params, "holder")?;
                let client = TronClient::new(endpoints);
                let bal = client
                    .fetch_trc20_balance(contract, holder)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            8 => {
                // Stellar — custom issued assets (credit_alphanum4/12).
                let holder = str_field(&params, "holder")?;
                let asset_code = str_field(&params, "asset_code")?;
                let asset_issuer = str_field(&params, "asset_issuer")?;
                let client = StellarClient::new(endpoints);
                let bal = client
                    .fetch_asset_balance(holder, asset_code, asset_issuer)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            17 => {
                // NEAR — NEP-141 fungible tokens.
                let contract = str_field(&params, "contract")?;
                let holder = str_field(&params, "holder")?;
                let client = NearClient::new(endpoints);
                let bal = client
                    .fetch_ft_balance(contract, holder)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            2 => {
                // Solana — SPL tokens. We deliberately use "mint"/"owner"
                // field names to mirror Solana RPC terminology.
                let mint = str_field(&params, "mint")?;
                let owner = str_field(&params, "owner")?;
                let client = SolanaClient::new(endpoints);
                let bal = client
                    .fetch_spl_balance(mint, owner)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&bal)?)
            }
            _ => Err(SpectraBridgeError::from(format!(
                "fetch_token_balance: unsupported chain_id: {chain_id}"
            ))),
        }
    }

    /// Fetch balances for a list of tokens in one call.
    ///
    /// `tokens_json` is a JSON array of objects:
    ///   `[{"contract": "<address>", "symbol": "<SYM>", "decimals": <u8>}, …]`
    ///
    /// For Solana (chain_id=2) `"contract"` is the mint address.
    ///
    /// Returns a JSON array in the same shape with additional fields:
    ///   `"balance_raw"` and `"balance_display"`.
    /// Tokens that fail to fetch are returned with `"balance_raw": "0"` so the
    /// caller always gets back the full list.
    pub async fn fetch_token_balances(
        &self,
        chain_id: u32,
        address: String,
        tokens_json: String,
    ) -> Result<String, SpectraBridgeError> {
        #[derive(serde::Deserialize)]
        struct TokenIn {
            contract: String,
            symbol: String,
            decimals: u8,
        }
        #[derive(serde::Serialize)]
        struct TokenOut {
            contract: String,
            symbol: String,
            decimals: u8,
            balance_raw: String,
            balance_display: String,
        }

        let inputs: Vec<TokenIn> = serde_json::from_str(&tokens_json)?;
        if inputs.is_empty() {
            return Ok("[]".to_string());
        }

        let endpoints = self.endpoints_for(chain_id).await;

        let results: Vec<TokenOut> = match chain_id {
            7 => {
                // Tron — TRC-20 tokens, fetched in parallel.
                use futures::future::join_all;
                let client = std::sync::Arc::new(TronClient::new(endpoints));
                let futs: Vec<_> = inputs
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let contract = t.contract.clone();
                        let holder = address.clone();
                        let symbol = t.symbol.clone();
                        let decimals = t.decimals;
                        async move {
                            match client.fetch_trc20_balance(&contract, &holder).await {
                                Ok(b) => TokenOut {
                                    contract,
                                    symbol,
                                    decimals,
                                    balance_raw: b.balance_raw,
                                    balance_display: b.balance_display,
                                },
                                Err(_) => TokenOut {
                                    contract,
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
            2 => {
                // Solana — SPL tokens, batch-fetched via getTokenAccountsByOwner.
                let client = SolanaClient::new(endpoints);
                let mints: Vec<String> = inputs.iter().map(|t| t.contract.clone()).collect();
                let spl = client
                    .fetch_spl_balances(&address, &mints)
                    .await
                    .unwrap_or_default();
                // Build a lookup from mint → SplBalance.
                let by_mint: std::collections::HashMap<&str, &crate::core::chains::solana::SplBalance> =
                    spl.iter().map(|b| (b.mint.as_str(), b)).collect();
                inputs
                    .iter()
                    .map(|t| {
                        let b = by_mint.get(t.contract.as_str());
                        TokenOut {
                            contract: t.contract.clone(),
                            symbol: t.symbol.clone(),
                            decimals: t.decimals,
                            balance_raw: b.map(|b| b.balance_raw.clone()).unwrap_or_else(|| "0".to_string()),
                            balance_display: b.map(|b| b.balance_display.clone()).unwrap_or_else(|| "0".to_string()),
                        }
                    })
                    .collect()
            }
            17 => {
                // NEAR — NEP-141 fungible tokens, fetched in parallel.
                use futures::future::join_all;
                let client = std::sync::Arc::new(NearClient::new(endpoints));
                let futs: Vec<_> = inputs
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
                            TokenOut {
                                contract,
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
            14 => {
                // Sui — per-coin-type balance via suix_getBalance, fetched in parallel.
                use futures::future::join_all;
                let client = std::sync::Arc::new(SuiClient::new(endpoints));
                let futs: Vec<_> = inputs
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
                            TokenOut {
                                contract: coin_type,
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
            15 => {
                // Aptos — per-coin-type balance via 0x1::coin::CoinStore resource,
                // fetched in parallel.
                use futures::future::join_all;
                let client = std::sync::Arc::new(AptosClient::new(endpoints));
                let futs: Vec<_> = inputs
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
                            TokenOut {
                                contract: coin_type,
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
            16 => {
                // TON — jetton balances via TonCenter v3 API.
                // v3 endpoints are registered at chain_id = TON_V3_OFFSET + 16 = 116.
                let v3_endpoints = self.endpoints_for(TON_V3_OFFSET + chain_id).await;
                let api_key = self.api_key_for(chain_id).await;
                let client = TonClient::new(endpoints, api_key).with_v3_endpoints(v3_endpoints);
                let jetton_balances = client
                    .fetch_jetton_balances(&address)
                    .await
                    .unwrap_or_default();

                inputs.iter().map(|t| {
                    // Match on master address (case-insensitive — TON addresses can be
                    // in EQ… or UQ… form; v3 normalises to EQ… but compare leniently).
                    let raw = jetton_balances
                        .iter()
                        .find(|j| j.master_address.eq_ignore_ascii_case(&t.contract))
                        .map(|j| j.balance_raw)
                        .unwrap_or(0u128);
                    let display = format_decimals(raw, t.decimals);
                    TokenOut {
                        contract: t.contract.clone(),
                        symbol: t.symbol.clone(),
                        decimals: t.decimals,
                        balance_raw: raw.to_string(),
                        balance_display: display,
                    }
                }).collect()
            }
            _ => {
                return Err(SpectraBridgeError::from(format!(
                    "fetch_token_balances: unsupported chain_id: {chain_id}"
                )))
            }
        };

        Ok(serde_json::to_string(&results)?)
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
    pub async fn sign_and_send_token(
        &self,
        chain_id: u32,
        params_json: String,
    ) -> Result<String, SpectraBridgeError> {
        let params: serde_json::Value = serde_json::from_str(&params_json)?;
        let endpoints = self.endpoints_for(chain_id).await;

        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
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
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let r = client
                    .sign_and_broadcast_erc20_with_overrides(
                        from, contract, to, amount_raw, &priv_bytes, overrides,
                    )
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&r)?)
            }
            7 => {
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
            8 => {
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
            17 => {
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
            2 => {
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
            _ => Err(SpectraBridgeError::from(format!(
                "sign_and_send_token: unsupported chain_id: {chain_id}"
            ))),
        }
    }

    // ----------------------------------------------------------------
    // Fee estimate
    // ----------------------------------------------------------------

    /// Fetch a fee estimate preview for the given chain.
    ///
    /// BTC and EVM return their chain-specific structs (`FeeRate`,
    /// `EvmFeeEstimate`) for backward compatibility with existing Swift
    /// decoders. All other chains return a unified JSON object:
    ///
    /// ```json
    /// {
    ///   "chain_id": 17,
    ///   "native_fee_raw": "1000000000000000000000",
    ///   "native_fee_display": "0.001",
    ///   "unit": "NEAR",
    ///   "source": "rpc" | "static"
    /// }
    /// ```
    ///
    /// `source` is `"rpc"` when the value comes from a live endpoint call,
    /// and `"static"` when it is a conservative hardcoded default (used for
    /// chains where no preview RPC exists yet).
    pub async fn fetch_fee_estimate(
        &self,
        chain_id: u32,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain_id).await;
        match chain_id {
            0 => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                let fee = client.fetch_fee_rate(6).await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&fee)?)
            }
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let fee = client.fetch_fee_estimate().await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&fee)?)
            }
            2 => {
                // Solana — base fee is a fixed 5000 lamports per signature.
                Ok(fee_preview(chain_id, 5_000, 9, "SOL", "static"))
            }
            4 => {
                // XRP — `fee` command returns drops.
                let client = XrpClient::new(endpoints);
                let drops = client.fetch_fee().await.map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, drops as u128, 6, "XRP", "rpc"))
            }
            7 => {
                // Tron — plain TRX transfers consume bandwidth (free if the
                // account has enough free daily bandwidth). Contract calls
                // burn sun. Show 1 TRX as a conservative default.
                Ok(fee_preview(chain_id, 1_000_000, 6, "TRX", "static"))
            }
            8 => {
                // Stellar — `/fee_stats` mode is the recommended base fee.
                let client = StellarClient::new(endpoints);
                let stroops = client
                    .fetch_base_fee()
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, stroops as u128, 7, "XLM", "rpc"))
            }
            9 => {
                // Cardano — protocol-driven tx-size fee. Typical ADA transfer
                // costs ~0.17 ADA. Lovelace decimals = 6.
                Ok(fee_preview(chain_id, 170_000, 6, "ADA", "static"))
            }
            10 => {
                // Polkadot — standard balance transfer ~0.016 DOT. Planck
                // decimals = 10.
                Ok(fee_preview(chain_id, 160_000_000, 10, "DOT", "static"))
            }
            14 => {
                // Sui — reference gas price is 1000 MIST by default.
                Ok(fee_preview(chain_id, 1_000, 9, "SUI", "static"))
            }
            15 => {
                // Aptos — `estimate_gas_price` returns octas per unit.
                let client = AptosClient::new(endpoints);
                let price = client
                    .fetch_gas_price()
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(fee_preview(chain_id, price as u128, 8, "APT", "rpc"))
            }
            16 => {
                // TON — a plain transfer consumes ~0.007 TON in forward fees.
                // Nanoton decimals = 9.
                Ok(fee_preview(chain_id, 7_000_000, 9, "TON", "static"))
            }
            17 => {
                // NEAR — standard transfer gas costs ~0.001 NEAR. yocto
                // decimals = 24. (Native balance uses a 24-digit decimal.)
                Ok(fee_preview_str(
                    chain_id,
                    "1000000000000000000000",
                    "0.001",
                    "NEAR",
                    "static",
                ))
            }
            18 => {
                // ICP — ledger transfer fee is a fixed 10_000 e8s.
                Ok(fee_preview(chain_id, 10_000, 8, "ICP", "static"))
            }
            19 => {
                // Monero — priority 2 (normal) fee is typically ~0.0005 XMR.
                // Piconero decimals = 12.
                Ok(fee_preview(chain_id, 500_000_000, 12, "XMR", "static"))
            }
            3 => {
                // Dogecoin — 0.01 DOGE (1_000_000 satoshis, decimals = 8).
                Ok(fee_preview(chain_id, 1_000_000, 8, "DOGE", "static"))
            }
            5 => {
                // Litecoin — 0.0001 LTC (10_000 satoshis, decimals = 8).
                Ok(fee_preview(chain_id, 10_000, 8, "LTC", "static"))
            }
            6 => {
                // Bitcoin Cash — 0.00002 BCH (2_000 satoshis, decimals = 8).
                Ok(fee_preview(chain_id, 2_000, 8, "BCH", "static"))
            }
            22 => {
                // Bitcoin SV — 0.00001 BSV (1_000 satoshis, decimals = 8).
                Ok(fee_preview(chain_id, 1_000, 8, "BSV", "static"))
            }
            _ => Ok(json!({"note": "fee estimation not supported for this chain"}).to_string()),
        }
    }

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
    ///
    /// Returns a JSON object `{"xpub": "xpub…"}`.
    pub fn derive_bitcoin_account_xpub(
        &self,
        mnemonic_phrase: String,
        passphrase: String,
        account_path: String,
    ) -> Result<String, SpectraBridgeError> {
        let xpub = crate::core::chains::bitcoin_hd::derive_account_xpub(
            &mnemonic_phrase,
            &passphrase,
            &account_path,
        )
        .map_err(SpectraBridgeError::from)?;
        Ok(json!({ "xpub": xpub }).to_string())
    }

    // ----------------------------------------------------------------
    // Bitcoin HD multi-address (xpub / ypub / zpub)
    // ----------------------------------------------------------------

    /// Derive a contiguous range of child addresses from an account-level
    /// extended public key (xpub/ypub/zpub).
    ///
    /// - `change` — 0 for external/receive, 1 for internal/change.
    /// - `start_index`, `count` — [start, start+count) scan window.
    ///
    /// Returns a JSON array of `{index, change, address}` objects.
    pub async fn derive_bitcoin_hd_addresses(
        &self,
        xpub: String,
        change: u32,
        start_index: u32,
        count: u32,
    ) -> Result<String, SpectraBridgeError> {
        let children =
            crate::core::chains::bitcoin_hd::derive_children(&xpub, change, start_index, count)
                .map_err(SpectraBridgeError::from)?;
        Ok(serde_json::to_string(&children)?)
    }

    /// Scan an xpub's receive + change legs and return aggregated balance
    /// plus per-UTXO breakdown. Uses the Bitcoin Esplora endpoint bundle.
    pub async fn fetch_bitcoin_xpub_balance(
        &self,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(0).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
        let bal = crate::core::chains::bitcoin_hd::fetch_xpub_balance(
            &client,
            &xpub,
            receive_count,
            change_count,
        )
        .await
        .map_err(SpectraBridgeError::from)?;
        Ok(serde_json::to_string(&bal)?)
    }

    /// Return the first address on the `change` leg (0 = receive, 1 = change)
    /// that has zero confirmed/unconfirmed history, scanning up to
    /// `gap_limit` candidates. Returns a JSON object or `null` if exhausted.
    pub async fn fetch_bitcoin_next_unused_address(
        &self,
        xpub: String,
        change: u32,
        gap_limit: u32,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(0).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
        let next = crate::core::chains::bitcoin_hd::fetch_next_unused_address(
            &client,
            &xpub,
            change,
            gap_limit,
        )
        .await
        .map_err(SpectraBridgeError::from)?;
        Ok(serde_json::to_string(&next)?)
    }

    // ----------------------------------------------------------------
    // Price / fiat rate service
    // ----------------------------------------------------------------

    /// Fetch USD spot prices for the supplied coins from `provider`.
    ///
    /// `provider` is the Swift-side display name (e.g. "CoinGecko",
    /// "Binance Public API"). `coins_json` is a JSON array of
    /// `{holdingKey, symbol, coinGeckoId}` objects — Swift hands us
    /// exactly what it currently hands `LivePriceService.fetchQuotes`.
    /// `api_key` is only consulted by CoinGecko; pass "" for others.
    ///
    /// Returns a JSON map keyed by `holdingKey` with USD prices as
    /// numeric values. Missing coins are simply absent from the map.
    pub async fn fetch_prices(
        &self,
        provider: String,
        coins_json: String,
        api_key: String,
    ) -> Result<String, SpectraBridgeError> {
        let coins: Vec<crate::core::price::PriceRequestCoin> =
            serde_json::from_str(&coins_json)?;
        let provider = crate::core::price::PriceProvider::from_str(&provider)
            .ok_or_else(|| format!("unknown price provider: {provider}"))?;
        let quotes = crate::core::price::fetch_prices(provider, &coins, &api_key)
            .await
            .map_err(SpectraBridgeError::from)?;
        Ok(serde_json::to_string(&quotes)?)
    }

    /// Fetch USD-relative fiat rates from `provider`. `currencies_json`
    /// is a JSON array of ISO codes (e.g. `["EUR","JPY"]`). The returned
    /// JSON map always includes `"USD": 1.0`.
    pub async fn fetch_fiat_rates(
        &self,
        provider: String,
        currencies_json: String,
    ) -> Result<String, SpectraBridgeError> {
        let currencies: Vec<String> = serde_json::from_str(&currencies_json)?;
        let provider = crate::core::price::FiatRateProvider::from_str(&provider)
            .ok_or_else(|| format!("unknown fiat rate provider: {provider}"))?;
        let rates = crate::core::price::fetch_fiat_rates(provider, &currencies)
            .await
            .map_err(SpectraBridgeError::from)?;
        Ok(serde_json::to_string(&rates)?)
    }

    // ----------------------------------------------------------------
    // EVM token balances (batch)
    // ----------------------------------------------------------------

    /// Fetch balances for multiple ERC-20 tokens in a single call.
    ///
    /// `tokens_json` is a JSON array of objects:
    /// ```json
    /// [{"contract": "0x...", "symbol": "USDC", "decimals": 6}, ...]
    /// ```
    ///
    /// Returns a JSON array of results:
    /// ```json
    /// [{"contract_address": "0x...", "symbol": "USDC", "balance_display": "10.5",
    ///   "balance_raw": "10500000", "decimals": 6}, ...]
    /// ```
    ///
    /// Tokens with zero balance are **included** in the response so the caller
    /// can detect the difference between "zero balance" and "missing token".
    pub async fn fetch_evm_token_balances_batch(
        &self,
        chain_id: u32,
        address: String,
        tokens_json: String,
    ) -> Result<String, SpectraBridgeError> {
        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {}
            _ => return Err(SpectraBridgeError::from(format!(
                "fetch_evm_token_balances_batch: unsupported chain_id: {chain_id}"
            ))),
        }
        let tokens: Vec<serde_json::Value> = serde_json::from_str(&tokens_json)?;
        let eps = self.endpoints_for(chain_id).await;
        let client = EvmClient::new(eps, evm_chain_id_for(chain_id));

        let mut results = Vec::with_capacity(tokens.len());
        for token in &tokens {
            let contract = token["contract"].as_str().unwrap_or("").to_lowercase();
            let symbol = token["symbol"].as_str().unwrap_or("?").to_string();
            let decimals = token["decimals"].as_u64().unwrap_or(18) as u8;
            if contract.is_empty() {
                continue;
            }
            let raw = client.fetch_erc20_balance_of(&contract, &address)
                .await
                .unwrap_or(0);
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
            results.push(json!({
                "contract_address": contract,
                "symbol": symbol,
                "balance_raw": raw.to_string(),
                "balance_display": balance_display,
                "decimals": decimals,
            }));
        }
        Ok(serde_json::to_string(&results)?)
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
    /// `tokens_json` is `[{"contract":"0x…","symbol":"USDC","name":"USD Coin","decimals":6},…]`.
    /// Only token transfers whose contract matches a tracked token are included in the result.
    /// Pass `"[]"` to skip token transfers.
    ///
    /// Returns `{"native": […], "tokens": […]}`.
    pub async fn fetch_evm_history_page(
        &self,
        chain_id: u32,
        address: String,
        tokens_json: String,
        page: u32,
        page_size: u32,
    ) -> Result<String, SpectraBridgeError> {
        use crate::core::chains::evm::EvmTokenTransferEntry;

        // Only Etherscan-indexed EVM chains are supported.
        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {}
            _ => return Err(SpectraBridgeError::from(format!(
                "fetch_evm_history_page: chain_id {chain_id} not supported"
            ))),
        }

        #[derive(serde::Deserialize)]
        struct TrackedToken {
            contract: String,
            symbol: String,
            name: String,
            decimals: u8,
        }
        let tracked: Vec<TrackedToken> = serde_json::from_str(&tokens_json)?;

        let eps = self.endpoints_for(chain_id).await;
        let explorer_eps = self.endpoints_for(EXPLORER_OFFSET + chain_id).await;
        let api_key = self.api_key_for(chain_id).await;
        let api_key_str = api_key.as_deref();

        let explorer_base = explorer_eps.into_iter().next().unwrap_or_default();
        let evm_chain_id = evm_chain_id_for(chain_id);
        let client = EvmClient::new(eps, evm_chain_id);

        // The Etherscan v2 `chainid` param distinguishes chains on the unified endpoint.
        // For chain-specific Etherscan deployments (ARB, OP, etc.) it is optional but harmless.
        let etherscan_chain_id: Option<u64> = match chain_id {
            1  => Some(1),
            11 => Some(42161),
            12 => Some(10),
            13 => Some(43114),
            20 => Some(8453),
            21 => Some(61),
            23 => Some(56),
            _  => None,
        };

        // Fetch native and token transfers concurrently.
        let (native_result, token_result) = tokio::join!(
            client.fetch_history(&address, &explorer_base, api_key_str),
            client.fetch_token_transfers(
                &address,
                &explorer_base,
                api_key_str,
                etherscan_chain_id,
                page,
                page_size,
            )
        );

        let native = native_result.unwrap_or_default();
        let raw_tokens = token_result.unwrap_or_default();

        // Build a lookup map from contract address (lowercased) → tracked token metadata.
        let addr_lower = address.to_lowercase();
        let token_map: std::collections::HashMap<String, (&str, &str, u8)> = tracked
            .iter()
            .map(|t| (t.contract.to_lowercase(), (t.symbol.as_str(), t.name.as_str(), t.decimals)))
            .collect();

        // Filter to tracked tokens only and reformat amount using tracked decimals.
        let tokens: Vec<EvmTokenTransferEntry> = raw_tokens
            .into_iter()
            .filter_map(|mut entry| {
                let key = entry.contract.to_lowercase();
                if let Some(&(sym, name, dec)) = token_map.get(&key) {
                    // Use the tracked decimals (more reliable than Etherscan's tokenDecimal).
                    entry.symbol = sym.to_string();
                    entry.token_name = name.to_string();
                    if dec != entry.decimals {
                        entry.decimals = dec;
                        entry.amount_display = crate::core::chains::evm::format_evm_decimals(&entry.amount_raw, dec);
                    }
                    // Only include transfers involving this wallet address.
                    if entry.from == addr_lower || entry.to == addr_lower {
                        return Some(entry);
                    }
                }
                None
            })
            .collect();

        Ok(json!({ "native": native, "tokens": tokens }).to_string())
    }

    // ----------------------------------------------------------------
    // ENS resolution
    // ----------------------------------------------------------------

    /// Resolve an ENS name to an Ethereum address via the ENS Ideas public API.
    ///
    /// Returns `{"address": "0x…"}` when resolved, `{"address": ""}` when the
    /// name has no registered address, or an error on network failure.
    pub async fn resolve_ens_name(
        &self,
        name: String,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for(1).await; // ENS is Ethereum mainnet
        let client = EvmClient::new(eps, 1);
        let address = client
            .resolve_ens(&name)
            .await
            .map_err(SpectraBridgeError::from)?;
        Ok(json!({ "address": address.unwrap_or_default() }).to_string())
    }

    // ----------------------------------------------------------------
    // EVM utilities (contract detection, nonce lookup)
    // ----------------------------------------------------------------

    /// Fetch the bytecode at `address` on the given EVM chain.
    /// Returns `{"code": "0x…"}`. "0x" / "0x0" means EOA (no contract code).
    pub async fn fetch_evm_code(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for(chain_id).await;
        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let client = EvmClient::new(eps, evm_chain_id_for(chain_id));
                let code = client.fetch_code(&address).await.map_err(SpectraBridgeError::from)?;
                Ok(json!({ "code": code }).to_string())
            }
            _ => Err(SpectraBridgeError::from(format!(
                "fetch_evm_code: unsupported chain_id: {chain_id}"
            ))),
        }
    }

    /// Fetch the nonce of a submitted transaction by hash on an EVM chain.
    /// Returns `{"nonce": <u64>}`. Used to pre-fill the replacement-tx nonce field.
    pub async fn fetch_evm_tx_nonce(
        &self,
        chain_id: u32,
        tx_hash: String,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for(chain_id).await;
        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let client = EvmClient::new(eps, evm_chain_id_for(chain_id));
                let nonce = client.fetch_tx_nonce(&tx_hash).await.map_err(SpectraBridgeError::from)?;
                Ok(json!({ "nonce": nonce }).to_string())
            }
            _ => Err(SpectraBridgeError::from(format!(
                "fetch_evm_tx_nonce: unsupported chain_id: {chain_id}"
            ))),
        }
    }

    // ----------------------------------------------------------------
    // UTXO fee preview (LTC / BCH / BSV)
    // ----------------------------------------------------------------

    /// Compute a fee-capacity preview for a UTXO-based chain.
    ///
    /// Fetches UTXOs for `address`, estimates the fee for a max-send transaction
    /// (no change output), and returns a JSON object:
    ///
    /// ```json
    /// {
    ///   "fee_rate_svb":          10,
    ///   "estimated_fee_sat":     1480,
    ///   "estimated_tx_bytes":    148,
    ///   "selected_input_count":  1,
    ///   "uses_change_output":    false,
    ///   "spendable_balance_sat": 1000000,
    ///   "max_sendable_sat":      998520
    /// }
    /// ```
    ///
    /// Pass `fee_rate_svb = 0` to let Rust fetch a live rate from the chain's
    /// Blockbook endpoint (falls back to 1 sat/vB). Otherwise the caller-
    /// supplied rate is used directly.
    ///
    /// Supported chain IDs: 0 (BTC), 5 (LTC), 6 (BCH), 22 (BSV).
    pub async fn fetch_utxo_fee_preview(
        &self,
        chain_id: u32,
        address: String,
        fee_rate_svb: u64,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for(chain_id).await;
        match chain_id {
            0 => {
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
            3 => {
                let client = DogecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_koin).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            5 => {
                let client = LitecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { client.fetch_fee_rate(3).await };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            6 => {
                let client = BitcoinCashClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { client.fetch_fee_rate(3).await };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            22 => {
                let client = BitcoinSvClient::new(eps);
                let utxos = client.fetch_utxos(&address).await.map_err(SpectraBridgeError::from)?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            _ => Err(SpectraBridgeError::from(format!(
                "fetch_utxo_fee_preview: unsupported chain_id: {chain_id}"
            ))),
        }
    }

    // ----------------------------------------------------------------
    // Rebroadcast
    // ----------------------------------------------------------------

    /// Rebroadcast a previously signed transaction.
    ///
    /// `chain_id` is the Spectra chain ID. `payload` is the chain-specific
    /// raw payload:
    ///   - BTC/LTC/BCH/BSV/DOGE/EVM: raw transaction hex string
    ///   - Solana: base64-encoded signed transaction bytes
    ///   - Tron: signed transaction JSON string (full broadcasttransaction body)
    pub async fn broadcast_raw(
        &self,
        chain_id: u32,
        payload: String,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for(chain_id).await;
        match chain_id {
            // Bitcoin
            0 => {
                let client = BitcoinClient::new(HttpClient::shared(), eps, "mainnet");
                let txid = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(json!({ "txid": txid }).to_string())
            }
            // Dogecoin
            3 => {
                let client = DogecoinClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // Litecoin
            5 => {
                let client = LitecoinClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // Bitcoin Cash
            6 => {
                let client = BitcoinCashClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // Bitcoin SV
            22 => {
                let client = BitcoinSvClient::new(eps);
                let res = client
                    .broadcast_raw_tx(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // Solana
            2 => {
                let client = SolanaClient::new(eps);
                let res = client
                    .broadcast_raw(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // Tron
            7 => {
                let client = TronClient::new(eps);
                let res = client
                    .broadcast_raw(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // EVM chains (ETH, ARB, OP, AVAX, BASE, ETC, BSC, HYPE)
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {
                let evm_id = evm_chain_id_for(chain_id);
                let client = EvmClient::new(eps, evm_id);
                let res = client
                    .broadcast_raw(&payload)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // XRP — payload is `tx_blob_hex` field from the stored rust_json
            4 => {
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
            // Stellar — payload is `signed_xdr_b64` field
            8 => {
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
            // Cardano — payload is `cbor_hex` field
            9 => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let cbor = val["cbor_hex"].as_str()
                    .ok_or("broadcast_raw cardano: missing cbor_hex")?
                    .to_string();
                let api_key = self.api_key_for(chain_id).await.unwrap_or_default();
                let client = CardanoClient::new(eps, api_key);
                let res = client
                    .submit_tx(&cbor)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // Polkadot — payload is `extrinsic_hex` field
            10 => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let ext_hex = val["extrinsic_hex"].as_str()
                    .ok_or("broadcast_raw polkadot: missing extrinsic_hex")?
                    .to_string();
                let subscan = self.endpoints_for(SUBSCAN_OFFSET + chain_id).await;
                let api_key = self.api_key_for(chain_id).await;
                let client = PolkadotClient::new(eps, subscan, api_key);
                let res = client
                    .submit_extrinsic_hex(&ext_hex)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // Sui — payload contains `tx_bytes_b64` and `sig_b64` fields
            14 => {
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
            // Aptos — payload is `signed_body_json` field
            15 => {
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
            // TON — payload is `boc_b64` field
            16 => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let boc = val["boc_b64"].as_str()
                    .ok_or("broadcast_raw ton: missing boc_b64")?
                    .to_string();
                let api_key = self.api_key_for(chain_id).await;
                let client = TonClient::new(eps, api_key);
                let res = client
                    .send_boc(&boc)
                    .await
                    .map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&res)?)
            }
            // NEAR — payload is `signed_tx_b64` field
            17 => {
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
            // ICP — rebroadcast not supported (Rosetta multi-step flow)
            18 => Err(SpectraBridgeError::from(
                "ICP rebroadcast is not supported".to_string()
            )),
            _ => Err(SpectraBridgeError::from(format!(
                "broadcast_raw: chain {chain_id} not supported"
            ))),
        }
    }

    // ----------------------------------------------------------------
    // EVM receipt polling
    // ----------------------------------------------------------------

    /// Fetch a transaction receipt by hash on an EVM chain.
    ///
    /// Returns the receipt JSON when the transaction has been mined, or
    /// `"null"` (JSON null as a string) when it is still pending.
    pub async fn fetch_evm_receipt(
        &self,
        chain_id: u32,
        tx_hash: String,
    ) -> Result<String, SpectraBridgeError> {
        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {}
            _ => return Err(SpectraBridgeError::from(format!(
                "fetch_evm_receipt: unsupported chain_id: {chain_id}"
            ))),
        }
        let eps = self.endpoints_for(chain_id).await;
        let client = EvmClient::new(eps, evm_chain_id_for(chain_id));
        match client.fetch_receipt(&tx_hash).await.map_err(SpectraBridgeError::from)? {
            Some(receipt) => Ok(serde_json::to_string(&receipt)?),
            None => Ok("null".to_string()),
        }
    }

    // ----------------------------------------------------------------
    // EVM send preview (nonce + fee + gas + balance in one call)
    // ----------------------------------------------------------------

    /// Compute an EVM send preview: fetches nonce, EIP-1559 fees, gas
    /// estimate, and native balance concurrently, then returns a single
    /// JSON bundle that Swift can map directly to `EthereumSendPreview`.
    ///
    /// `value_wei` is the native amount as a decimal string (`"0"` for token
    /// sends). `data_hex` is the ABI-encoded call data (`"0x"` for native
    /// transfers, e.g. `"0xa9059cbb…"` for ERC-20 transfers).
    ///
    /// ```json
    /// {
    ///   "nonce": 42,
    ///   "gas_limit": 65000,
    ///   "max_fee_per_gas_gwei": 15.5,
    ///   "max_priority_fee_per_gas_gwei": 1.0,
    ///   "estimated_fee_eth": 0.00101,
    ///   "balance_eth": 0.5,
    ///   "spendable_eth": 0.49899,
    ///   "fee_rate_description": "Max 15.50 gwei / Priority 1.00 gwei"
    /// }
    /// ```
    pub async fn fetch_evm_send_preview(
        &self,
        chain_id: u32,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
    ) -> Result<String, SpectraBridgeError> {
        match chain_id {
            1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => {}
            _ => return Err(SpectraBridgeError::from(format!(
                "fetch_evm_send_preview: unsupported chain_id: {chain_id}"
            ))),
        }
        let eps = self.endpoints_for(chain_id).await;
        let evm_id = evm_chain_id_for(chain_id);
        let client = EvmClient::new(eps, evm_id);

        let value_u128: u128 = value_wei.parse().unwrap_or(0);
        let data_opt: Option<&str> = if data_hex == "0x" || data_hex.is_empty() {
            None
        } else {
            Some(&data_hex)
        };

        // Run the four independent RPC calls in parallel.
        let (nonce_res, fee_res, gas_res, bal_res) = tokio::join!(
            client.fetch_nonce(&from),
            client.fetch_fee_estimate(),
            client.estimate_gas(&from, &to, value_u128, data_opt),
            client.fetch_balance(&from)
        );

        let nonce = nonce_res.unwrap_or(0);
        let fee = fee_res.unwrap_or(crate::core::chains::evm::EvmFeeEstimate {
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

    // ----------------------------------------------------------------
    // Tron send preview (fee + balance for TRX / TRC-20)
    // ----------------------------------------------------------------

    /// Compute a Tron send preview for TRX or TRC-20 sends.
    ///
    /// For TRX native sends: fetches the TRX balance and applies a conservative
    /// static 1 TRX fee (typical plain-transfer bandwidth cost).
    ///
    /// For TRC-20 sends: additionally fetches the token balance via `fetch_trc20_balance`.
    /// Uses a static ~15 TRX fee (typical USDT transfer energy cost).
    ///
    /// Returns:
    /// ```json
    /// {
    ///   "estimated_fee_trx": 1.0,
    ///   "fee_limit_sun": 0,
    ///   "spendable_balance": 99.0,
    ///   "max_sendable": 99.0,
    ///   "fee_rate_description": "Static bandwidth estimate"
    /// }
    /// ```
    pub async fn fetch_tron_send_preview(
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

        // TRC-20 token: fetch token balance alongside TRX (for fee headroom).
        let token_balance = client.fetch_trc20_balance_of(&contract_address, &address).await
            .map(|raw| raw as f64 / 1_000_000.0)  // default 6 decimals
            .unwrap_or(0.0);

        // Conservative static fee: ~15 TRX covers typical USDT energy cost.
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

    /// Persist the full wallet-list snapshot as a JSON string. Swift calls this
    /// whenever the wallet list changes so Rust/SQLite is always up-to-date.
    /// Key is fixed ("wallets.snapshot.v1") so old values are overwritten.
    pub async fn save_wallet_snapshot(
        &self,
        db_path: String,
        snapshot_json: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            sqlite_save(&db_path, "wallets.snapshot.v1", &snapshot_json)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Load the wallet-list snapshot. Returns `"[]"` if no snapshot has been
    /// saved yet (first launch or after a reset).
    pub async fn load_wallet_snapshot(
        &self,
        db_path: String,
    ) -> Result<String, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            match sqlite_load(&db_path, "wallets.snapshot.v1") {
                Ok(v) if v == "{}" => Ok("[]".to_string()), // empty-object sentinel → empty array
                other => other,
            }
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Persist arbitrary app settings as a JSON blob. Separate from the wallet
    /// snapshot so each can be saved independently.
    pub async fn save_app_settings(
        &self,
        db_path: String,
        settings_json: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            sqlite_save(&db_path, "app.settings.v1", &settings_json)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Load app settings. Returns `"{}"` if not yet saved.
    pub async fn load_app_settings(
        &self,
        db_path: String,
    ) -> Result<String, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            sqlite_load(&db_path, "app.settings.v1")
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

    /// Persist keypool state for one (wallet_id, chain_name) pair.
    /// `state_json` encodes `KeypoolState` (nextExternalIndex, nextChangeIndex, reservedReceiveIndex).
    pub async fn save_keypool_state(
        &self,
        db_path: String,
        wallet_id: String,
        chain_name: String,
        state_json: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let state: crate::core::wallet_db::KeypoolState =
                serde_json::from_str(&state_json)
                    .map_err(|e| format!("save_keypool_state parse: {e}"))?;
            crate::core::wallet_db::keypool_save(&db_path, &wallet_id, &chain_name, &state)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Load keypool state for one (wallet_id, chain_name). Returns `null` JSON if not found.
    pub async fn load_keypool_state(
        &self,
        db_path: String,
        wallet_id: String,
        chain_name: String,
    ) -> Result<Option<String>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let state = crate::core::wallet_db::keypool_load(&db_path, &wallet_id, &chain_name)?;
            match state {
                Some(s) => serde_json::to_string(&s)
                    .map(Some)
                    .map_err(|e| format!("load_keypool_state serialize: {e}")),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Bulk-load ALL keypool state across all wallets and chains.
    /// Returns a JSON object: `{ "Bitcoin": { "<uuid>": { state }, … }, … }`.
    /// Used at app startup to restore the in-memory keypool dictionaries.
    pub async fn load_all_keypool_state(
        &self,
        db_path: String,
    ) -> Result<String, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let all = crate::core::wallet_db::keypool_load_all(&db_path)?;
            serde_json::to_string(&all).map_err(|e| format!("load_all_keypool_state serialize: {e}"))
        })
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
            crate::core::wallet_db::keypool_delete_for_wallet(&db_path, &wallet_id)
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
            crate::core::wallet_db::keypool_delete_for_chain(&db_path, &chain_name)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Upsert a single owned address record.
    /// `record_json` encodes `OwnedAddressRecord` (walletId, chainName, address, derivationPath, branch, branchIndex).
    pub async fn save_owned_address(
        &self,
        db_path: String,
        record_json: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let record: crate::core::wallet_db::OwnedAddressRecord =
                serde_json::from_str(&record_json)
                    .map_err(|e| format!("save_owned_address parse: {e}"))?;
            crate::core::wallet_db::address_save(&db_path, &record)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Load all owned addresses for a (wallet, chain) pair.
    /// Returns a JSON array of `OwnedAddressRecord` objects.
    pub async fn load_owned_addresses(
        &self,
        db_path: String,
        wallet_id: String,
        chain_name: String,
    ) -> Result<String, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let records = crate::core::wallet_db::address_load_all(&db_path, &wallet_id, &chain_name)?;
            serde_json::to_string(&records)
                .map_err(|e| format!("load_owned_addresses serialize: {e}"))
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Bulk-load ALL owned address records across all wallets and chains.
    /// Returns a JSON array; used at app startup to restore the in-memory address maps.
    pub async fn load_all_owned_addresses(
        &self,
        db_path: String,
    ) -> Result<String, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let records = crate::core::wallet_db::address_load_all_chains(&db_path)?;
            serde_json::to_string(&records)
                .map_err(|e| format!("load_all_owned_addresses serialize: {e}"))
        })
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
            crate::core::wallet_db::address_delete_for_wallet(&db_path, &wallet_id)
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
            crate::core::wallet_db::address_delete_for_chain(&db_path, &chain_name)
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
            crate::core::wallet_db::delete_wallet_data(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    // ----------------------------------------------------------------
    // Phase 2.2 — Balance cache
    // ----------------------------------------------------------------

    /// Return the cached balance JSON for `(chain_id, address)` if present and
    /// not expired. Returns `null` as JSON when the cache is cold.
    pub fn cached_balance(&self, chain_id: u32, address: String) -> Option<String> {
        self.balance_cache.get(chain_id, &address)
    }

    /// Store a balance JSON snapshot in the cache.
    pub fn cache_balance(&self, chain_id: u32, address: String, balance_json: String) {
        self.balance_cache.set(chain_id, &address, balance_json);
    }

    /// Evict a specific cached balance (call after a send completes).
    pub fn invalidate_cached_balance(&self, chain_id: u32, address: String) {
        self.balance_cache.invalidate(chain_id, &address);
    }

    /// Evict all expired entries. Cheap to call on any balance-refresh tick.
    pub fn evict_expired_balance_cache(&self) {
        self.balance_cache.evict_expired();
    }

    /// Fetch the balance, returning the cached value if still fresh, otherwise
    /// fetching from the chain and caching the result.
    pub async fn fetch_balance_cached(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        if let Some(cached) = self.balance_cache.get(chain_id, &address) {
            return Ok(cached);
        }
        let fresh = self.fetch_balance(chain_id, address.clone()).await?;
        self.balance_cache.set(chain_id, &address, fresh.clone());
        Ok(fresh)
    }

    // ----------------------------------------------------------------
    // Phase 2.3 — History cache
    // ----------------------------------------------------------------

    /// Return cached history JSON for `(chain_id, address)` if present and not expired.
    pub fn cached_history(&self, chain_id: u32, address: String) -> Option<String> {
        self.history_cache.get(chain_id, &address)
    }

    /// Store a history JSON snapshot in the in-memory cache.
    pub fn cache_history(&self, chain_id: u32, address: String, history_json: String) {
        self.history_cache.set(chain_id, &address, history_json);
    }

    /// Evict the history cache entry for `(chain_id, address)`.
    pub fn invalidate_cached_history(&self, chain_id: u32, address: String) {
        self.history_cache.invalidate(chain_id, &address);
    }

    /// Fetch history from the chain, returning the cached value if still fresh.
    pub async fn fetch_history_cached(
        &self,
        chain_id: u32,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        if let Some(cached) = self.history_cache.get(chain_id, &address) {
            return Ok(cached);
        }
        let fresh = self.fetch_history(chain_id, address.clone()).await?;
        self.history_cache.set(chain_id, &address, fresh.clone());
        Ok(fresh)
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

    /// Register the Swift Keychain implementation. Must be called once at app
    /// start before any method that reads or writes secrets.
    pub fn set_secret_store(&self, store: Arc<dyn SecretStore>) {
        if let Ok(mut guard) = self.secret_store.write() {
            *guard = Some(store);
        }
    }

    /// Read a secret via the registered `SecretStore`.
    pub fn load_secret(&self, key: String) -> Option<String> {
        let guard = self.secret_store.read().ok()?;
        guard.as_ref()?.load_secret(key)
    }

    /// Write a secret via the registered `SecretStore`.
    pub fn save_secret(&self, key: String, value: String) -> bool {
        let guard = self.secret_store.read().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(s) => s.save_secret(key, value),
            None => false,
        }
    }

    /// Delete a secret via the registered `SecretStore`.
    pub fn delete_secret(&self, key: String) -> bool {
        let guard = self.secret_store.read().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(s) => s.delete_secret(key),
            None => false,
        }
    }

    /// List all secret keys matching a prefix via the registered `SecretStore`.
    pub fn list_secret_keys(&self, prefix_filter: String) -> Vec<String> {
        let guard = self.secret_store.read().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(s) => s.list_keys(prefix_filter),
            None => vec![],
        }
    }

    // ----------------------------------------------------------------
    // Token catalog
    // ----------------------------------------------------------------

    /// Return the built-in token catalog for the given chain as a JSON array.
    /// Pass `chain_id = 4294967295` (u32::MAX) to return all chains.
    pub async fn list_builtin_tokens(
        &self,
        chain_id: u32,
    ) -> Result<String, SpectraBridgeError> {
        Ok(tokens::list_tokens_json(chain_id))
    }

    // ----------------------------------------------------------------
    // UTXO tx status
    // ----------------------------------------------------------------

    /// Fetch confirmation status for a UTXO chain transaction.
    /// Returns JSON `{"txid","confirmed","block_height","block_time"}`.
    /// Supported chain_ids: 0 (BTC), 3 (DOGE), 5 (LTC), 6 (BCH), 22 (BSV).
    pub async fn fetch_utxo_tx_status(
        &self,
        chain_id: u32,
        txid: String,
    ) -> Result<String, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain_id).await;
        let status: UtxoTxStatus = match chain_id {
            0 => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints, "mainnet");
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            3 => {
                let client = DogecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            5 => {
                let client = LitecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            6 => {
                let client = BitcoinCashClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            22 => {
                let client = BitcoinSvClient::new(endpoints);
                client.fetch_tx_status(&txid).await.map_err(SpectraBridgeError::from)?
            }
            _ => return Err(SpectraBridgeError::from(format!(
                "fetch_utxo_tx_status: unsupported chain_id: {chain_id}"
            ))),
        };
        Ok(serde_json::to_string(&status)?)
    }
}

// ----------------------------------------------------------------
// Token catalog (synchronous free function — no network I/O)
// ----------------------------------------------------------------

/// Return the built-in token catalog as a JSON array string.
/// Pass `chain_id = 4294967295` (u32::MAX) to get all chains.
/// This is a synchronous free function so Swift can call it from a `static let`.
#[uniffi::export]
pub fn list_builtin_tokens_json(chain_id: u32) -> String {
    tokens::list_tokens_json(chain_id)
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
    use bip39::{Language};
    Language::English.word_list().join("\n")
}

// ----------------------------------------------------------------
// Internal helpers (not exported)
// ----------------------------------------------------------------

/// Logical offset for Subscan / secondary endpoint bundles.
const SUBSCAN_OFFSET: u32 = 100;
/// Logical offset for IC endpoint bundles.
const IC_OFFSET: u32 = 100;
/// Logical offset for TON v3 (TonCenter) endpoint bundles.
const TON_V3_OFFSET: u32 = 100;
/// Logical offset for explorer (Etherscan-compatible) endpoint bundles.
const EXPLORER_OFFSET: u32 = 200;

impl WalletService {
    async fn endpoints_for(&self, chain_id: u32) -> Vec<String> {
        let guard = self.endpoints.read().await;
        guard
            .iter()
            .find(|e| e.chain_id == chain_id)
            .map(|e| e.endpoints.clone())
            .unwrap_or_default()
    }

    async fn api_key_for(&self, chain_id: u32) -> Option<String> {
        let guard = self.endpoints.read().await;
        guard
            .iter()
            .find(|e| e.chain_id == chain_id)
            .and_then(|e| e.api_key.clone())
    }

}

// ----------------------------------------------------------------
// EVM chain ID mapping
// ----------------------------------------------------------------

fn evm_chain_id_for(spectra_chain_id: u32) -> u64 {
    match spectra_chain_id {
        1 => 1,
        11 => 42161,
        12 => 10,
        13 => 43114,
        20 => 8453,
        21 => 61,
        23 => 56,
        24 => 999,
        _ => 1,
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

/// Build a `fee_preview` JSON string from an integer raw amount plus
/// decimals. Scales the raw amount down for a human-readable display field.
fn fee_preview(chain_id: u32, raw: u128, decimals: u8, unit: &str, source: &str) -> String {
    let display = format_decimals(raw, decimals);
    serde_json::json!({
        "chain_id": chain_id,
        "native_fee_raw": raw.to_string(),
        "native_fee_display": display,
        "unit": unit,
        "source": source,
    })
    .to_string()
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
    serde_json::json!({
        "chain_id": chain_id,
        "native_fee_raw": raw,
        "native_fee_display": display,
        "unit": unit,
        "source": source,
    })
    .to_string()
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
fn read_evm_overrides(params: &serde_json::Value) -> crate::core::chains::evm::EvmSendOverrides {
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
    crate::core::chains::evm::EvmSendOverrides {
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

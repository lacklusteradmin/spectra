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

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::chains::{
    aptos::AptosClient,
    bitcoin::{BitcoinClient, BitcoinSendParams, sign_and_broadcast as bitcoin_sign_and_broadcast},
    bitcoin_cash::BitcoinCashClient,
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
use crate::core::http::HttpClient;
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
}

#[uniffi::export]
impl WalletService {
    #[uniffi::constructor]
    pub fn new(endpoints_json: String) -> Result<Arc<Self>, SpectraBridgeError> {
        let endpoints: Vec<ChainEndpoints> = serde_json::from_str(&endpoints_json)?;
        Ok(Arc::new(Self {
            endpoints: Arc::new(RwLock::new(endpoints)),
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
            1 | 11 | 12 | 13 | 20 | 21 => {
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
            _ => Err(SpectraBridgeError::from(format!("unknown chain_id: {chain_id}"))),
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
            1 | 11 | 12 | 13 | 20 | 21 => {
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
                let h = client.fetch_history(&address, 50).await.map_err(SpectraBridgeError::from)?;
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
            5 | 6 => Ok(json!([]).to_string()),
            7 => {
                let client = TronClient::new(endpoints);
                let tronscan = self.endpoints_for(EXPLORER_OFFSET + chain_id).await
                    .into_iter().next()
                    .unwrap_or_else(|| "https://apilist.tronscan.org".to_string());
                let h = client.fetch_history(&address, &tronscan)
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
            1 | 11 | 12 | 13 | 20 | 21 => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let value_wei: u128 = params["value_wei"].as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("missing value_wei")?;
                let priv_bytes = hex_field(&params, "private_key_hex")?;
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let r = client.sign_and_broadcast(from, to, value_wei, &priv_bytes)
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
                let pub_hex = str_field(&params, "public_key_hex")?;
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
            8 => {
                let from = str_field(&params, "from")?;
                let to = str_field(&params, "to")?;
                let stroops = params["stroops"].as_i64().ok_or("missing stroops")?;
                let priv_arr: [u8; 64] = hex_field(&params, "private_key_hex")?
                    .try_into().map_err(|_| "privkey wrong length")?;
                let pub_arr: [u8; 32] = hex_field(&params, "public_key_hex")?
                    .try_into().map_err(|_| "pubkey wrong length")?;
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
                let pub_bytes = hex_field(&params, "public_key_hex")?;
                let ic_endpoints = self.endpoints_for(IC_OFFSET + chain_id).await;
                let client = IcpClient::new(endpoints, ic_endpoints);
                let r = client.sign_and_submit(from, to, e8s, &priv_bytes, &pub_bytes)
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
    // Fee estimate
    // ----------------------------------------------------------------

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
            1 | 11 | 12 | 13 | 20 | 21 => {
                let client = EvmClient::new(endpoints, evm_chain_id_for(chain_id));
                let fee = client.fetch_fee_estimate().await.map_err(SpectraBridgeError::from)?;
                Ok(serde_json::to_string(&fee)?)
            }
            _ => Ok(json!({"note": "fee estimation not supported for this chain"}).to_string()),
        }
    }
}

// ----------------------------------------------------------------
// Internal helpers (not exported)
// ----------------------------------------------------------------

/// Logical offset for Subscan / secondary endpoint bundles.
const SUBSCAN_OFFSET: u32 = 100;
/// Logical offset for IC endpoint bundles.
const IC_OFFSET: u32 = 100;
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

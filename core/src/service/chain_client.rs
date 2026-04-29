//! Per-chain client dispatch — collapses the 17-arm `match chain` blocks that
//! were repeated across `fetch_balance`, `fetch_history`, and
//! `fetch_native_balance_summary` into one place.
//!
//! Construction lives in `ChainClient::build`. Each method on the enum
//! dispatches via `match self`. New chains add one variant + one arm per
//! method, instead of editing 3+ separate match tables in `service/mod.rs`.

use super::helpers::{format_smallest_unit_decimal, json_response};
use super::{NativeBalanceSummary, WalletService};
use crate::fetch::chains::{
    aptos::AptosClient, bitcoin::BitcoinClient, bitcoin_cash::BitcoinCashClient,
    bitcoin_gold::BitcoinGoldClient, bitcoin_sv::BitcoinSvClient, bittensor::BittensorClient,
    cardano::CardanoClient, dash::DashClient, decred::DecredClient, dogecoin::DogecoinClient,
    evm::EvmClient, icp::IcpClient, kaspa::KaspaClient, litecoin::LitecoinClient,
    monero::MoneroClient, near::NearClient, polkadot::PolkadotClient, solana::SolanaClient,
    stellar::StellarClient, sui::SuiClient, ton::TonClient, tron::TronClient, xrp::XrpClient,
    zcash::ZcashClient,
};
use crate::http::HttpClient;
use crate::registry::{Chain, EndpointSlot};
use crate::SpectraBridgeError;

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


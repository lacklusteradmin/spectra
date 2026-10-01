//! The UTXO family's client: one set of answers over every indexer API a
//! chain speaks — Esplora, Blockbook, BlockCypher, WhatsOnChain and BCH REST.
//!
//! Every request goes to every endpoint at once, whatever its API, and the
//! first success answers. For that to be the same answer whichever endpoint
//! wins, every adapter reports in the types below and with one meaning: a
//! balance is the confirmed balance, a UTXO list includes the mempool's
//! outputs, and history is newest first.

use crate::api::error::ApiError;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::api::http::{HttpClient, first_success};
use crate::api::{
    bch_rest_v2::BchRestClient, blockbook::BlockbookClient, blockcypher::BlockcypherClient,
    esplora::EsploraClient, whatsonchain::WhatsonchainClient,
};
use crate::registry::Chain;
use crate::{Endpoint, EndpointApi};

// ── Answers

/// An unspent output. The field names are Esplora's, which answers in this
/// shape directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub status: UtxoStatus,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoStatus {
    pub confirmed: bool,
    pub block_height: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoBalance {
    /// Confirmed balance in the coin's smallest unit.
    pub confirmed_sats: u64,
    /// The mempool's net change to it; negative while a spend is pending.
    pub unconfirmed_sats: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoHistoryEntry {
    pub txid: String,
    pub confirmed: bool,
    pub block_height: Option<u64>,
    pub block_time: Option<u64>,
    /// Net change for the watched address (positive = received, negative =
    /// sent).
    pub net_sats: i64,
    pub fee_sats: Option<u64>,
}

/// Satoshis (or the coin's smallest unit) per virtual byte.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FeeRate {
    pub sats_per_vbyte: f64,
}

/// A transaction's confirmation status.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct UtxoTxStatus {
    pub txid: String,
    pub confirmed: bool,
    pub block_height: Option<u64>,
    pub block_time: Option<u64>,
    /// Number of confirmations, where the indexer reports it.
    pub confirmations: Option<u64>,
}

// ── Client

pub struct UtxoClient {
    chain: Chain,
    endpoints: Vec<Endpoint>,
}

/// One endpoint, behind the adapter for its API.
enum Adapter {
    Esplora(EsploraClient),
    Blockbook(BlockbookClient),
    Blockcypher(BlockcypherClient),
    Whatsonchain(WhatsonchainClient),
    BchRest(BchRestClient),
}

impl UtxoClient {
    pub fn new(chain: Chain, endpoints: Vec<Endpoint>) -> Self {
        Self { chain, endpoints }
    }

    fn adapter(&self, endpoint: &Endpoint) -> Result<Adapter, ApiError> {
        let url = Arc::new(vec![endpoint.url.clone()]);
        Ok(match endpoint.api {
            EndpointApi::Esplora => Adapter::Esplora(EsploraClient::new(HttpClient::shared(), url)),
            EndpointApi::Blockbook => Adapter::Blockbook(BlockbookClient::new(url, self.chain)),
            EndpointApi::Blockcypher => Adapter::Blockcypher(BlockcypherClient::new(url)),
            EndpointApi::Whatsonchain => Adapter::Whatsonchain(WhatsonchainClient::new(url)),
            EndpointApi::BchRestV2 => Adapter::BchRest(BchRestClient::new(url)),
            api => {
                return Err(ApiError::InvalidInput(format!(
                    "{} is not a UTXO indexer",
                    api.as_str()
                )));
            }
        })
    }

    /// Ask every endpoint at once; the first success answers.
    async fn race<T, F, Fut>(&self, request: F) -> Result<T, ApiError>
    where
        F: Fn(Adapter) -> Fut,
        Fut: std::future::Future<Output = Result<T, ApiError>>,
    {
        let request = &request;
        first_success(self.endpoints.iter().map(|endpoint| {
            let adapter = self.adapter(endpoint);
            async move { request(adapter?).await }
        }))
        .await
    }

    /// Has this address ever been used on chain?
    pub async fn has_activity(&self, address: &str) -> Result<bool, ApiError> {
        self.race(|adapter| async move {
            match adapter {
                Adapter::Esplora(c) => c.has_activity(address).await,
                Adapter::Blockbook(c) => c.has_activity(address).await,
                Adapter::Blockcypher(c) => c.has_activity(address).await,
                Adapter::Whatsonchain(c) => c.has_activity(address).await,
                Adapter::BchRest(c) => c.has_activity(address).await,
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, ApiError> {
        self.race(|adapter| async move {
            match adapter {
                Adapter::Esplora(c) => c.fetch_balance(address).await,
                Adapter::Blockbook(c) => c.fetch_balance(address).await,
                Adapter::Blockcypher(c) => c.fetch_balance(address).await,
                Adapter::Whatsonchain(c) => c.fetch_balance(address).await,
                Adapter::BchRest(c) => c.fetch_balance(address).await,
            }
        })
        .await
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, ApiError> {
        self.race(|adapter| async move {
            match adapter {
                Adapter::Esplora(c) => c.fetch_utxos(address).await,
                Adapter::Blockbook(c) => c.fetch_utxos(address).await,
                Adapter::Blockcypher(c) => c.fetch_utxos(address).await,
                Adapter::Whatsonchain(c) => c.fetch_utxos(address).await,
                Adapter::BchRest(c) => c.fetch_utxos(address).await,
            }
        })
        .await
    }

    /// Recent history, newest first. `after_txid` continues from a confirmed
    /// transaction, which only Esplora can do; the others answer the first
    /// page alone.
    pub async fn fetch_history(
        &self,
        address: &str,
        after_txid: Option<&str>,
    ) -> Result<Vec<UtxoHistoryEntry>, ApiError> {
        let mut entries = self
            .race(|adapter| async move {
                match (adapter, after_txid) {
                    (Adapter::Esplora(c), after) => c.fetch_history(address, after).await,
                    (_, Some(_)) => Err(ApiError::InvalidInput(
                        "this indexer cannot continue history".into(),
                    )),
                    (Adapter::Blockbook(c), None) => c.fetch_history(address).await,
                    (Adapter::Blockcypher(c), None) => c.fetch_history(address).await,
                    (Adapter::Whatsonchain(c), None) => c.fetch_history(address).await,
                    (Adapter::BchRest(c), None) => c.fetch_history(address).await,
                }
            })
            .await?;
        // Unconfirmed first, then newest block first; stable, so an order the
        // indexer already gave within a block stands.
        entries.sort_by_key(|entry| {
            std::cmp::Reverse(match (entry.confirmed, entry.block_height) {
                (true, Some(height)) => height,
                _ => u64::MAX,
            })
        });
        Ok(entries)
    }

    /// The fee rate for a `confirmation_target` in blocks. WhatsOnChain and
    /// BCH REST report none.
    pub async fn fetch_fee_rate(&self, confirmation_target: u32) -> Result<FeeRate, ApiError> {
        self.race(|adapter| async move {
            match adapter {
                Adapter::Esplora(c) => c.fetch_fee_rate(confirmation_target).await,
                Adapter::Blockbook(c) => c.fetch_fee_rate(confirmation_target).await,
                Adapter::Blockcypher(c) => c.fetch_fee_rate(confirmation_target).await,
                Adapter::Whatsonchain(_) => Err(ApiError::InvalidInput(
                    "WhatsOnChain reports no fee rate".into(),
                )),
                Adapter::BchRest(_) => Err(ApiError::InvalidInput(
                    "BCH REST reports no fee rate".into(),
                )),
            }
        })
        .await
    }

    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, ApiError> {
        self.race(|adapter| async move {
            match adapter {
                Adapter::Esplora(c) => c.fetch_tx_status(txid).await,
                Adapter::Blockbook(c) => c.fetch_tx_status(txid).await,
                Adapter::Blockcypher(c) => c.fetch_tx_status(txid).await,
                Adapter::Whatsonchain(c) => c.fetch_tx_status(txid).await,
                Adapter::BchRest(c) => c.fetch_tx_status(txid).await,
            }
        })
        .await
    }

    /// Submit a signed transaction to every endpoint and answer with the
    /// txid the first one accepted. Every submission runs to its end: one
    /// endpoint's acceptance does not cut another's short.
    pub async fn broadcast(&self, raw_tx_hex: &str) -> Result<String, ApiError> {
        let submissions = self.endpoints.iter().map(|endpoint| {
            let adapter = self.adapter(endpoint);
            async move {
                match adapter? {
                    Adapter::Esplora(c) => c.broadcast_raw_tx(raw_tx_hex).await,
                    Adapter::Blockbook(c) => Ok(c.broadcast_raw_tx(raw_tx_hex).await?.txid),
                    Adapter::Blockcypher(c) => c.broadcast_raw_tx(raw_tx_hex).await,
                    Adapter::Whatsonchain(c) => Ok(c.broadcast_raw_tx(raw_tx_hex).await?.txid),
                    Adapter::BchRest(c) => c.broadcast_raw_tx(raw_tx_hex).await,
                }
            }
        });
        let results = futures::future::join_all(submissions).await;
        let mut last_err = ApiError::NoEndpoint;
        for result in results {
            match result {
                Ok(txid) => return Ok(txid),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn endpoint(api: EndpointApi, server: &MockServer) -> Endpoint {
        Endpoint {
            api,
            url: server.uri(),
        }
    }

    /// Litecoin speaks Esplora and BlockCypher. One dead endpoint of either
    /// API costs nothing while the other answers, and both answer alike.
    #[tokio::test]
    async fn every_api_answers_a_chain_in_the_same_terms() {
        let esplora = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/address/ltc1holder"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "address": "ltc1holder",
                "chain_stats": {"funded_txo_sum": 900, "spent_txo_sum": 100, "tx_count": 2},
                "mempool_stats": {"funded_txo_sum": 0, "spent_txo_sum": 50, "tx_count": 1}
            })))
            .mount(&esplora)
            .await;
        let blockcypher = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/addrs/ltc1holder/balance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "balance": 800, "unconfirmed_balance": -50, "n_tx": 2, "unconfirmed_n_tx": 1
            })))
            .mount(&blockcypher)
            .await;
        let dead = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&dead)
            .await;

        for (label, live) in [
            ("esplora", endpoint(EndpointApi::Esplora, &esplora)),
            (
                "blockcypher",
                endpoint(EndpointApi::Blockcypher, &blockcypher),
            ),
        ] {
            for dead_api in [EndpointApi::Esplora, EndpointApi::Blockcypher] {
                let client = UtxoClient::new(
                    Chain::Litecoin,
                    vec![endpoint(dead_api, &dead), live.clone()],
                );
                let balance = client.fetch_balance("ltc1holder").await.expect(label);
                assert_eq!(
                    (balance.confirmed_sats, balance.unconfirmed_sats),
                    (800, -50),
                    "{label} behind a dead {}",
                    dead_api.as_str()
                );
            }
        }
    }

    #[tokio::test]
    async fn only_esplora_continues_history() {
        let blockcypher = MockServer::start().await;
        let client = UtxoClient::new(
            Chain::Bitcoin,
            vec![endpoint(EndpointApi::Blockcypher, &blockcypher)],
        );
        assert!(client.fetch_history("bc1q", Some("txid")).await.is_err());
        assert!(blockcypher.received_requests().await.unwrap().is_empty());
    }

    /// A broadcast reaches every endpoint, even after one has accepted it.
    #[tokio::test]
    async fn a_broadcast_is_submitted_everywhere() {
        let txid = "ab".repeat(32);
        let esplora = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tx"))
            .respond_with(ResponseTemplate::new(200).set_body_string(txid.clone()))
            .expect(1)
            .mount(&esplora)
            .await;
        let blockcypher = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/txs/push"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(serde_json::json!({"tx": {"hash": txid}}))
                    .set_delay(std::time::Duration::from_millis(200)),
            )
            .expect(1)
            .mount(&blockcypher)
            .await;
        let client = UtxoClient::new(
            Chain::Bitcoin,
            vec![
                endpoint(EndpointApi::Esplora, &esplora),
                endpoint(EndpointApi::Blockcypher, &blockcypher),
            ],
        );
        assert_eq!(client.broadcast("00").await.unwrap(), txid);
    }
}

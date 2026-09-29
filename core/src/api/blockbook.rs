//! The Blockbook adapter, for every network a Blockbook instance indexes:
//! balances, UTXOs, history, fee estimates, status and broadcast.
//! `api::utxo` decides which adapter serves a request.
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::utxo::{FeeRate, Utxo, UtxoBalance, UtxoHistoryEntry, UtxoStatus, UtxoTxStatus};

// ── Wire shapes ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BlockbookUtxo {
    txid: String,
    vout: u32,
    value: String,
    #[serde(default)]
    confirmations: u32,
    height: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookAddress {
    balance: String,
    #[serde(default)]
    unconfirmed_balance: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookActivity {
    txs: u64,
    unconfirmed_txs: u64,
}

#[derive(Debug, Deserialize)]
struct BlockbookFeeEstimate {
    result: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookTxList {
    #[serde(default)]
    transactions: Vec<BlockbookTx>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookTx {
    txid: String,
    block_time: Option<u64>,
    block_height: Option<u64>,
    fees: Option<String>,
    #[serde(default)]
    vin: Vec<BlockbookIo>,
    #[serde(default)]
    vout: Vec<BlockbookIo>,
}

#[derive(Debug, Deserialize)]
struct BlockbookIo {
    addresses: Option<Vec<String>>,
    /// Satoshis, as a decimal string.
    value: Option<String>,
}

/// `/api/v2` reports the backend's chain tip, and on Zcash its consensus
/// branches.
#[derive(Debug, Deserialize)]
struct BlockbookStatus {
    backend: BlockbookBackend,
}

#[derive(Debug, Deserialize)]
struct BlockbookBackend {
    blocks: u32,
    consensus: Option<BlockbookConsensus>,
}

#[derive(Debug, Deserialize)]
struct BlockbookConsensus {
    chaintip: String,
    nextblock: String,
}

// ── Result types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockbookSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ── Client ────────────────────────────────────────────────────────────────

pub struct BlockbookClient {
    pub(crate) endpoints: Arc<Vec<String>>,
    pub(crate) client: Arc<HttpClient>,
    pub(crate) chain: crate::registry::Chain,
}

impl BlockbookClient {
    #[cfg(test)]
    pub(crate) fn require_chain(&self, expected: crate::registry::Chain) -> Result<(), String> {
        if self.chain.mainnet_counterpart() != expected {
            return Err("signer does not match the client's network".into());
        }
        Ok(())
    }

    fn normalize_address(&self, address: &str) -> String {
        if self.chain.mainnet_counterpart() == crate::registry::Chain::BitcoinCash {
            crate::derivation::bitcoin_cash::normalize_bch_address(address)
        } else {
            address.to_string()
        }
    }

    pub fn new(endpoints: Arc<Vec<String>>, chain: crate::registry::Chain) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
            chain,
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, String> {
        self.client.get_path(&self.endpoints, path).await
    }

    /// Has this address ever been used on chain? Blockbook's `details=basic`
    /// answers with counts, without transaction bodies.
    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, String> {
        let address = self.normalize_address(address);
        let info: BlockbookActivity = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        Ok(info.txs > 0 || info.unconfirmed_txs > 0)
    }

    /// The confirmed balance and the mempool's net change to it.
    pub async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, String> {
        let address = self.normalize_address(address);
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        Ok(UtxoBalance {
            confirmed_sats: parse_units(&info.balance)?,
            unconfirmed_sats: match info.unconfirmed_balance.as_deref() {
                Some(value) => value
                    .parse()
                    .map_err(|_| format!("invalid Blockbook amount {value:?}"))?,
                None => 0,
            },
        })
    }

    /// Unspent outputs, the mempool's included.
    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, String> {
        let address = self.normalize_address(address);
        let utxos: Vec<BlockbookUtxo> = self.get(&format!("/api/v2/utxo/{address}")).await?;
        utxos
            .into_iter()
            .map(|u| {
                let confirmed = u.confirmations > 0;
                Ok(Utxo {
                    txid: u.txid,
                    vout: u.vout,
                    value: parse_units(&u.value)?,
                    status: UtxoStatus {
                        confirmed,
                        block_height: u.height.filter(|_| confirmed),
                    },
                })
            })
            .collect::<Result<_, String>>()
    }

    /// The fee rate for a `blocks` confirmation target, in sat/vB.
    pub async fn fetch_fee_rate(&self, blocks: u32) -> Result<FeeRate, String> {
        let estimate: BlockbookFeeEstimate =
            self.get(&format!("/api/v2/estimatefee/{blocks}")).await?;
        let coin_per_kb = estimate
            .result
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or("Blockbook has no fee estimate")?;
        Ok(FeeRate {
            sats_per_vbyte: coin_per_kb * 1e8 / 1000.0,
        })
    }

    /// The most recent 50 transactions touching `address`, newest first.
    /// `net_sats` is the change to the queried address; the fee is the whole
    /// transaction's.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<UtxoHistoryEntry>, String> {
        let normalized = self.normalize_address(address);
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{normalized}?details=txs&page=1&pageSize=50"
            ))
            .await?;

        let entries: Result<Vec<Option<UtxoHistoryEntry>>, String> = list
            .transactions
            .into_iter()
            .map(|tx| {
                // The transaction's `value` is its total output — every
                // party's, change included — so the address's own inputs and
                // outputs are summed instead.
                let own = |ios: &[BlockbookIo]| -> i64 {
                    ios.iter()
                        .filter(|io| {
                            io.addresses
                                .as_deref()
                                .unwrap_or_default()
                                .iter()
                                .any(|a| a == &normalized || a == address)
                        })
                        .filter_map(|io| io.value.as_deref()?.parse::<i64>().ok())
                        .sum()
                };
                let net_sats = own(&tx.vout) - own(&tx.vin);
                let block_height = tx.block_height.filter(|height| *height > 0);
                let block_time = crate::api::time::history_time(
                    block_height.is_some(),
                    tx.block_time,
                    &tx.txid,
                )?;
                Ok((net_sats != 0).then_some(UtxoHistoryEntry {
                    txid: tx.txid,
                    confirmed: block_height.is_some(),
                    block_height,
                    block_time,
                    net_sats,
                    fee_sats: tx.fees.as_deref().and_then(|s| s.parse().ok()),
                }))
            })
            .collect();
        Ok(entries?.into_iter().flatten().collect())
    }

    /// Fetch confirmation status for a single txid via `/api/v2/tx/{txid}`.
    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, String> {
        let txid = txid.to_string();
        race(&self.endpoints, |base| {
            let txid = txid.clone();
            let client = self.client.clone();
            async move {
                let url = format!("{base}/api/v2/tx/{txid}");
                let tx: BlockbookTx = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
                Ok(UtxoTxStatus {
                    txid: tx.txid,
                    confirmed,
                    block_height: tx.block_height,
                    block_time: tx.block_time,
                    confirmations: None,
                })
            }
        })
        .await
    }

    /// The backend's current chain tip. Zcash's V5 builder needs it to pick an
    /// `nExpiryHeight` (`tip + 40`, the zcashd default).
    #[cfg(test)]
    pub async fn fetch_chain_tip_height(&self) -> Result<u64, String> {
        let status: BlockbookStatus = self.get("/api/v2").await?;
        Ok(u64::from(status.backend.blocks))
    }

    /// Submit a signed transaction. Blockbook answers with the txid.
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<BlockbookSendResult, String> {
        let hex = hex_tx.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let hex = hex.clone();
            let url = format!("{}/api/v2/sendtx/", base.trim_end_matches('/'));
            async move {
                let raw_tx_hex = hex.clone();
                let response = client
                    .post_text(&url, hex, RetryProfile::ChainWrite)
                    .await?;
                let body: serde_json::Value = serde_json::from_str(&response)
                    .map_err(|e| format!("Invalid Blockbook submission response: {e}"))?;
                let txid = body["result"]
                    .as_str()
                    .ok_or("Blockbook submission response is missing result")?;
                if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Invalid Blockbook transaction hash".into());
                }
                Ok(BlockbookSendResult {
                    txid: txid.to_lowercase(),
                    raw_tx_hex,
                })
            }
        })
        .await
    }
}

impl BlockbookClient {
    /// Bind the actual backend to the selected genesis, height and known consensus schedule.
    /// No silent NU5 fallback when older Blockbook versions omit consensus information.
    pub(crate) async fn zcash_context(&self) -> Result<(u32, u32), String> {
        let expected_genesis = self.chain.zcash_genesis()?;
        let genesis: serde_json::Value = self.get("/api/v2/block-index/0").await?;
        if genesis["blockHash"].as_str() != Some(expected_genesis) {
            return Err("Zcash endpoint is on the wrong network".into());
        }
        let status: BlockbookStatus = self.get("/api/v2").await?;
        let height = status.backend.blocks;
        let consensus = status
            .backend
            .consensus
            .ok_or("Blockbook reports no Zcash consensus branch")?;
        let branch = self
            .chain
            .zcash_consensus_branch(height.checked_add(1).ok_or("Height overflow")?)?;
        let parse = |s: &str| -> Result<u32, String> {
            if s.len() != 8 {
                return Err("Invalid Zcash consensus branch".into());
            }
            u32::from_str_radix(s, 16).map_err(|e| e.to_string())
        };
        if parse(&consensus.nextblock)? != branch
            || parse(&consensus.chaintip)? != self.chain.zcash_consensus_branch(height)?
        {
            return Err(
                "Zcash consensus upgrade is unsupported or inconsistent; update before sending"
                    .into(),
            );
        }
        Ok((height, branch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_client_without_endpoints_reports_rather_than_hangs() {
        let client = BlockbookClient::new(Arc::new(vec![]), crate::registry::Chain::Dash);
        assert!(client.fetch_balance("addr").await.is_err());
        assert!(client.fetch_utxos("addr").await.is_err());
        assert!(client.fetch_fee_rate(6).await.is_err());
    }
}

fn parse_units(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Blockbook amount must be unsigned integer digits".into());
    }
    value
        .parse()
        .map_err(|_| "Blockbook amount exceeds u64".into())
}

#[cfg(test)]
mod strict_amount_tests {
    use super::*;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
    #[tokio::test]
    async fn malformed_balances_and_utxos_are_errors() {
        for amount in [
            "0",
            "18446744073709551615",
            "",
            "-1",
            "+1",
            "1.0",
            "junk",
            "18446744073709551616",
        ] {
            let server = MockServer::start().await;
            Mock::given(any())
                .respond_with(move |request: &Request| {
                    let body = if request.url.path().contains("/utxo/") {
                        serde_json::json!([
                            {"txid":"aa".repeat(32),"vout":0,"value":"1"},
                            {"txid":"bb".repeat(32),"vout":1,"value":amount}
                        ])
                    } else {
                        serde_json::json!({"balance":amount})
                    };
                    ResponseTemplate::new(200).set_body_json(body)
                })
                .mount(&server)
                .await;
            let client = BlockbookClient::new(
                Arc::new(vec![server.uri()]),
                crate::registry::Chain::Litecoin,
            );
            let valid = ["0", "18446744073709551615"].contains(&amount);
            assert_eq!(
                client.fetch_balance("holder").await.is_ok(),
                valid,
                "{amount}"
            );
            assert_eq!(
                client.fetch_utxos("holder").await.is_ok(),
                valid,
                "{amount}"
            );
        }
    }
}

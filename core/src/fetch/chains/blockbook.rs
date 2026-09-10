//! Trezor Blockbook REST client, shared by every chain served by one.
//!
//! Blockbook exposes the same `/api/v2/...` surface for Litecoin, Bitcoin
//! Cash, Bitcoin Gold, Zcash and Dash: balance, UTXOs, history, fee estimate,
//! broadcast and tx status. Each chain had its own copy of it — five files
//! that pairwise matched 76–91%, with the same six JSON shapes redeclared in
//! each. The copies had drifted rather than diverged: `has_activity` existed
//! on two of the five and `fetch_chain_tip_height` on one, for no reason but
//! which file was edited when.
//!
//! What is genuinely per-chain is the marker type: it keeps the five clients
//! distinct so each chain's signing code can hang its own `sign_and_broadcast`
//! off its own client, and it carries the one behavioural difference — Bitcoin
//! Cash accepts CashAddr and legacy forms of the same address, and normalizes
//! before asking.

use std::marker::PhantomData;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

/// A chain served by Blockbook.
pub trait BlockbookNetwork: 'static {
    /// The form of `address` this chain's Blockbook instance is asked about.
    /// Only Bitcoin Cash rewrites anything.
    fn normalize_address(address: &str) -> String {
        address.to_string()
    }
}

// ── Wire shapes ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BlockbookUtxo {
    txid: String,
    vout: u32,
    value: String,
    #[serde(default)]
    confirmations: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookAddress {
    balance: String,
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
    #[serde(default)]
    value: String,
    fees: Option<String>,
    #[serde(default)]
    vin: Vec<BlockbookVin>,
}

#[derive(Debug, Deserialize)]
struct BlockbookVin {
    addresses: Option<Vec<String>>,
}

/// `/api/v2` reports the backend's chain tip.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookStatus {
    backend: BlockbookBackend,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockbookBackend {
    blocks: u64,
}

// ── Result types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockbookBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockbookUtxoEntry {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockbookHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Net value change for the queried address. Negative = outgoing.
    pub amount_sat: i64,
    pub fee_sat: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockbookSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

impl super::SignedSubmission for BlockbookSendResult {
    fn submission_id(&self) -> &str {
        &self.txid
    }
    fn signed_payload(&self) -> &str {
        &self.raw_tx_hex
    }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat {
        super::SignedPayloadFormat::Hex
    }
}

// ── Client ────────────────────────────────────────────────────────────────

pub struct BlockbookClient<N: BlockbookNetwork> {
    pub(crate) endpoints: Arc<Vec<String>>,
    pub(crate) client: Arc<HttpClient>,
    network: PhantomData<fn() -> N>,
}

impl<N: BlockbookNetwork> BlockbookClient<N> {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
            network: PhantomData,
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    /// Has this address ever been used on chain? Blockbook's `details=basic`
    /// answers with counts, without transaction bodies.
    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, String> {
        let address = N::normalize_address(address);
        let info: BlockbookActivity = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        Ok(info.txs > 0 || info.unconfirmed_txs > 0)
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<BlockbookBalance, String> {
        let address = N::normalize_address(address);
        let info: BlockbookAddress = self
            .get(&format!("/api/v2/address/{address}?details=basic"))
            .await?;
        let sat = parse_units(&info.balance)?;
        Ok(BlockbookBalance {
            balance_sat: sat,
            balance_display: format_sats(sat),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<BlockbookUtxoEntry>, String> {
        let address = N::normalize_address(address);
        let utxos: Vec<BlockbookUtxo> = self.get(&format!("/api/v2/utxo/{address}")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| {
                Ok(BlockbookUtxoEntry {
                    txid: u.txid,
                    vout: u.vout,
                    value_sat: parse_units(&u.value)?,
                    confirmations: u.confirmations,
                })
            })
            .collect::<Result<_, String>>()?)
    }

    /// Fetch recommended fee rate for `blocks` confirmation target.
    /// Returns satoshis per vbyte. Falls back to 1 sat/vB on failure.
    pub async fn fetch_fee_rate(&self, blocks: u32) -> u64 {
        let estimate: Result<BlockbookFeeEstimate, _> =
            self.get(&format!("/api/v2/estimatefee/{blocks}")).await;
        estimate
            .ok()
            .and_then(|e| e.result.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|coin_per_kb| ((coin_per_kb * 1e8 / 1000.0).ceil() as u64).max(1))
            .unwrap_or(1)
    }

    /// Fetch the most recent 50 transactions touching `address` via
    /// Blockbook's `details=txs` pagination. `amount_sat` is the net value
    /// change from the queried address's perspective (positive = received,
    /// negative = sent). Fee is the absolute tx fee; direction detection
    /// inspects the vin address lists.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<BlockbookHistoryEntry>, String> {
        let normalized = N::normalize_address(address);
        let list: BlockbookTxList = self
            .get(&format!(
                "/api/v2/address/{normalized}?details=txs&page=1&pageSize=50"
            ))
            .await?;

        Ok(list
            .transactions
            .into_iter()
            .map(|tx| {
                let is_incoming = !tx.vin.iter().any(|i| {
                    i.addresses
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .any(|a| a == &normalized || a == address)
                });
                let amount_sat: i64 = tx.value.parse().unwrap_or(0);
                let fee_sat: u64 = tx.fees.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
                BlockbookHistoryEntry {
                    txid: tx.txid,
                    block_height: tx.block_height.unwrap_or(0),
                    timestamp: tx.block_time.unwrap_or(0),
                    amount_sat: if is_incoming { amount_sat } else { -amount_sat },
                    fee_sat,
                    is_incoming,
                }
            })
            .collect())
    }

    /// Fetch confirmation status for a single txid via `/api/v2/tx/{txid}`.
    pub async fn fetch_tx_status(
        &self,
        txid: &str,
    ) -> Result<crate::fetch::chains::bitcoin::UtxoTxStatus, String> {
        let txid = txid.to_string();
        with_fallback(&self.endpoints, |base| {
            let txid = txid.clone();
            let client = self.client.clone();
            async move {
                let url = format!("{base}/api/v2/tx/{txid}");
                let tx: BlockbookTx = client.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed = tx.block_height.map(|h| h > 0).unwrap_or(false);
                Ok(crate::fetch::chains::bitcoin::UtxoTxStatus {
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
    pub async fn fetch_chain_tip_height(&self) -> Result<u64, String> {
        let status: BlockbookStatus = self.get("/api/v2").await?;
        Ok(status.backend.blocks)
    }

    /// Submit a signed transaction. Blockbook answers with the txid.
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<BlockbookSendResult, String> {
        let hex = hex_tx.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let hex = hex.clone();
            let url = format!("{}/api/v2/sendtx/", base.trim_end_matches('/'));
            async move {
                let raw_tx_hex = hex.clone();
                let txid: String = client
                    .post_text(&url, hex, RetryProfile::ChainWrite)
                    .await?;
                Ok(BlockbookSendResult {
                    txid: txid.trim().to_string(),
                    raw_tx_hex,
                })
            }
        })
        .await
    }
}

/// Render satoshis (or the equivalent 8-decimal unit every chain in this
/// family uses) as a trimmed decimal string.
fn format_sats(sat: u64) -> String {
    let whole = sat / 100_000_000;
    let frac = sat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    format!("{}.{}", whole, format!("{frac:08}").trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;
    impl BlockbookNetwork for Plain {}

    #[test]
    fn balances_render_with_trailing_zeros_trimmed() {
        assert_eq!(format_sats(0), "0");
        assert_eq!(format_sats(100_000_000), "1");
        assert_eq!(format_sats(150_000_000), "1.5");
        assert_eq!(format_sats(1), "0.00000001");
        assert_eq!(format_sats(100_000_001), "1.00000001");
    }

    #[tokio::test]
    async fn a_client_without_endpoints_reports_rather_than_hangs() {
        let client = BlockbookClient::<Plain>::new(Arc::new(vec![]));
        assert!(client.fetch_balance("addr").await.is_err());
        assert!(client.fetch_utxos("addr").await.is_err());
        assert_eq!(client.fetch_fee_rate(6).await, 1, "fee estimate falls back");
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
    use crate::fetch::chains::litecoin::LitecoinClient;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
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
            let client = LitecoinClient::new(Arc::new(vec![server.uri()]));
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

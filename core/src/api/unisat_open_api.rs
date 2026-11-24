//! UniSat's Open API indexer (`/v1/indexer/…`), the one keyless indexer
//! that lists a Fractal Bitcoin address's unspent outputs and its whole
//! history. Every answer is `{code, msg, data}`, and a `code` other than 0
//! is a refusal.
//!
//! Its spendable outputs are the "available" ones: UniSat leaves out every
//! output carrying an inscription, a rune or another asset, which a plain
//! transfer would hand to the recipient or the fee.
use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::api::error::{ApiError, OrDecode};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::utxo::{Utxo, UtxoBalance, UtxoHistoryEntry, UtxoStatus, UtxoTxStatus};
use crate::registry::Chain;

/// History rows per page; the API serves at most 64.
const HISTORY_PAGE: usize = 20;
/// Outputs, inputs or unspent outputs asked for at once.
const LIST_PAGE: usize = 64;

#[derive(Debug, Deserialize)]
struct Answer<T> {
    code: i64,
    #[serde(default)]
    msg: String,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct ChainInfo {
    blocks: u64,
}

#[derive(Debug, Deserialize)]
struct BlockAtHeight {
    id: String,
    height: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Balance {
    satoshi: u64,
    #[serde(default)]
    pending_satoshi: i64,
}

#[derive(Debug, Deserialize)]
struct UtxoPage {
    #[serde(default)]
    total: usize,
    #[serde(default)]
    utxo: Vec<AvailableUtxo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AvailableUtxo {
    txid: String,
    vout: u32,
    satoshi: u64,
    script_pk: String,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    confirmations: u64,
    #[serde(default)]
    is_spent: bool,
    #[serde(default)]
    inscriptions: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct HistoryPage {
    total: usize,
    #[serde(default)]
    detail: Vec<HistoryTx>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryTx {
    txid: String,
    n_in: usize,
    n_out: usize,
    #[serde(default)]
    in_satoshi: u64,
    #[serde(default)]
    out_satoshi: u64,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    timestamp: u64,
    #[serde(default)]
    confirmations: u64,
}

/// One input or output of a transaction, with the address it pays.
#[derive(Debug, Deserialize)]
struct TxIo {
    #[serde(default)]
    address: String,
    satoshi: u64,
}

#[derive(Debug, Deserialize)]
struct TxInfo {
    txid: String,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    timestamp: u64,
    #[serde(default)]
    confirmations: u64,
}

pub struct UnisatOpenApiClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
    chain: Chain,
}

impl UnisatOpenApiClient {
    pub fn new(endpoints: Arc<Vec<String>>, chain: Chain) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
            chain,
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}/v1/indexer{path}", base.trim_end_matches('/'));
            async move {
                let answer: Answer<T> = client.get_json(&url, RetryProfile::ChainRead).await?;
                if answer.code != 0 {
                    return Err(ApiError::Rejected(format!("UniSat: {}", answer.msg)));
                }
                answer.data.or_decode("UniSat answer has no data")
            }
        })
        .await
    }

    /// `address` as the indexer is asked for it: one of the network's own.
    fn address<'a>(&self, address: &'a str) -> Result<&'a str, ApiError> {
        crate::derivation::utxo_address::parse_utxo_address(self.chain, address)
            .map_err(ApiError::invalid)?;
        Ok(address.trim())
    }

    pub(crate) async fn fetch_tip_height(&self) -> Result<u64, ApiError> {
        Ok(self.get::<ChainInfo>("/blockchain/info").await?.blocks)
    }

    pub(crate) async fn fetch_block_hash(&self, height: u64) -> Result<String, ApiError> {
        let block: BlockAtHeight = self.get(&format!("/height/{height}/block")).await?;
        if block.height != height {
            return Err(ApiError::decode("UniSat answered another block"));
        }
        Ok(block.id)
    }

    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, ApiError> {
        let address = self.address(address)?;
        let page: HistoryPage = self
            .get(&format!("/address/{address}/history?cursor=0&size=1"))
            .await?;
        Ok(page.total > 0)
    }

    pub(crate) async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, ApiError> {
        let address = self.address(address)?;
        let balance: Balance = self.get(&format!("/address/{address}/balance")).await?;
        Ok(UtxoBalance {
            confirmed_sats: balance.satoshi,
            unconfirmed_sats: balance.pending_satoshi,
        })
    }

    /// The outputs a plain transfer can spend, each paying the address's own
    /// script and carrying no inscription.
    pub(crate) async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, ApiError> {
        let address = self.address(address)?;
        let script = hex::encode(
            crate::derivation::utxo_address::parse_utxo_address(self.chain, address)
                .map_err(ApiError::invalid)?
                .script_pubkey(),
        );
        let mut utxos = Vec::new();
        let mut cursor = 0usize;
        let mut total = None;
        loop {
            // A page reaching past the end of the list is refused.
            let size = total.map_or(LIST_PAGE, |total: usize| {
                LIST_PAGE.min(total.saturating_sub(cursor))
            });
            let page: UtxoPage = self
                .get(&format!(
                    "/address/{address}/available-utxo-data?cursor={cursor}&size={size}"
                ))
                .await?;
            total = Some(page.total);
            let received = page.utxo.len();
            for utxo in page.utxo {
                if !utxo.script_pk.eq_ignore_ascii_case(&script) {
                    return Err(ApiError::decode("UniSat output pays another script"));
                }
                if utxo.is_spent || !utxo.inscriptions.is_empty() {
                    continue;
                }
                let confirmed = utxo.confirmations > 0;
                utxos.push(Utxo {
                    txid: utxo.txid,
                    vout: utxo.vout,
                    value: utxo.satoshi,
                    status: UtxoStatus {
                        confirmed,
                        block_height: confirmed.then_some(utxo.height),
                    },
                });
            }
            cursor += received;
            if received == 0 || cursor >= page.total {
                break;
            }
        }
        Ok(utxos)
    }

    /// Every input or output of `txid` (`side` is `ins` or `outs`), of
    /// which there are `count`.
    async fn tx_side(&self, txid: &str, side: &str, count: usize) -> Result<Vec<TxIo>, ApiError> {
        let mut all = Vec::with_capacity(count);
        while all.len() < count {
            let page: Vec<TxIo> = self
                .get(&format!(
                    "/tx/{txid}/{side}?cursor={}&size={}",
                    all.len(),
                    LIST_PAGE.min(count - all.len())
                ))
                .await?;
            if page.is_empty() {
                return Err(ApiError::decode(
                    "UniSat transaction is shorter than it says",
                ));
            }
            all.extend(page);
        }
        Ok(all)
    }

    /// A page of 20 transactions touching `address`, newest first, each
    /// with the address's net change, read from its inputs and outputs.
    pub(crate) async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<UtxoHistoryEntry>, ApiError> {
        let number = crate::api::history_page::page_number(cursor)?;
        let address = self.address(address)?;
        let start = (number as usize - 1)
            .checked_mul(HISTORY_PAGE)
            .ok_or_else(|| ApiError::invalid("invalid history page cursor"))?;
        let page: HistoryPage = self
            .get(&format!(
                "/address/{address}/history?cursor={start}&size={HISTORY_PAGE}"
            ))
            .await?;
        let next_cursor = (start + page.detail.len() < page.total && !page.detail.is_empty())
            .then(|| (number + 1).to_string());
        let items: Vec<Option<UtxoHistoryEntry>> = futures::stream::iter(page.detail)
            .map(|tx| self.history_entry(address, tx))
            .buffered(4)
            .try_collect()
            .await?;
        Ok(crate::api::HistoryPage {
            items: items.into_iter().flatten().collect(),
            next_cursor,
        })
    }

    /// `tx` as a history row of `address`: its net change there, read from
    /// the transaction's inputs and outputs. `None` where it nets nothing.
    async fn history_entry(
        &self,
        address: &str,
        tx: HistoryTx,
    ) -> Result<Option<UtxoHistoryEntry>, ApiError> {
        let (inputs, outputs) = futures::try_join!(
            self.tx_side(&tx.txid, "ins", tx.n_in),
            self.tx_side(&tx.txid, "outs", tx.n_out),
        )?;
        let own = |ios: &[TxIo]| -> Result<i64, ApiError> {
            ios.iter()
                .filter(|io| io.address == address)
                .try_fold(0i64, |sum, io| {
                    i64::try_from(io.satoshi)
                        .ok()
                        .and_then(|value| sum.checked_add(value))
                })
                .or_decode("UniSat amount overflow")
        };
        let net = own(&outputs)?
            .checked_sub(own(&inputs)?)
            .or_decode("UniSat amount overflow")?;
        if net == 0 {
            return Ok(None);
        }
        let confirmed = tx.confirmations > 0;
        Ok(Some(UtxoHistoryEntry {
            block_time: crate::api::time::history_time(confirmed, Some(tx.timestamp), &tx.txid)?,
            confirmed,
            block_height: confirmed.then_some(tx.height),
            net_sats: net,
            fee_sats: (tx.in_satoshi > 0)
                .then(|| tx.in_satoshi.checked_sub(tx.out_satoshi))
                .flatten(),
            txid: tx.txid,
        }))
    }

    pub(crate) async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, ApiError> {
        let tx: TxInfo = self.get(&format!("/tx/{txid}")).await?;
        if !tx.txid.eq_ignore_ascii_case(txid) {
            return Err(ApiError::decode("UniSat answered another transaction"));
        }
        let confirmed = tx.confirmations > 0;
        Ok(UtxoTxStatus {
            txid: tx.txid,
            confirmed,
            block_height: confirmed.then_some(tx.height),
            block_time: confirmed.then_some(tx.timestamp),
            confirmations: Some(tx.confirmations),
        })
    }

    /// Submit a signed transaction: `POST /v1/indexer/local_pushtx`, which
    /// answers with its txid.
    pub(crate) async fn broadcast_raw_tx(&self, raw_tx_hex: &str) -> Result<String, ApiError> {
        let body = serde_json::json!({ "txHex": raw_tx_hex });
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let url = format!("{}/v1/indexer/local_pushtx", base.trim_end_matches('/'));
            async move {
                let answer: Answer<String> = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                if answer.code != 0 {
                    return Err(ApiError::Rejected(format!("UniSat: {}", answer.msg)));
                }
                let txid = answer
                    .data
                    .or_decode("UniSat broadcast answer has no txid")?;
                if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(ApiError::decode("UniSat broadcast answered no txid"));
                }
                Ok(txid.to_ascii_lowercase())
            }
        })
        .await
    }
}

#[cfg(test)]
#[path = "tests/unisat_open_api.rs"]
mod tests;

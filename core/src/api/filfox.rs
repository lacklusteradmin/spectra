//! Filfox's API (`filfox.info/api/v1`): an address's FIL transfers, which no
//! Lotus node indexes.

use std::sync::Arc;

use serde_json::Value;

use crate::api::error::{ApiError, OrDecode};
use crate::api::history_page::{HistoryAmount, HistoryEntry, HistoryOutcome, HistoryPage};
use crate::api::http::HttpClient;

/// Transfers one request asks for: the most Filfox serves at once.
const PAGE_SIZE: u32 = 100;

pub struct FilfoxClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl FilfoxClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// One page of the address's history, newest first: each message that
    /// moved FIL to or from it, internal sends included, as what it did to
    /// the address's balance, the fee aside. A message that moved nothing
    /// but its fee, as a failed one, is not listed.
    pub(crate) async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<HistoryPage<HistoryEntry>, ApiError> {
        let page = crate::api::history_page::page_number(cursor)?;
        let response: Value = self
            .client
            .get_path(
                &self.endpoints,
                &format!(
                    "/address/{address}/transfers?pageSize={PAGE_SIZE}&page={}&type=transfer",
                    page - 1
                ),
            )
            .await?;
        let total = response["totalCount"]
            .as_u64()
            .or_decode("Filfox transfers: missing total")?;
        let transfers = response["transfers"]
            .as_array()
            .or_decode("Filfox transfers: missing list")?;
        let items = history_entries(transfers)?;
        let next_cursor = (!transfers.is_empty() && u64::from(page) * u64::from(PAGE_SIZE) < total)
            .then(|| (page + 1).to_string());
        Ok(HistoryPage { items, next_cursor })
    }
}

/// One entry per message, in the order the transfers name them: its
/// transfers' signed amounts summed, from the address's side. A message
/// whose transfers cancel out, a send to itself, is not listed.
fn history_entries(transfers: &[Value]) -> Result<Vec<HistoryEntry>, ApiError> {
    struct Net {
        txid: String,
        received: u128,
        sent: u128,
        counterparty: String,
        block_height: Option<i64>,
        timestamp: Option<f64>,
    }
    let mut messages: Vec<Net> = Vec::new();
    // What a message burns and pays its miner is its fee, which a request
    // for transfers alone leaves out; it is passed over all the same.
    for transfer in transfers.iter().filter(|transfer| {
        !transfer["type"]
            .as_str()
            .is_some_and(|kind| kind.ends_with("-fee"))
    }) {
        let txid = transfer["message"]
            .as_str()
            .filter(|cid| !cid.is_empty())
            .or_decode("Filfox transfer: missing message")?;
        let value = transfer["value"]
            .as_str()
            .or_decode("Filfox transfer: missing value")?;
        let (outgoing, digits) = match value.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, value),
        };
        let amount: u128 = (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
            .or_decode("Filfox transfer: value is not a whole number of attoFIL")?;
        let index = match messages.iter().position(|net| net.txid == txid) {
            Some(index) => index,
            None => {
                messages.push(Net {
                    txid: txid.to_string(),
                    received: 0,
                    sent: 0,
                    counterparty: String::new(),
                    block_height: transfer["height"].as_i64(),
                    timestamp: transfer["timestamp"].as_f64(),
                });
                messages.len() - 1
            }
        };
        let net = &mut messages[index];
        let side = if outgoing {
            &mut net.sent
        } else {
            &mut net.received
        };
        *side = side
            .checked_add(amount)
            .or_decode("Filfox transfer: amount overflows")?;
        let other = if outgoing { "to" } else { "from" };
        if amount > 0 {
            net.counterparty = transfer[other].as_str().unwrap_or_default().to_string();
        }
    }
    Ok(messages
        .into_iter()
        .filter(|net| net.received != net.sent)
        .map(|net| HistoryEntry {
            incoming: net.received > net.sent,
            amount: HistoryAmount::Units(net.received.abs_diff(net.sent)),
            txid: net.txid,
            contract: None,
            outcome: HistoryOutcome::Confirmed,
            block_height: net.block_height,
            timestamp: net.timestamp,
            counterparty: net.counterparty,
        })
        .collect())
}

#[cfg(test)]
#[path = "tests/filfox.rs"]
mod tests;

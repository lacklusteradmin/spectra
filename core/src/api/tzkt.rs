//! TzKT's indexer API for Tezos: the network it indexes, an address's
//! transactions and an operation's status. A node keeps no index by
//! address, so history and status are read here; balances, simulation and
//! injection are `api::tezos_rpc`'s.

use std::sync::Arc;

use serde_json::Value;

use crate::api::error::{ApiError, OrDecode};
use crate::api::history_page::{HistoryAmount, HistoryEntry, HistoryOutcome, HistoryPage};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::transaction_status::TransactionStatus;
use crate::derivation::tezos::{decode_base58check, prefix};
use crate::registry::Chain;

/// How many transactions one history request asks for.
const HISTORY_PAGE_SIZE: usize = 50;

pub struct TzktClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl TzktClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    /// Refuse an indexer of another network: its head must name `chain`'s
    /// chain id.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let expected = chain.tezos_network()?.chain_id;
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            async move {
                let head: Value = client
                    .get_json(
                        &format!("{}/v1/head", base.trim_end_matches('/')),
                        RetryProfile::ChainRead,
                    )
                    .await?;
                let id = head["chainId"]
                    .as_str()
                    .or_decode("TzKT head: missing chainId")?;
                if id != expected {
                    return Err(ApiError::invalid(format!(
                        "Endpoint indexes {id}, not {expected}"
                    )));
                }
                Ok(())
            }
        })
        .await
    }

    /// One page of the address's transactions, newest first, each operation
    /// group as what it did to the address's balance. The cursor is the id
    /// of the last transaction read.
    pub(crate) async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<HistoryPage<HistoryEntry>, ApiError> {
        crate::derivation::tezos::decode_address(address)
            .map_err(|_| ApiError::invalid("Not a Tezos address"))?;
        let last_id = cursor
            .map(|cursor| {
                cursor
                    .parse::<u64>()
                    .map_err(|_| ApiError::InvalidInput("invalid history cursor".into()))
            })
            .transpose()?;
        let answer = self
            .get(&format!(
                "/v1/accounts/{address}/operations?type=transaction&limit={HISTORY_PAGE_SIZE}{}",
                last_id.map_or(String::new(), |id| format!("&lastId={id}"))
            ))
            .await?;
        let operations = answer.as_array().or_decode("TzKT history: not a list")?;
        history_page(operations, address)
    }

    /// Whether the operation `hash` is in a block, and whether every
    /// operation in it was applied. One the indexer does not know is still
    /// pending.
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<TransactionStatus, ApiError> {
        decode_base58check(hash, &prefix::OPERATION, 32)
            .ok_or_else(|| ApiError::invalid("Invalid Tezos operation hash"))?;
        let answer = self.get(&format!("/v1/operations/{hash}")).await?;
        status_from(&answer, hash)
    }
}

/// The page `operations` make: one entry per operation group. A full page's
/// last group may continue on the next one, so it is left for that page to
/// read whole, unless it is all the page holds.
fn history_page(
    operations: &[Value],
    address: &str,
) -> Result<HistoryPage<HistoryEntry>, ApiError> {
    let hash = |operation: &Value| operation["hash"].as_str().map(str::to_string);
    let mut groups: Vec<Vec<&Value>> = Vec::new();
    for operation in operations {
        match groups.last_mut() {
            Some(group) if hash(group[0]) == hash(operation) => group.push(operation),
            _ => groups.push(vec![operation]),
        }
    }
    let full = operations.len() >= HISTORY_PAGE_SIZE;
    if full && groups.len() > 1 {
        groups.pop();
    }
    let next_cursor = if full {
        let last = groups
            .last()
            .and_then(|group| group.last())
            .or_decode("TzKT history: empty page")?;
        Some(
            last["id"]
                .as_u64()
                .or_decode("TzKT history: missing id")?
                .to_string(),
        )
    } else {
        None
    };
    let mut items = Vec::new();
    for group in groups {
        if let Some(entry) = history_entry(&group, address)? {
            items.push(entry);
        }
    }
    Ok(HistoryPage { items, next_cursor })
}

/// What one operation group did to `address`'s balance: the tez its
/// transactions moved to and from it, fees aside. A group that failed moved
/// nothing but its fees; it is shown with the amount it asked to send.
fn history_entry(group: &[&Value], address: &str) -> Result<Option<HistoryEntry>, ApiError> {
    let first = group.first().or_decode("TzKT history: empty group")?;
    let txid = first["hash"]
        .as_str()
        .or_decode("TzKT history: missing hash")?
        .to_string();
    let applied = group
        .iter()
        .all(|operation| operation["status"].as_str() == Some("applied"));
    let (mut received, mut sent) = (0u128, 0u128);
    let mut counterparty = String::new();
    for operation in group {
        let amount = u128::from(
            operation["amount"]
                .as_u64()
                .or_decode("TzKT history: missing amount")?,
        );
        let sender = operation["sender"]["address"].as_str().unwrap_or_default();
        let target = operation["target"]["address"].as_str().unwrap_or_default();
        if amount == 0 || sender == target {
            continue;
        }
        if target == address && applied {
            received = received
                .checked_add(amount)
                .or_decode("TzKT history: overflow")?;
            counterparty = sender.to_string();
        } else if sender == address {
            sent = sent
                .checked_add(amount)
                .or_decode("TzKT history: overflow")?;
            counterparty = target.to_string();
        }
    }
    if received == sent {
        return Ok(None);
    }
    Ok(Some(HistoryEntry {
        txid,
        contract: None,
        amount: HistoryAmount::Units(received.abs_diff(sent)),
        incoming: received > sent,
        outcome: if applied {
            HistoryOutcome::Confirmed
        } else {
            HistoryOutcome::Failed
        },
        block_height: first["level"].as_i64(),
        timestamp: first["timestamp"]
            .as_str()
            .and_then(crate::api::time::parse_iso8601_timestamp),
        counterparty,
    }))
}

/// An operation's status from TzKT's operations under its hash.
fn status_from(answer: &Value, hash: &str) -> Result<TransactionStatus, ApiError> {
    let operations = answer.as_array().or_decode("TzKT status: not a list")?;
    if operations.is_empty() {
        return Ok(TransactionStatus::Pending);
    }
    if operations
        .iter()
        .any(|operation| operation["hash"].as_str() != Some(hash))
    {
        return Err(ApiError::decode("TzKT status: another operation"));
    }
    let block = operations[0]["level"]
        .as_u64()
        .filter(|level| *level > 0)
        .or_decode("TzKT status: missing level")?;
    Ok(TransactionStatus::Confirmed {
        succeeded: operations
            .iter()
            .all(|operation| operation["status"].as_str() == Some("applied")),
        block: Some(block),
    })
}

#[cfg(test)]
#[path = "tests/tzkt.rs"]
mod tests;

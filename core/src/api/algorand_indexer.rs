//! An Algorand indexer (v2): an address's transactions and a committed
//! transaction by id, each on the network its genesis names.

use std::sync::Arc;

use serde_json::Value;

use crate::api::algod::{check_genesis, validate_txid};
use crate::api::error::{ApiError, OrDecode};
use crate::api::history_page::{HistoryAmount, HistoryEntry, HistoryOutcome, HistoryPage};
use crate::api::http::HttpClient;
use crate::registry::Chain;

/// How many transactions one history request asks for.
const HISTORY_PAGE_SIZE: u32 = 50;

pub struct AlgorandIndexerClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl AlgorandIndexerClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    /// Refuse an indexer that is not on `chain`'s network: its first block
    /// names the genesis every transaction binds.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        check_genesis(chain, &self.get("/v2/blocks/0").await?)
    }

    /// The round `txid` was committed in, or `None` while the indexer has
    /// not seen it.
    pub(crate) async fn fetch_confirmed_round(
        &self,
        chain: Chain,
        txid: &str,
    ) -> Result<Option<u64>, ApiError> {
        validate_txid(txid)?;
        let value = match self.get(&format!("/v2/transactions/{txid}")).await {
            Ok(value) => value,
            Err(ApiError::Status { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let transaction = &value["transaction"];
        if transaction["id"].as_str() != Some(txid) {
            return Err(ApiError::decode("Algorand indexer: another transaction"));
        }
        check_genesis(chain, transaction)?;
        transaction["confirmed-round"]
            .as_u64()
            .filter(|round| *round > 0)
            .map(Some)
            .or_decode("Algorand indexer: missing confirmed round")
    }

    /// One page of `address`'s history, newest first: each transaction it
    /// took part in, inner transactions included, as what it did to the
    /// address's ALGO. Every transaction must name `chain`'s genesis.
    pub(crate) async fn fetch_history_page(
        &self,
        chain: Chain,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<HistoryPage<HistoryEntry>, ApiError> {
        let address = crate::derivation::algorand::decode_address(address)
            .map(|key| crate::derivation::algorand::address_from_public_key(&key))
            .map_err(|_| ApiError::invalid("Not an Algorand address"))?;
        let next = match cursor {
            Some(token) if !token.is_empty() => {
                format!("&next={}", crate::api::history_page::query_value(token))
            }
            Some(_) => return Err(ApiError::invalid("invalid history cursor")),
            None => String::new(),
        };
        let value = self
            .get(&format!(
                "/v2/accounts/{address}/transactions?limit={HISTORY_PAGE_SIZE}{next}"
            ))
            .await?;
        let transactions = value["transactions"]
            .as_array()
            .or_decode("Algorand indexer: missing transactions")?;
        let mut items = Vec::new();
        for transaction in transactions {
            check_genesis(chain, transaction)?;
            if let Some(entry) = history_entry(transaction, &address)? {
                items.push(entry);
            }
        }
        let next_cursor = value["next-token"]
            .as_str()
            .filter(|token| !token.is_empty() && !transactions.is_empty())
            .map(str::to_string);
        Ok(HistoryPage { items, next_cursor })
    }
}

/// What a transaction moved to and from `address`, its inner
/// transactions included: payments, and the remainder an account closed
/// out to another. Fees aside, as the amount a transfer shows.
#[derive(Default)]
struct Movement {
    received: u128,
    sent: u128,
    counterparty: String,
}

impl Movement {
    fn add(&mut self, transaction: &Value, address: &str) -> Result<(), ApiError> {
        if transaction["tx-type"] == "pay" {
            let payment = &transaction["payment-transaction"];
            let sender = transaction["sender"]
                .as_str()
                .or_decode("Algorand history: missing sender")?;
            let receiver = payment["receiver"]
                .as_str()
                .or_decode("Algorand history: missing receiver")?;
            let amount = payment["amount"]
                .as_u64()
                .or_decode("Algorand history: missing amount")?;
            let close_to = payment["close-remainder-to"].as_str();
            let closed = payment["close-amount"].as_u64().unwrap_or(0);
            let ours = |party: &str| party == address;
            for (from, to, amount) in [(sender, Some(receiver), amount), (sender, close_to, closed)]
            {
                let Some(to) = to else { continue };
                if amount == 0 || ours(from) == ours(to) {
                    continue;
                }
                if ours(to) {
                    self.received = self
                        .received
                        .checked_add(u128::from(amount))
                        .or_decode("Algorand history: overflow")?;
                    self.counterparty = from.to_string();
                } else {
                    self.sent = self
                        .sent
                        .checked_add(u128::from(amount))
                        .or_decode("Algorand history: overflow")?;
                    self.counterparty = to.to_string();
                }
            }
        }
        for inner in transaction["inner-txns"].as_array().into_iter().flatten() {
            self.add(inner, address)?;
        }
        Ok(())
    }
}

/// One committed transaction as `address`'s history shows it, or `None`
/// where it moved none of its ALGO. Algorand commits no transaction that
/// fails, so every one is confirmed.
fn history_entry(transaction: &Value, address: &str) -> Result<Option<HistoryEntry>, ApiError> {
    let txid = transaction["id"]
        .as_str()
        .or_decode("Algorand history: missing id")?;
    let mut movement = Movement::default();
    movement.add(transaction, address)?;
    if movement.received == movement.sent {
        return Ok(None);
    }
    Ok(Some(HistoryEntry {
        txid: txid.to_string(),
        contract: None,
        amount: HistoryAmount::Units(movement.received.abs_diff(movement.sent)),
        incoming: movement.received > movement.sent,
        outcome: HistoryOutcome::Confirmed,
        block_height: transaction["confirmed-round"]
            .as_u64()
            .and_then(|round| i64::try_from(round).ok()),
        timestamp: transaction["round-time"].as_u64().map(|time| time as f64),
        counterparty: movement.counterparty,
    }))
}

#[cfg(test)]
#[path = "tests/algorand_indexer.rs"]
mod tests;

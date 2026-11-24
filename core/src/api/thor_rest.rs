//! The REST API a VeChainThor node serves: the node's genesis, the best
//! block and its base fee, an account's VET and VTHO, VIP-180 contract
//! reads, clause simulation, the suggested priority fee, submission of a
//! signed transaction, its receipt, and an address's history from the
//! node's own transfer and event logs.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::api::error::{ApiError, OrDecode};
use crate::api::history_page::{HistoryAmount, HistoryEntry, HistoryOutcome, HistoryPage};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::transaction_status::TransactionStatus;
use crate::registry::Chain;

/// `Transfer(address,address,uint256)`, the event every VIP-180 transfer
/// emits.
pub(crate) const TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// How many log entries one history request asks for, per kind; a node
/// answers at most 1,000.
const HISTORY_PAGE_SIZE: u32 = 50;

/// The block a transaction is built on: its number, its id, whose first
/// eight bytes are a transaction's block reference, and its base fee, which
/// every block carries since the Galactica fork.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThorBlock {
    pub number: u32,
    pub id: [u8; 32],
    pub base_fee_per_gas: Option<u128>,
}

/// An account's VET and VTHO, in wei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThorAccount {
    pub balance: u128,
    pub energy: u128,
}

/// One clause, as `POST /accounts/*` and a transaction carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThorCall<'a> {
    pub to: &'a str,
    pub value: u128,
    pub data: &'a [u8],
}

impl ThorCall<'_> {
    fn json(&self) -> Value {
        json!({
            "to": self.to,
            "value": format!("0x{:x}", self.value),
            "data": format!("0x{}", hex::encode(self.data)),
        })
    }
}

/// What a node's execution of one clause returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThorCallResult {
    pub data: Vec<u8>,
    pub gas_used: u64,
    pub reverted: bool,
    pub vm_error: String,
}

pub struct ThorClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl ThorClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    async fn post(
        &self,
        path: &str,
        body: &Value,
        profile: RetryProfile,
    ) -> Result<Value, ApiError> {
        let body = Arc::new(body.clone());
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = Arc::clone(&body);
            async move { client.post_json(&url, &*body, profile).await }
        })
        .await
    }

    /// Refuse a node that is not on `chain`: its block 0 must be the
    /// genesis whose last byte every transaction's chain tag repeats.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let expected = chain.thor_network()?.genesis_id;
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            async move {
                let block: Value = client
                    .get_json(
                        &format!("{}/blocks/0", base.trim_end_matches('/')),
                        RetryProfile::ChainRead,
                    )
                    .await?;
                verify_genesis(&block, expected)
            }
        })
        .await
    }

    /// The newest block the node has.
    pub(crate) async fn fetch_best_block(&self) -> Result<ThorBlock, ApiError> {
        block_from(&self.get("/blocks/best").await?)
    }

    /// The address's VET and VTHO.
    pub(crate) async fn fetch_account(&self, address: &str) -> Result<ThorAccount, ApiError> {
        account_from(&self.get(&format!("/accounts/{address}")).await?)
    }

    /// Run `clauses` in order on top of the best block, as `caller` would.
    /// A node stops at the first clause that fails, so fewer results than
    /// clauses can come back.
    async fn call(
        &self,
        clauses: &[ThorCall<'_>],
        caller: Option<&str>,
        revision: &str,
    ) -> Result<Vec<ThorCallResult>, ApiError> {
        let mut body = json!({ "clauses": clauses.iter().map(ThorCall::json).collect::<Vec<_>>() });
        if let Some(caller) = caller {
            body["caller"] = json!(caller);
        }
        let response = match self
            .post(
                &format!("/accounts/*?revision={revision}"),
                &body,
                RetryProfile::ChainRead,
            )
            .await
        {
            Ok(response) => response,
            Err(ApiError::Status { status, body }) if status < 500 => {
                return Err(ApiError::rejected(format!(
                    "VeChainThor node refused the call: {}",
                    body.trim()
                )));
            }
            Err(error) => return Err(error),
        };
        let results = call_results(&response)?;
        if results.len() > clauses.len() {
            return Err(ApiError::decode(
                "VeChainThor call: more results than clauses",
            ));
        }
        Ok(results)
    }

    /// Each clause's result, refused unless every clause ran and none
    /// reverted.
    async fn read(&self, clauses: &[ThorCall<'_>]) -> Result<Vec<Vec<u8>>, ApiError> {
        let results = self.call(clauses, None, "best").await?;
        if results.len() != clauses.len() || results.iter().any(|result| result.reverted) {
            return Err(ApiError::rejected(format!(
                "VeChainThor contract call reverted: {}",
                results
                    .iter()
                    .map(|result| result.vm_error.as_str())
                    .find(|error| !error.is_empty())
                    .unwrap_or("no result")
            )));
        }
        Ok(results.into_iter().map(|result| result.data).collect())
    }

    /// What `holder` holds of the VIP-180 token `contract`, and the token's
    /// own `decimals()`, read together.
    pub(crate) async fn fetch_token_balance(
        &self,
        contract: &str,
        holder: &str,
    ) -> Result<(u128, u8), ApiError> {
        let balance_of = balance_of_call(holder)?;
        let words = self
            .read(&[
                ThorCall {
                    to: contract,
                    value: 0,
                    data: &balance_of,
                },
                ThorCall {
                    to: contract,
                    value: 0,
                    data: &crate::api::evm_json_rpc::SEL_DECIMALS,
                },
            ])
            .await?;
        Ok((abi_word(&words[0])?, decimals_word(&words[1])?))
    }

    /// The VIP-180 token `contract`'s own `decimals()`.
    pub(crate) async fn fetch_token_decimals(&self, contract: &str) -> Result<u8, ApiError> {
        let words = self
            .read(&[ThorCall {
                to: contract,
                value: 0,
                data: &crate::api::evm_json_rpc::SEL_DECIMALS,
            }])
            .await?;
        decimals_word(&words[0])
    }

    /// The gas the virtual machine uses running `clause` from `caller` in the
    /// next block, intrinsic gas aside. Refused when it reverts: the
    /// transaction would only pay its fee.
    pub(crate) async fn simulate(
        &self,
        caller: &str,
        clause: ThorCall<'_>,
    ) -> Result<u64, ApiError> {
        let results = self.call(&[clause], Some(caller), "next").await?;
        let result = results
            .first()
            .or_decode("VeChainThor simulation: no result")?;
        if result.reverted || !result.vm_error.is_empty() {
            return Err(ApiError::rejected(format!(
                "The node's simulation of this transfer failed: {}",
                if result.vm_error.is_empty() {
                    "reverted"
                } else {
                    &result.vm_error
                }
            )));
        }
        Ok(result.gas_used)
    }

    /// The priority fee per gas the node suggests for inclusion now.
    pub(crate) async fn fetch_priority_fee(&self) -> Result<u128, ApiError> {
        quantity(
            self.get("/fees/priority").await?["maxPriorityFeePerGas"]
                .as_str()
                .or_decode("VeChainThor fees: missing priority fee")?,
        )
    }

    /// Submit a signed transaction, `0x`-prefixed hex, and answer the id the
    /// node accepted it under. A node that refuses it says why in its body.
    pub(crate) async fn broadcast(&self, raw: &str) -> Result<String, ApiError> {
        let response = match self
            .post(
                "/transactions",
                &json!({ "raw": raw }),
                RetryProfile::ChainWrite,
            )
            .await
        {
            Ok(response) => response,
            Err(ApiError::Status { status, body }) if status < 500 => {
                return Err(ApiError::rejected(format!(
                    "VeChainThor node refused the transaction: {}",
                    body.trim()
                )));
            }
            Err(error) => return Err(error),
        };
        transaction_id(
            response["id"]
                .as_str()
                .or_decode("VeChainThor broadcast: missing transaction id")?,
        )
    }

    /// Whether the transaction `id` has been included, and whether its
    /// clauses ran or it reverted, paying only its fee. One the node has no
    /// receipt for is still pending.
    pub(crate) async fn fetch_transaction_status(
        &self,
        id: &str,
    ) -> Result<TransactionStatus, ApiError> {
        let id = transaction_id(id)?;
        receipt_status(
            &self.get(&format!("/transactions/{id}/receipt")).await?,
            &id,
        )
    }

    /// One page of `address`'s history, newest first: its VET transfers from
    /// the node's transfer log, and its transfers of `tokens` (VIP-180
    /// contracts) from their `Transfer` events. Each source is read to the
    /// block the first page stopped at, so later blocks do not move the
    /// offsets of the pages after it. A reverted clause logs nothing, so
    /// every entry here succeeded.
    pub(crate) async fn fetch_history_page(
        &self,
        address: &str,
        tokens: &[String],
        cursor: Option<&str>,
    ) -> Result<HistoryPage<HistoryEntry>, ApiError> {
        let address = address.to_ascii_lowercase();
        let mut tokens: Vec<String> = tokens.iter().map(|t| t.to_ascii_lowercase()).collect();
        tokens.sort();
        tokens.dedup();
        let mut position: HistoryCursor = cursor
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| ApiError::InvalidInput("invalid history cursor".into()))?
            .unwrap_or_default();
        let to = match position.to {
            Some(to) => to,
            None => self.fetch_best_block().await?.number,
        };
        position.to = Some(to);
        // Another list of tokens is another query: its offsets start over,
        // and what the earlier pages read again is merged by its id.
        if position.tokens != tokens {
            position.tokens = tokens.clone();
            position.events = 0;
            position.events_done = false;
        }
        // No criteria would match every event on the chain.
        position.events_done |= tokens.is_empty();
        let range = json!({ "unit": "block", "from": 0, "to": to });
        let mut items = Vec::new();
        if !position.transfers_done {
            let logs = self
                .post(
                    "/logs/transfer",
                    &json!({
                        "range": range,
                        "options": { "offset": position.transfers, "limit": HISTORY_PAGE_SIZE },
                        "criteriaSet": [{ "sender": address }, { "recipient": address }],
                        "order": "desc",
                    }),
                    RetryProfile::ChainRead,
                )
                .await?;
            let logs = page(&logs)?;
            items.extend(vet_entries(logs, &address)?);
            position.transfers += logs.len() as u32;
            position.transfers_done = logs.len() < HISTORY_PAGE_SIZE as usize;
        }
        if !position.events_done {
            let topic = address_topic(&address)?;
            let criteria: Vec<Value> = tokens
                .iter()
                .flat_map(|token| {
                    [
                        json!({ "address": token, "topic0": TRANSFER_TOPIC, "topic1": topic }),
                        json!({ "address": token, "topic0": TRANSFER_TOPIC, "topic2": topic }),
                    ]
                })
                .collect();
            let logs = self
                .post(
                    "/logs/event",
                    &json!({
                        "range": range,
                        "options": { "offset": position.events, "limit": HISTORY_PAGE_SIZE },
                        "criteriaSet": criteria,
                        "order": "desc",
                    }),
                    RetryProfile::ChainRead,
                )
                .await?;
            let logs = page(&logs)?;
            let transfers = token_transfers(logs, &address, &tokens)?;
            let mut decimals = HashMap::new();
            for contract in transfers.iter().map(|transfer| &transfer.contract) {
                if !decimals.contains_key(contract) {
                    decimals.insert(contract.clone(), self.fetch_token_decimals(contract).await?);
                }
            }
            items.extend(token_entries(transfers, &address, &decimals)?);
            position.events += logs.len() as u32;
            position.events_done = logs.len() < HISTORY_PAGE_SIZE as usize;
        }
        let next_cursor = (!(position.transfers_done && position.events_done))
            .then(|| serde_json::to_string(&position))
            .transpose()
            .map_err(ApiError::invalid)?;
        Ok(HistoryPage { items, next_cursor })
    }
}

/// Where an address's history has been read to: the block every page stops
/// at, and the offset into each log.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct HistoryCursor {
    to: Option<u32>,
    transfers: u32,
    transfers_done: bool,
    /// The token contracts the event offsets were counted over.
    tokens: Vec<String>,
    events: u32,
    events_done: bool,
}

/// One page of a log: a list no longer than was asked for.
fn page(logs: &Value) -> Result<&[Value], ApiError> {
    let logs = logs.as_array().or_decode("VeChainThor logs: not a list")?;
    if logs.len() > HISTORY_PAGE_SIZE as usize {
        return Err(ApiError::decode(
            "VeChainThor logs: more entries than asked for",
        ));
    }
    Ok(logs)
}

/// A hex quantity as the node writes one: `0x` and up to 32 hex digits.
pub(crate) fn quantity(text: &str) -> Result<u128, ApiError> {
    let digits = text
        .strip_prefix("0x")
        .filter(|digits| {
            !digits.is_empty()
                && digits.len() <= 32
                && digits.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .or_decode("VeChainThor: not a hex quantity")?;
    u128::from_str_radix(digits, 16)
        .map_err(|_| ApiError::decode("VeChainThor: not a hex quantity"))
}

/// A transaction id, `0x` and 64 hex digits, lowercased.
pub(crate) fn transaction_id(text: &str) -> Result<String, ApiError> {
    let digits = text.strip_prefix("0x").unwrap_or_default();
    if digits.len() != 64 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::invalid("Invalid VeChainThor transaction id"));
    }
    Ok(format!("0x{}", digits.to_ascii_lowercase()))
}

fn verify_genesis(block: &Value, expected: &str) -> Result<(), ApiError> {
    let id = block["id"]
        .as_str()
        .or_decode("VeChainThor block 0: missing id")?;
    if block["number"].as_u64() != Some(0) || !id.eq_ignore_ascii_case(expected) {
        return Err(ApiError::invalid(format!(
            "Endpoint's genesis is {id}, not VeChainThor's {expected}"
        )));
    }
    Ok(())
}

fn block_from(block: &Value) -> Result<ThorBlock, ApiError> {
    let number = block["number"]
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .or_decode("VeChainThor block: missing number")?;
    let id: [u8; 32] = crate::api::evm_json_rpc::decode_hex(
        block["id"]
            .as_str()
            .or_decode("VeChainThor block: missing id")?,
    )?
    .try_into()
    .map_err(|_| ApiError::decode("VeChainThor block: id is not 32 bytes"))?;
    // A block id starts with its number.
    if id[..4] != number.to_be_bytes() {
        return Err(ApiError::decode(
            "VeChainThor block: id does not carry its number",
        ));
    }
    let base_fee_per_gas = match &block["baseFeePerGas"] {
        Value::Null => None,
        fee => Some(quantity(
            fee.as_str().or_decode("VeChainThor block: bad base fee")?,
        )?),
    };
    Ok(ThorBlock {
        number,
        id,
        base_fee_per_gas,
    })
}

fn account_from(account: &Value) -> Result<ThorAccount, ApiError> {
    let amount = |field: &str| {
        quantity(
            account[field]
                .as_str()
                .or_decode("VeChainThor account: missing balance")?,
        )
    };
    Ok(ThorAccount {
        balance: amount("balance")?,
        energy: amount("energy")?,
    })
}

fn call_results(response: &Value) -> Result<Vec<ThorCallResult>, ApiError> {
    response
        .as_array()
        .or_decode("VeChainThor call: not a list")?
        .iter()
        .map(|result| {
            Ok(ThorCallResult {
                data: crate::api::evm_json_rpc::decode_hex(
                    result["data"]
                        .as_str()
                        .or_decode("VeChainThor call: missing data")?,
                )?,
                gas_used: result["gasUsed"]
                    .as_u64()
                    .or_decode("VeChainThor call: missing gas used")?,
                reverted: result["reverted"]
                    .as_bool()
                    .or_decode("VeChainThor call: missing reverted")?,
                vm_error: result["vmError"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// `balanceOf(holder)`.
fn balance_of_call(holder: &str) -> Result<Vec<u8>, ApiError> {
    let mut data = crate::api::evm_json_rpc::SEL_BALANCE_OF.to_vec();
    data.extend([0u8; 12]);
    data.extend(address_bytes(holder)?);
    Ok(data)
}

fn address_bytes(address: &str) -> Result<[u8; 20], ApiError> {
    crate::api::evm_json_rpc::decode_hex(address)?
        .try_into()
        .map_err(|_| ApiError::invalid("Not a VeChainThor address"))
}

/// An address as an indexed event argument: left-padded to 32 bytes.
fn address_topic(address: &str) -> Result<String, ApiError> {
    Ok(format!(
        "0x{}{}",
        "0".repeat(24),
        hex::encode(address_bytes(address)?)
    ))
}

/// One ABI word as an amount.
fn abi_word(word: &[u8]) -> Result<u128, ApiError> {
    crate::api::evm_json_rpc::abi_u128(&hex::encode(word))
}

fn decimals_word(word: &[u8]) -> Result<u8, ApiError> {
    crate::api::checked_token_decimals(abi_word(word)?)
}

/// The address a topic holds: its last 20 bytes, under 12 zero bytes.
fn topic_address(topic: &str) -> Result<String, ApiError> {
    let bytes = crate::api::evm_json_rpc::decode_hex(topic)?;
    if bytes.len() != 32 || bytes[..12].iter().any(|b| *b != 0) {
        return Err(ApiError::decode(
            "VeChainThor event: topic is not an address",
        ));
    }
    Ok(format!("0x{}", hex::encode(&bytes[12..])))
}

fn receipt_status(receipt: &Value, id: &str) -> Result<TransactionStatus, ApiError> {
    if receipt.is_null() {
        return Ok(TransactionStatus::Pending);
    }
    let meta = &receipt["meta"];
    if !meta["txID"]
        .as_str()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(id))
    {
        return Err(ApiError::decode(
            "VeChainThor receipt: transaction id mismatch",
        ));
    }
    Ok(TransactionStatus::Confirmed {
        succeeded: !receipt["reverted"]
            .as_bool()
            .or_decode("VeChainThor receipt: missing reverted")?,
        block: Some(
            meta["blockNumber"]
                .as_u64()
                .or_decode("VeChainThor receipt: missing block number")?,
        ),
    })
}

/// Where a log was written: its transaction, block and time.
struct LogMeta {
    txid: String,
    block: i64,
    timestamp: u64,
}

fn log_meta(log: &Value) -> Result<LogMeta, ApiError> {
    let meta = &log["meta"];
    let txid = transaction_id(
        meta["txID"]
            .as_str()
            .or_decode("VeChainThor log: missing txID")?,
    )?;
    let timestamp =
        crate::api::time::confirmed_history_time(meta["blockTimestamp"].as_u64(), &txid)?;
    Ok(LogMeta {
        block: meta["blockNumber"]
            .as_i64()
            .or_decode("VeChainThor log: missing block number")?,
        txid,
        timestamp,
    })
}

/// A transaction's moves of one asset to and from the address, netted: a
/// transaction may carry several clauses.
struct Netted {
    meta: LogMeta,
    contract: Option<String>,
    received: u128,
    sent: u128,
    counterparty: String,
}

impl Netted {
    fn add(&mut self, from: &str, to: &str, amount: u128, address: &str) -> Result<(), ApiError> {
        let overflow = || ApiError::decode("VeChainThor history: amount overflows");
        if to == address && from != address {
            self.received = self.received.checked_add(amount).ok_or_else(overflow)?;
            self.counterparty = from.to_string();
        } else if from == address && to != address {
            self.sent = self.sent.checked_add(amount).ok_or_else(overflow)?;
            self.counterparty = to.to_string();
        }
        Ok(())
    }

    fn entry(self, amount: impl FnOnce(u128) -> HistoryAmount) -> Option<HistoryEntry> {
        if self.received == self.sent {
            return None;
        }
        Some(HistoryEntry {
            txid: self.meta.txid,
            contract: self.contract,
            amount: amount(self.received.abs_diff(self.sent)),
            incoming: self.received > self.sent,
            outcome: HistoryOutcome::Confirmed,
            block_height: Some(self.meta.block),
            timestamp: Some(self.meta.timestamp as f64),
            counterparty: self.counterparty,
        })
    }
}

/// Add one move to the transaction's netted entry, keeping the order the
/// node listed them in.
fn net(
    netted: &mut Vec<Netted>,
    meta: LogMeta,
    contract: Option<String>,
    (from, to, amount): (&str, &str, u128),
    address: &str,
) -> Result<(), ApiError> {
    let index = match netted
        .iter()
        .position(|entry| entry.meta.txid == meta.txid && entry.contract == contract)
    {
        Some(index) => index,
        None => {
            netted.push(Netted {
                meta,
                contract,
                received: 0,
                sent: 0,
                counterparty: String::new(),
            });
            netted.len() - 1
        }
    };
    netted[index].add(from, to, amount, address)
}

/// The address's VET transfers, one entry per transaction.
fn vet_entries(logs: &[Value], address: &str) -> Result<Vec<HistoryEntry>, ApiError> {
    let mut netted = Vec::new();
    for log in logs {
        let party = |field: &str| {
            log[field]
                .as_str()
                .map(str::to_ascii_lowercase)
                .or_decode("VeChainThor transfer log: missing party")
        };
        let (sender, recipient) = (party("sender")?, party("recipient")?);
        if sender != address && recipient != address {
            return Err(ApiError::decode(
                "VeChainThor transfer log: another address's",
            ));
        }
        let amount = quantity(
            log["amount"]
                .as_str()
                .or_decode("VeChainThor transfer log: missing amount")?,
        )?;
        net(
            &mut netted,
            log_meta(log)?,
            None,
            (&sender, &recipient, amount),
            address,
        )?;
    }
    Ok(netted
        .into_iter()
        .filter_map(|entry| entry.entry(HistoryAmount::Units))
        .collect())
}

/// One token transfer from a `Transfer` event.
struct TokenTransfer {
    meta: LogMeta,
    contract: String,
    from: String,
    to: String,
    amount: u128,
}

/// The `Transfer` events of `tokens` that moved the address's tokens: three
/// topics and a 32-byte amount, as VIP-180 writes them.
fn token_transfers(
    logs: &[Value],
    address: &str,
    tokens: &[String],
) -> Result<Vec<TokenTransfer>, ApiError> {
    logs.iter()
        .map(|log| {
            let contract = log["address"]
                .as_str()
                .or_decode("VeChainThor event: missing address")?
                .to_ascii_lowercase();
            let topics = log["topics"]
                .as_array()
                .or_decode("VeChainThor event: missing topics")?;
            let topic = |index: usize| {
                topics[index]
                    .as_str()
                    .or_decode("VeChainThor event: bad topic")
            };
            if !tokens.contains(&contract)
                || topics.len() != 3
                || !topic(0)?.eq_ignore_ascii_case(TRANSFER_TOPIC)
            {
                return Err(ApiError::decode(
                    "VeChainThor event: not a token transfer asked for",
                ));
            }
            let (from, to) = (topic_address(topic(1)?)?, topic_address(topic(2)?)?);
            if from != address && to != address {
                return Err(ApiError::decode(
                    "VeChainThor event: another address's transfer",
                ));
            }
            let data = crate::api::evm_json_rpc::decode_hex(
                log["data"]
                    .as_str()
                    .or_decode("VeChainThor event: missing data")?,
            )?;
            if data.len() != 32 {
                return Err(ApiError::decode(
                    "VeChainThor event: amount is not one word",
                ));
            }
            Ok(TokenTransfer {
                meta: log_meta(log)?,
                contract,
                from,
                to,
                amount: abi_word(&data)?,
            })
        })
        .collect()
}

/// The token transfers, one entry per transaction and token, at each
/// token's own decimals.
fn token_entries(
    transfers: Vec<TokenTransfer>,
    address: &str,
    decimals: &HashMap<String, u8>,
) -> Result<Vec<HistoryEntry>, ApiError> {
    let mut netted = Vec::new();
    for transfer in transfers {
        net(
            &mut netted,
            transfer.meta,
            Some(transfer.contract),
            (&transfer.from, &transfer.to, transfer.amount),
            address,
        )?;
    }
    netted
        .into_iter()
        .filter_map(|entry| {
            let Some(places) = entry.contract.as_ref().and_then(|c| decimals.get(c)) else {
                return Some(Err(ApiError::decode(
                    "VeChainThor history: token decimals unread",
                )));
            };
            let places = u32::from(*places);
            entry
                .entry(|units| {
                    HistoryAmount::Decimal(crate::amount::decimal::from_units(units, places))
                })
                .map(Ok)
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/thor_rest.rs"]
mod tests;

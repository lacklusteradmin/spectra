//! The Cosmos SDK REST adapter (the gRPC gateway a node serves): the node's
//! network, an account's number and sequence, balances, gas prices,
//! simulation, submission of a signed transaction, its status and an
//! address's history.

use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::api::error::{ApiError, OrDecode};
use crate::api::history_page::{HistoryAmount, HistoryEntry, HistoryOutcome, HistoryPage};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::transaction_status::TransactionStatus;
use crate::registry::{Chain, CosmosFeeMarket, CosmosTxSearch, GasPrice};

/// An account's number and its next sequence, which a sign document binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CosmosAccount {
    pub account_number: u64,
    pub sequence: u64,
}

/// How many transactions one history request asks for, per direction.
const HISTORY_PAGE_SIZE: u32 = 50;

pub struct CosmosClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

/// The header of a `blocks/latest` answer: `sdk_block`'s where the SDK
/// writes one (0.47 and later), else `block`'s.
fn latest_header(block: &Value) -> &Value {
    if block["sdk_block"]["header"].is_object() {
        &block["sdk_block"]["header"]
    } else {
        &block["block"]["header"]
    }
}

impl CosmosClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::GatewayRead).await }
        })
        .await
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

    /// Refuse a node that is not on `chain`: its newest block must name the
    /// chain id every sign document binds. Every gateway serves the block;
    /// not every one serves `node_info`.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let expected = chain.cosmos_network()?.chain_id;
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            async move {
                let block: Value = client
                    .get_json(
                        &format!(
                            "{}/cosmos/base/tendermint/v1beta1/blocks/latest",
                            base.trim_end_matches('/')
                        ),
                        RetryProfile::GatewayRead,
                    )
                    .await?;
                let network = latest_header(&block)["chain_id"]
                    .as_str()
                    .or_decode("Cosmos block: missing chain id")?;
                if network != expected {
                    return Err(ApiError::invalid(format!(
                        "Endpoint is on {network}, not {expected}"
                    )));
                }
                Ok(())
            }
        })
        .await
    }

    /// The height of the newest block the node has.
    pub(crate) async fn fetch_latest_height(&self) -> Result<u64, ApiError> {
        let block = self
            .get("/cosmos/base/tendermint/v1beta1/blocks/latest")
            .await?;
        latest_header(&block)["height"]
            .as_str()
            .and_then(|height| height.parse().ok())
            .or_decode("Cosmos block: missing height")
    }

    /// The address's balance of `denom`, spendable or in all.
    async fn fetch_denom_balance(
        &self,
        route: &str,
        address: &str,
        denom: &str,
    ) -> Result<u128, ApiError> {
        let response = self
            .get(&format!(
                "/cosmos/bank/v1beta1/{route}/{address}/by_denom?denom={}",
                crate::api::history_page::query_value(denom)
            ))
            .await?;
        let balance = &response["balance"];
        if balance.is_null() {
            return Ok(0);
        }
        if balance["denom"].as_str() != Some(denom) {
            return Err(ApiError::decode("Cosmos balance: another denomination"));
        }
        balance["amount"]
            .as_str()
            .and_then(|amount| amount.parse().ok())
            .or_decode("Cosmos balance: missing amount")
    }

    /// Everything the address holds of the network's own coin, vesting
    /// included: what its balance shows.
    pub(crate) async fn fetch_balance(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<u128, ApiError> {
        let denom = chain.cosmos_network()?.denom;
        self.fetch_denom_balance("balances", address, denom).await
    }

    /// What the address can spend of the network's own coin now: its
    /// balance less what is still vesting.
    pub(crate) async fn fetch_spendable(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<u128, ApiError> {
        let denom = chain.cosmos_network()?.denom;
        self.fetch_denom_balance("spendable_balances", address, denom)
            .await
    }

    /// The account's number and sequence, or `None` while the chain has no
    /// account at the address: it has never received anything.
    pub(crate) async fn fetch_account(
        &self,
        address: &str,
    ) -> Result<Option<CosmosAccount>, ApiError> {
        let response = match self
            .get(&format!("/cosmos/auth/v1beta1/accounts/{address}"))
            .await
        {
            Ok(response) => response,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        account_from(&response["account"], address).map(Some)
    }

    /// The gas a transaction uses, from a node's simulation of it.
    pub(crate) async fn simulate(&self, tx_bytes: &[u8]) -> Result<u64, ApiError> {
        let body = json!({
            "tx_bytes": base64::engine::general_purpose::STANDARD.encode(tx_bytes),
        });
        let response = match self
            .post(
                "/cosmos/tx/v1beta1/simulate",
                &body,
                RetryProfile::GatewayRead,
            )
            .await
        {
            Ok(response) => response,
            Err(ApiError::Status { body, .. }) => {
                return Err(ApiError::rejected(node_message(&body)));
            }
            Err(error) => return Err(error),
        };
        response["gas_info"]["gas_used"]
            .as_str()
            .and_then(|gas| gas.parse().ok())
            .filter(|gas: &u64| *gas > 0)
            .or_decode("Cosmos simulation: missing gas used")
    }

    /// The current minimum price of gas where `chain` runs a fee market, in
    /// its own coin's base units; `None` where validators fix it.
    pub(crate) async fn fetch_market_gas_price(
        &self,
        chain: Chain,
    ) -> Result<Option<GasPrice>, ApiError> {
        let network = chain.cosmos_network()?;
        let price = match network.fee_market {
            None => return Ok(None),
            Some(CosmosFeeMarket::Skip) => {
                let response = self
                    .get(&format!("/feemarket/v1/gas_price/{}", network.denom))
                    .await?;
                if response["price"]["denom"].as_str() != Some(network.denom) {
                    return Err(ApiError::decode("Cosmos fee market: another denomination"));
                }
                response["price"]["amount"].as_str().map(str::to_string)
            }
            Some(CosmosFeeMarket::OsmosisTxFees) => {
                self.get("/osmosis/txfees/v1beta1/cur_eip_base_fee").await?["base_fee"]
                    .as_str()
                    .map(str::to_string)
            }
            Some(CosmosFeeMarket::InjectiveTxFees) => self
                .get("/injective/txfees/v1beta1/cur_eip_base_fee")
                .await?["base_fee"]["base_fee"]
                .as_str()
                .map(str::to_string),
        };
        let price = price.or_decode("Cosmos fee market: missing price")?;
        decimal_gas_price(&price)
            .map(Some)
            .or_decode("Cosmos fee market: price is not a decimal")
    }

    /// Submit a signed `TxRaw` and answer the hash the node accepted it
    /// under. A node that runs the transaction's checks and refuses it
    /// answers a non-zero code, which is a rejection.
    pub(crate) async fn broadcast(&self, tx_bytes_b64: &str) -> Result<String, ApiError> {
        let body = json!({ "tx_bytes": tx_bytes_b64, "mode": "BROADCAST_MODE_SYNC" });
        let response = self
            .post("/cosmos/tx/v1beta1/txs", &body, RetryProfile::ChainWrite)
            .await?;
        let tx = &response["tx_response"];
        let code = tx["code"]
            .as_u64()
            .or_decode("Cosmos broadcast: missing code")?;
        if code != 0 {
            return Err(ApiError::rejected(format!(
                "Cosmos node refused the transaction (code {code}): {}",
                tx["raw_log"].as_str().unwrap_or_default()
            )));
        }
        let hash = tx["txhash"]
            .as_str()
            .or_decode("Cosmos broadcast: missing transaction hash")?;
        crate::api::transaction_status::validate_hex_hash(hash)?;
        Ok(hash.to_ascii_uppercase())
    }

    /// Whether the transaction `hash` has been included, and whether it ran.
    /// One the node does not know is still pending.
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<TransactionStatus, ApiError> {
        crate::api::transaction_status::validate_hex_hash(hash)?;
        let response = match self.get(&format!("/cosmos/tx/v1beta1/txs/{hash}")).await {
            Ok(response) => response,
            Err(error) if is_not_found(&error) => return Ok(TransactionStatus::Pending),
            Err(error) => return Err(error),
        };
        let tx = &response["tx_response"];
        if !tx["txhash"]
            .as_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(hash))
        {
            return Err(ApiError::decode("Cosmos status: transaction hash mismatch"));
        }
        let block = tx["height"]
            .as_str()
            .and_then(|height| height.parse().ok())
            .filter(|height: &u64| *height > 0)
            .or_decode("Cosmos status: missing height")?;
        Ok(TransactionStatus::Confirmed {
            succeeded: tx["code"]
                .as_u64()
                .or_decode("Cosmos status: missing code")?
                == 0,
            block: Some(block),
        })
    }

    /// One page of the address's history: the transactions it signed and
    /// those that paid it, newest first, each as what it did to the
    /// address's balance of the network's own coin.
    pub(crate) async fn fetch_history_page(
        &self,
        chain: Chain,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<HistoryPage<HistoryEntry>, ApiError> {
        let network = chain.cosmos_network()?;
        let mut position: HistoryCursor = cursor
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| ApiError::InvalidInput("invalid history cursor".into()))?
            .unwrap_or_default();
        let mut items: Vec<HistoryEntry> = Vec::new();
        for (query, done, page) in [
            (
                format!("message.sender='{address}'"),
                &mut position.sent_done,
                &mut position.sent_page,
            ),
            (
                format!("transfer.recipient='{address}'"),
                &mut position.received_done,
                &mut position.received_page,
            ),
        ] {
            if *done {
                continue;
            }
            let next = page.saturating_add(1);
            // Newest first: a node that indexes transactions older than the
            // blocks it keeps fails a search that starts from the oldest.
            let parameter = match network.tx_search {
                CosmosTxSearch::Query => "query",
                CosmosTxSearch::Events => "events",
            };
            let response = self
                .get(&format!(
                    "/cosmos/tx/v1beta1/txs?{parameter}={}&page={next}&limit={HISTORY_PAGE_SIZE}&order_by=ORDER_BY_DESC",
                    crate::api::history_page::query_value(&query)
                ))
                .await?;
            let responses = response["tx_responses"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for tx in &responses {
                if let Some(entry) = history_entry(tx, address, network.denom)?
                    && !items.iter().any(|item| item.txid == entry.txid)
                {
                    items.push(entry);
                }
            }
            *page = next;
            // Some nodes answer pages shorter than asked for and count index
            // hits rather than transactions in `total`: only an empty page
            // ends a direction.
            *done = responses.is_empty();
        }
        let next_cursor = (!(position.sent_done && position.received_done))
            .then(|| serde_json::to_string(&position))
            .transpose()
            .map_err(ApiError::invalid)?;
        Ok(HistoryPage { items, next_cursor })
    }
}

/// Where an address's history has been read to, in each direction.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct HistoryCursor {
    sent_page: u32,
    sent_done: bool,
    received_page: u32,
    received_done: bool,
}

/// Whether a node answered that what was asked for does not exist: an
/// account that never received anything, a transaction not yet included.
fn is_not_found(error: &ApiError) -> bool {
    match error {
        ApiError::Status { status: 404, .. } => true,
        ApiError::Status { body, .. } => {
            let body = body.to_ascii_lowercase();
            body.contains("not found") || body.contains("\"code\":5")
        }
        _ => false,
    }
}

/// The message a node's error body carries, or the body itself.
fn node_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string())
}

/// A gas price written as a decimal (`"0.005000000000000000"`) as an exact
/// fraction of base units per gas.
pub(crate) fn decimal_gas_price(text: &str) -> Option<GasPrice> {
    let (whole, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    let fraction = fraction.trim_end_matches('0');
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 30
    {
        return None;
    }
    let denominator = 10u128.checked_pow(fraction.len() as u32)?;
    let numerator: u128 = format!("{whole}{fraction}").parse().ok()?;
    Some(GasPrice {
        numerator,
        denominator,
    })
}

/// An account's number and sequence, wherever its type keeps them: a base
/// account at the top, inside Ethermint's and Injective's `EthAccount`, or
/// at the bottom of a vesting account. A module account cannot sign.
fn account_from(account: &Value, address: &str) -> Result<CosmosAccount, ApiError> {
    let base = [
        account,
        &account["base_account"],
        &account["base_vesting_account"]["base_account"],
    ]
    .into_iter()
    .find(|candidate| candidate["account_number"].is_string())
    .or_decode("Cosmos account: no base account")?;
    if !base["address"]
        .as_str()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(address))
    {
        return Err(ApiError::decode("Cosmos account: another address"));
    }
    let number = |field: &str| {
        base[field]
            .as_str()
            .and_then(|value| value.parse().ok())
            .or_decode("Cosmos account: missing number or sequence")
    };
    Ok(CosmosAccount {
        account_number: number("account_number")?,
        sequence: number("sequence")?,
    })
}

/// The amount of `denom` an SDK coin list (`"1000uatom,5ibc/…"`) holds.
fn coins_of(list: &str, denom: &str) -> Result<u128, ApiError> {
    let mut total: u128 = 0;
    for coin in list
        .split(',')
        .map(str::trim)
        .filter(|coin| !coin.is_empty())
    {
        let split = coin
            .find(|c: char| !c.is_ascii_digit())
            .or_decode("Cosmos coin: no denomination")?;
        let (amount, coin_denom) = coin.split_at(split);
        if coin_denom == denom {
            let amount: u128 = amount
                .parse()
                .map_err(|_| ApiError::decode("Cosmos coin: bad amount"))?;
            total = total
                .checked_add(amount)
                .or_decode("Cosmos coin: amount overflows")?;
        }
    }
    Ok(total)
}

/// The `transfer` events the transaction's messages emitted, each as its
/// attributes: an SDK 0.50 node lists them flat with a `msg_index` (the fee
/// payment has none), an older one under each message's log.
fn message_transfers(tx: &Value) -> Vec<std::collections::HashMap<String, String>> {
    let attributes = |event: &Value| -> std::collections::HashMap<String, String> {
        event["attributes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|attribute| {
                Some((
                    attribute["key"].as_str()?.to_string(),
                    attribute["value"].as_str().unwrap_or_default().to_string(),
                ))
            })
            .collect()
    };
    let flat: Vec<_> = tx["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["type"] == "transfer")
        .map(attributes)
        .filter(|attributes| attributes.contains_key("msg_index"))
        .collect();
    if !flat.is_empty() {
        return flat;
    }
    tx["logs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|log| log["events"].as_array().cloned().unwrap_or_default())
        .filter(|event| event["type"] == "transfer")
        .map(|event| attributes(&event))
        .collect()
}

/// What one transaction did to `address`'s balance of `denom`: the coins
/// its messages moved to and from the address, the fee aside. A failed
/// transaction moved nothing but its fee; it is shown with the amount its
/// sends asked for.
fn history_entry(tx: &Value, address: &str, denom: &str) -> Result<Option<HistoryEntry>, ApiError> {
    let txid = tx["txhash"]
        .as_str()
        .or_decode("Cosmos history: missing hash")?
        .to_ascii_uppercase();
    let code = tx["code"].as_u64().unwrap_or(0);
    let block_height = tx["height"].as_str().and_then(|height| height.parse().ok());
    let timestamp = tx["timestamp"]
        .as_str()
        .and_then(crate::api::time::parse_iso8601_timestamp);
    let ours =
        |value: Option<&String>| value.is_some_and(|value| value.eq_ignore_ascii_case(address));
    let (mut received, mut sent) = (0u128, 0u128);
    let mut counterparty = String::new();
    if code == 0 {
        for transfer in message_transfers(tx) {
            let amount = coins_of(transfer.get("amount").map_or("", String::as_str), denom)?;
            if amount == 0 {
                continue;
            }
            let (to_us, from_us) = (
                ours(transfer.get("recipient")),
                ours(transfer.get("sender")),
            );
            if to_us && !from_us {
                received = received
                    .checked_add(amount)
                    .or_decode("Cosmos history: overflow")?;
                counterparty = transfer.get("sender").cloned().unwrap_or_default();
            } else if from_us && !to_us {
                sent = sent
                    .checked_add(amount)
                    .or_decode("Cosmos history: overflow")?;
                counterparty = transfer.get("recipient").cloned().unwrap_or_default();
            }
        }
    } else {
        for message in tx["tx"]["body"]["messages"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if message["@type"] == "/cosmos.bank.v1beta1.MsgSend"
                && message["from_address"]
                    .as_str()
                    .is_some_and(|from| from.eq_ignore_ascii_case(address))
            {
                for coin in message["amount"].as_array().into_iter().flatten() {
                    if coin["denom"].as_str() == Some(denom) {
                        let amount: u128 = coin["amount"]
                            .as_str()
                            .and_then(|amount| amount.parse().ok())
                            .or_decode("Cosmos history: bad amount")?;
                        sent = sent
                            .checked_add(amount)
                            .or_decode("Cosmos history: overflow")?;
                    }
                }
                counterparty = message["to_address"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
    }
    if received == sent {
        return Ok(None);
    }
    let incoming = received > sent;
    Ok(Some(HistoryEntry {
        txid,
        contract: None,
        amount: HistoryAmount::Units(received.abs_diff(sent)),
        incoming,
        outcome: if code == 0 {
            HistoryOutcome::Confirmed
        } else {
            HistoryOutcome::Failed
        },
        block_height,
        timestamp,
        counterparty,
    }))
}

#[cfg(test)]
#[path = "tests/cosmos_rest.rs"]
mod tests;

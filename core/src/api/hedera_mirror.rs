//! A Hedera mirror node's REST API: the network it serves, an account's key
//! and balance, the accounts that hold a key, an account's HBAR transfers, a
//! transaction's result, the exchange rate fees are charged at, the latest
//! consensus time and the address book of consensus nodes.
//!
//! A mirror node reads the network's record stream; it takes no
//! transactions, which go to consensus nodes (`api::hedera_hapi`).

use std::sync::Arc;

use serde_json::Value;

use crate::api::error::{ApiError, OrDecode};
use crate::api::history_page::{HistoryAmount, HistoryEntry, HistoryOutcome, HistoryPage};
use crate::api::http::{HttpClient, RetryProfile};
use crate::api::transaction_status::TransactionStatus;
use crate::derivation::hedera::{HederaAccountId, HederaKeyScheme, HederaPublicKey};
use crate::registry::Chain;

/// The key an account holds, as far as a wallet can sign for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HederaAccountKey {
    /// One Ed25519 or ECDSA secp256k1 key.
    Single(HederaPublicKey),
    /// A key list, a threshold key, a contract or no key: nothing one key
    /// signs for alone.
    Other,
}

/// An account as a mirror node reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HederaAccount {
    pub id: HederaAccountId,
    pub key: HederaAccountKey,
    pub deleted: bool,
    /// Its HBAR, in tinybars; `None` where the answer left it out.
    pub balance: Option<u128>,
    /// Whether a transfer into it needs its own signature too.
    pub receiver_sig_required: bool,
}

/// The rate fees are charged at: `cent_equivalent` US cents buy
/// `hbar_equivalent` HBAR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HederaExchangeRate {
    pub cent_equivalent: u64,
    pub hbar_equivalent: u64,
}

impl HederaExchangeRate {
    /// The tinybars `mills` thousandths of a dollar buy at this rate,
    /// rounded up.
    pub(crate) fn tinybars_for_mills(self, mills: u128) -> Option<u128> {
        // mills / 10 cents; cents * hbar / cent_equivalent HBAR; 1e8 tinybars.
        let numerator = mills
            .checked_mul(u128::from(self.hbar_equivalent))?
            .checked_mul(100_000_000)?;
        let denominator = u128::from(self.cent_equivalent).checked_mul(10)?;
        (denominator > 0).then(|| numerator.div_ceil(denominator))
    }
}

/// A moment on the network's clock: seconds and nanoseconds since the Unix
/// epoch, as consensus timestamps are written (`"1795522576.874181104"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HederaTimestamp {
    pub seconds: i64,
    pub nanos: i32,
}

impl HederaTimestamp {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (seconds, nanos) = text.split_once('.').unwrap_or((text, "0"));
        if nanos.is_empty() || nanos.len() > 9 || !nanos.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let nanos: i32 = format!("{nanos:0<9}").parse().ok()?;
        Some(Self {
            seconds: seconds.parse().ok().filter(|s: &i64| *s >= 0)?,
            nanos,
        })
    }
}

/// A consensus node a transaction can go to: its account, where it takes
/// gRPC over TLS, and the SHA-384 of its TLS certificate in PEM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HederaNode {
    pub account: HederaAccountId,
    /// `host:port` of each TLS service endpoint.
    pub tls_endpoints: Vec<(String, u16)>,
    pub cert_hash: [u8; 48],
}

/// The port a consensus node takes plaintext gRPC on; its other service
/// endpoints (50212) take TLS.
pub(crate) const NODE_PLAINTEXT_PORT: u16 = 50211;

/// How many transfers one history request asks for.
const HISTORY_PAGE_SIZE: u32 = 25;

/// How many requests one history page may take: the mirror node scans a
/// bounded stretch of time per request and answers an empty one with a
/// link to the stretch before it.
const HISTORY_REQUESTS_PER_PAGE: usize = 8;

/// How many pages of accounts a key lookup reads.
const KEY_LOOKUP_PAGES: usize = 4;

pub struct HederaMirrorClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl HederaMirrorClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    /// Refuse a mirror node that does not serve `chain`: its first record
    /// file must be the network's.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let expected = chain.hedera_network()?.genesis_block_hash;
        let block = self.get("/api/v1/blocks/0").await?;
        let hash = block["hash"]
            .as_str()
            .or_decode("Hedera mirror block 0: missing hash")?;
        if !hash.eq_ignore_ascii_case(expected) {
            return Err(ApiError::invalid(format!(
                "Mirror node serves another Hedera network (block 0 is {hash})"
            )));
        }
        Ok(())
    }

    /// The account `id` names, or `None` while the network has none.
    pub(crate) async fn fetch_account(
        &self,
        id: HederaAccountId,
    ) -> Result<Option<HederaAccount>, ApiError> {
        let response = match self
            .get(&format!("/api/v1/accounts/{id}?transactions=false"))
            .await
        {
            Ok(response) => response,
            Err(ApiError::Status { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let account = account_from(&response)?;
        if account.id != id {
            return Err(ApiError::decode("Hedera account: another account"));
        }
        Ok(Some(account))
    }

    /// The accounts whose key is `key` alone, deleted ones left out: the
    /// mirror node's `account.publickey` filter, each answer checked.
    pub(crate) async fn accounts_holding_key(
        &self,
        key: &HederaPublicKey,
    ) -> Result<Vec<HederaAccountId>, ApiError> {
        let mut path = format!(
            "/api/v1/accounts?account.publickey={}&balance=false&limit=100&order=asc",
            hex::encode(&key.bytes)
        );
        let mut found = Vec::new();
        for _ in 0..KEY_LOOKUP_PAGES {
            let response = self.get(&path).await?;
            for account in response["accounts"]
                .as_array()
                .or_decode("Hedera accounts: missing list")?
            {
                let account = account_from(account)?;
                if !account.deleted && account.key == HederaAccountKey::Single(key.clone()) {
                    found.push(account.id);
                }
            }
            match next_link(&response)? {
                Some(next) => path = next,
                None => return Ok(found),
            }
        }
        Err(ApiError::Rejected(
            "More accounts hold this key than one lookup reads; name the account".into(),
        ))
    }

    /// The account's HBAR balance in tinybars; an account the network does
    /// not have holds none.
    pub(crate) async fn fetch_balance(&self, id: HederaAccountId) -> Result<u128, ApiError> {
        Ok(self
            .fetch_account(id)
            .await?
            .and_then(|account| account.balance)
            .unwrap_or(0))
    }

    /// The exchange rate fees are charged at now, and the one that follows
    /// it: a fee bound is priced at whichever buys fewer HBAR per cent.
    pub(crate) async fn fetch_exchange_rates(&self) -> Result<[HederaExchangeRate; 2], ApiError> {
        let response = self.get("/api/v1/network/exchangerate").await?;
        let rate = |field: &str| -> Result<HederaExchangeRate, ApiError> {
            let rate = &response[field];
            let number = |name: &str| {
                rate[name]
                    .as_u64()
                    .filter(|value| *value > 0)
                    .or_decode("Hedera exchange rate: missing value")
            };
            Ok(HederaExchangeRate {
                cent_equivalent: number("cent_equivalent")?,
                hbar_equivalent: number("hbar_equivalent")?,
            })
        };
        Ok([rate("current_rate")?, rate("next_rate")?])
    }

    /// The consensus time of the newest record file the mirror node has
    /// read: a few seconds behind the network, and never ahead of it.
    pub(crate) async fn fetch_network_time(&self) -> Result<HederaTimestamp, ApiError> {
        let response = self.get("/api/v1/blocks?limit=1&order=desc").await?;
        let block = response["blocks"]
            .as_array()
            .and_then(|blocks| blocks.first())
            .or_decode("Hedera blocks: none")?;
        block["timestamp"]["to"]
            .as_str()
            .and_then(HederaTimestamp::parse)
            .or_decode("Hedera block: missing timestamp")
    }

    /// The transaction `transaction_id` (`0.0.N-seconds-nanos`), as its
    /// result says it ran; one the mirror node has not seen is pending.
    pub(crate) async fn fetch_transaction_status(
        &self,
        transaction_id: &str,
    ) -> Result<TransactionStatus, ApiError> {
        if !is_transaction_id(transaction_id) {
            return Err(ApiError::invalid("Invalid Hedera transaction id"));
        }
        let response = match self
            .get(&format!("/api/v1/transactions/{transaction_id}"))
            .await
        {
            Ok(response) => response,
            Err(ApiError::Status { status: 404, .. }) => return Ok(TransactionStatus::Pending),
            Err(error) => return Err(error),
        };
        // The user transaction itself, not a child it caused or a
        // scheduled one under the same id.
        let transaction = response["transactions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|transaction| {
                transaction["transaction_id"].as_str() == Some(transaction_id)
                    && transaction["scheduled"].as_bool() == Some(false)
                    && transaction["nonce"].as_u64().unwrap_or(0) == 0
            });
        let Some(transaction) = transaction else {
            return Ok(TransactionStatus::Pending);
        };
        let result = transaction["result"]
            .as_str()
            .or_decode("Hedera transaction: missing result")?;
        Ok(TransactionStatus::Confirmed {
            succeeded: result == "SUCCESS",
            block: None,
        })
    }

    /// One page of the account's history: its HBAR transfers, newest first,
    /// each as what it did to the account's balance with the fee and any
    /// staking reward aside. `cursor` is the mirror node's own next link.
    pub(crate) async fn fetch_history_page(
        &self,
        account: HederaAccountId,
        cursor: Option<&str>,
    ) -> Result<HistoryPage<HistoryEntry>, ApiError> {
        let mut path = match cursor {
            Some(cursor) => {
                if !cursor.starts_with("/api/v1/transactions?") {
                    return Err(ApiError::InvalidInput("invalid history cursor".into()));
                }
                cursor.to_string()
            }
            None => format!(
                "/api/v1/transactions?account.id={account}&transactiontype=CRYPTOTRANSFER&limit={HISTORY_PAGE_SIZE}&order=desc"
            ),
        };
        let mut items = Vec::new();
        for _ in 0..HISTORY_REQUESTS_PER_PAGE {
            let response = self.get(&path).await?;
            for transaction in response["transactions"]
                .as_array()
                .or_decode("Hedera transactions: missing list")?
            {
                if let Some(entry) = history_entry(transaction, account)? {
                    items.push(entry);
                }
            }
            match next_link(&response)? {
                Some(next) if items.is_empty() => path = next,
                next => {
                    return Ok(HistoryPage {
                        items,
                        next_cursor: next,
                    });
                }
            }
        }
        Ok(HistoryPage {
            items,
            next_cursor: Some(path),
        })
    }
}

/// The consensus nodes an address book at `url` names (a mirror node's
/// `/api/v1/network/nodes`), each with its TLS endpoints and certificate
/// hash; a node without them is left out.
pub(crate) async fn fetch_address_book(url: &str) -> Result<Vec<HederaNode>, ApiError> {
    let client = HttpClient::shared();
    let origin = mirror_of_address_book(url)?;
    let mut next = Some(url.to_string());
    let mut nodes = Vec::new();
    // Mainnet lists a few dozen nodes, 25 to a page.
    for _ in 0..8 {
        let Some(url) = next.take() else {
            return Ok(nodes);
        };
        let response: Value = client.get_json(&url, RetryProfile::ChainRead).await?;
        for node in response["nodes"]
            .as_array()
            .or_decode("Hedera address book: missing nodes")?
        {
            if let Some(node) = node_from(node)? {
                nodes.push(node);
            }
        }
        next = next_link(&response)?.map(|path| format!("{origin}{path}"));
    }
    if next.is_some() {
        return Err(ApiError::decode("Hedera address book: too many pages"));
    }
    Ok(nodes)
}

/// The mirror node an address book URL belongs to: the URL without its
/// `/api/v1/network/nodes`.
pub(crate) fn mirror_of_address_book(url: &str) -> Result<String, ApiError> {
    url.trim_end_matches('/')
        .strip_suffix("/api/v1/network/nodes")
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError::invalid(format!(
                "A Hedera address book is a mirror node's /api/v1/network/nodes, not {url}"
            ))
        })
}

/// A mirror node's link to the next page, a path on the same node; `None`
/// on the last page.
fn next_link(response: &Value) -> Result<Option<String>, ApiError> {
    match &response["links"]["next"] {
        Value::Null => Ok(None),
        Value::String(next) if next.starts_with("/api/v1/") => Ok(Some(next.clone())),
        _ => Err(ApiError::decode("Hedera mirror node: invalid next link")),
    }
}

/// Whether `text` is a transaction id as a mirror node writes it:
/// `0.0.N-seconds-nanos`, nanos in nine digits.
pub(crate) fn is_transaction_id(text: &str) -> bool {
    let mut parts = text.split('-');
    let (Some(payer), Some(seconds), Some(nanos), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    HederaAccountId::parse(Chain::Hedera, payer).is_ok_and(|id| id.to_string() == payer)
        && !seconds.is_empty()
        && seconds.bytes().all(|b| b.is_ascii_digit())
        && nanos.len() == 9
        && nanos.bytes().all(|b| b.is_ascii_digit())
}

/// The account an answer describes.
fn account_from(account: &Value) -> Result<HederaAccount, ApiError> {
    let id = account["account"]
        .as_str()
        .and_then(|id| HederaAccountId::parse(Chain::Hedera, id).ok())
        .or_decode("Hedera account: missing id")?;
    let key = &account["key"];
    let scheme = match key["_type"].as_str() {
        Some("ED25519") => Some(HederaKeyScheme::Ed25519),
        Some("ECDSA_SECP256K1") => Some(HederaKeyScheme::EcdsaSecp256k1),
        _ => None,
    };
    let key = match scheme {
        Some(scheme) => {
            let bytes = key["key"]
                .as_str()
                .and_then(|key| hex::decode(key).ok())
                .or_decode("Hedera account: missing key")?;
            HederaAccountKey::Single(
                HederaPublicKey::from_raw(scheme, &bytes)
                    .or_decode("Hedera account: invalid key")?,
            )
        }
        None => HederaAccountKey::Other,
    };
    let balance = match &account["balance"]["balance"] {
        Value::Null => None,
        value => Some(u128::from(
            value
                .as_u64()
                .or_decode("Hedera account: invalid balance")?,
        )),
    };
    Ok(HederaAccount {
        id,
        key,
        deleted: account["deleted"].as_bool().unwrap_or(false),
        balance,
        receiver_sig_required: account["receiver_sig_required"].as_bool().unwrap_or(false),
    })
}

/// A node of the address book, with the TLS endpoints and certificate hash
/// a submission needs; `None` for a node without them.
fn node_from(node: &Value) -> Result<Option<HederaNode>, ApiError> {
    let account = node["node_account_id"]
        .as_str()
        .and_then(|id| HederaAccountId::parse(Chain::Hedera, id).ok())
        .or_decode("Hedera node: missing account")?;
    let Some(cert_hash) = node["node_cert_hash"]
        .as_str()
        .and_then(|hash| hash.strip_prefix("0x"))
        .and_then(|hash| hex::decode(hash).ok())
        .and_then(|hash| <[u8; 48]>::try_from(hash).ok())
    else {
        return Ok(None);
    };
    let tls_endpoints: Vec<(String, u16)> = node["service_endpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|endpoint| {
            let port = endpoint["port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port != NODE_PLAINTEXT_PORT && *port != 0)?;
            let host = [&endpoint["domain_name"], &endpoint["ip_address_v4"]]
                .into_iter()
                .filter_map(Value::as_str)
                .find(|host| !host.is_empty())?;
            Some((host.to_string(), port))
        })
        .collect();
    Ok((!tls_endpoints.is_empty()).then_some(HederaNode {
        account,
        tls_endpoints,
        cert_hash,
    }))
}

/// One transfer list entry: the account and its signed tinybar change.
fn transfers(list: &Value) -> Result<Vec<(HederaAccountId, i128)>, ApiError> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|transfer| {
            let account = transfer["account"]
                .as_str()
                .and_then(|id| HederaAccountId::parse(Chain::Hedera, id).ok())
                .or_decode("Hedera transfer: missing account")?;
            let amount = transfer["amount"]
                .as_i64()
                .or_decode("Hedera transfer: missing amount")?;
            Ok((account, i128::from(amount)))
        })
        .collect()
}

/// What one transfer did to `account`'s HBAR: its change in the transfer
/// list, less the staking reward the transaction paid it and, where it paid
/// the fee, plus that fee. A transfer that moved nothing else is no entry,
/// a failed one included: it only paid its fee.
fn history_entry(
    transaction: &Value,
    account: HederaAccountId,
) -> Result<Option<HistoryEntry>, ApiError> {
    let txid = transaction["transaction_id"]
        .as_str()
        .filter(|id| is_transaction_id(id))
        .or_decode("Hedera history: missing transaction id")?
        .to_string();
    let payer = txid
        .split('-')
        .next()
        .and_then(|payer| HederaAccountId::parse(Chain::Hedera, payer).ok())
        .or_decode("Hedera history: missing payer")?;
    let moved = transfers(&transaction["transfers"])?;
    let ours = |list: &[(HederaAccountId, i128)]| -> i128 {
        list.iter()
            .filter(|(id, _)| *id == account)
            .map(|(_, amount)| amount)
            .sum()
    };
    let fee = if payer == account {
        i128::from(
            transaction["charged_tx_fee"]
                .as_u64()
                .or_decode("Hedera history: missing fee")?,
        )
    } else {
        0
    };
    let net = ours(&moved) - ours(&transfers(&transaction["staking_reward_transfers"])?) + fee;
    if net == 0 {
        return Ok(None);
    }
    let incoming = net > 0;
    // The other party: the person's account that moved the most the other
    // way, the network's own fee and reward accounts aside.
    let counterparty = moved
        .iter()
        .filter(|(id, amount)| {
            *id != account && !id.is_system() && (*amount < 0) == incoming && *amount != 0
        })
        .max_by_key(|(_, amount)| amount.unsigned_abs())
        .map(|(id, _)| id.to_string())
        .unwrap_or_default();
    let succeeded = transaction["result"].as_str() == Some("SUCCESS");
    Ok(Some(HistoryEntry {
        txid,
        contract: None,
        amount: HistoryAmount::Units(net.unsigned_abs()),
        incoming,
        outcome: if succeeded {
            HistoryOutcome::Confirmed
        } else {
            HistoryOutcome::Failed
        },
        block_height: None,
        timestamp: transaction["consensus_timestamp"]
            .as_str()
            .and_then(|text| text.parse::<f64>().ok()),
        counterparty,
    }))
}

/// A mirror node for the tests that import on every network: it serves
/// mainnet's block 0 and holds one account for any key it is asked about,
/// numbered from the key, so a phrase or key imports without the network.
#[cfg(test)]
pub(crate) mod test_mirror {
    use serde_json::json;

    /// The account the test mirror holds for the raw key `key`.
    pub(crate) fn account_of(key: &[u8]) -> String {
        let prefix = u32::from_be_bytes(key[1..5].try_into().expect("a key of 32 or 33 bytes"));
        format!("0.0.{}", 1001 + u64::from(prefix % 1_000_000))
    }

    /// The test mirror's URL, started once for the whole run.
    pub(crate) async fn url() -> String {
        static SERVER: tokio::sync::OnceCell<wiremock::MockServer> =
            tokio::sync::OnceCell::const_new();
        SERVER
            .get_or_init(|| async {
                let server = wiremock::MockServer::start().await;
                wiremock::Mock::given(wiremock::matchers::any())
                    .respond_with(|request: &wiremock::Request| {
                        let query: std::collections::HashMap<_, _> =
                            request.url.query_pairs().into_owned().collect();
                        let body = match request.url.path() {
                            "/api/v1/blocks/0" => json!({
                                "hash": crate::registry::Chain::Hedera
                                    .hedera_network()
                                    .unwrap()
                                    .genesis_block_hash,
                            }),
                            "/api/v1/accounts" => {
                                let key = query.get("account.publickey").cloned().unwrap_or_default();
                                let bytes = hex::decode(&key).unwrap_or_default();
                                let accounts = if bytes.len() == 32 || bytes.len() == 33 {
                                    vec![json!({
                                        "account": account_of(&bytes),
                                        "deleted": false,
                                        "key": {
                                            "_type": if bytes.len() == 32 { "ED25519" } else { "ECDSA_SECP256K1" },
                                            "key": key,
                                        },
                                    })]
                                } else {
                                    Vec::new()
                                };
                                json!({"accounts": accounts, "links": {"next": null}})
                            }
                            _ => return wiremock::ResponseTemplate::new(404),
                        };
                        wiremock::ResponseTemplate::new(200).set_body_json(body)
                    })
                    .mount(&server)
                    .await;
                server
            })
            .await
            .uri()
    }
}

#[cfg(test)]
#[path = "tests/hedera_mirror.rs"]
mod tests;

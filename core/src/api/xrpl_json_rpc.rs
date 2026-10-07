//! The XRP Ledger JSON-RPC adapter (rippled / Clio): account info and
//! sequence, fees, transaction history and blob submission.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpBalance {
    /// XRP drops (1 XRP = 1_000_000 drops).
    pub drops: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpHistoryEntry {
    pub txid: String,
    pub ledger_index: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_drops: u64,
    pub fee_drops: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpSendResult {
    pub txid: String,
    /// Signed tx blob hex — stored for rebroadcast.
    pub tx_blob_hex: String,
}

// ── Client

pub struct XrplClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

/// An XRP account's reserve, in drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpReserveState {
    pub exists: bool,
    pub balance_drops: u64,
    pub owner_count: u64,
    pub reserve_base: u64,
    pub reserve_increment: u64,
}

/// What deleting an account depends on, from one node's validated ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpDeletionState {
    /// `None` when the account does not exist.
    pub source: Option<XrpDeletableAccount>,
    /// The destination's flags; `None` when it does not exist.
    pub destination_flags: Option<u32>,
    pub ledger_index: u32,
    pub reserve_base: u64,
    pub reserve_increment: u64,
}

/// The account being deleted, as the validated ledger holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpDeletableAccount {
    pub balance_drops: u64,
    pub sequence: u32,
    pub owner_count: u64,
    /// `FirstNFTokenSequence + MintedNFTokens`, where the account has minted.
    pub minted_nft_sequence: Option<u64>,
    /// Objects in its owner directory the network will not delete with it:
    /// trust lines, escrows, payment channels, checks and the like.
    pub blockers: u64,
}

impl XrplClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            crate::EndpointApi::XrplJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }
}

// XRP fetch paths: balance, sequence, fee, history.

impl XrplClient {
    pub(crate) async fn verify_network(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let expected = chain
            .xrp_network_id()
            .or_decode("Missing XRP network identity")?;
        let result = self.call("server_info", json!({})).await?;
        if result.pointer("/info/network_id").and_then(Value::as_u64) != Some(expected) {
            return Err(ApiError::invalid("XRP endpoint is on the wrong network"));
        }
        Ok(())
    }

    /// A provisional result is not final. Validated `tec` results are committed failures.
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        use crate::api::http::{RetryProfile, race};
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::invalid("Invalid XRP transaction hash"));
        }
        // Inspect the machine-readable error before generic RPC decoding loses it.
        let body = json!({"method":"tx","params":[{"transaction":hash,"binary":false}]});
        race(&self.endpoints, |endpoint| {
            let body = &body;
            async move {
                let response: Value = self
                    .client
                    .post_json(&endpoint, body, RetryProfile::ChainRead)
                    .await?;
                xrp_transaction_status(response, hash)
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<XrpBalance, ApiError> {
        let result = self
            .call(
                "account_info",
                json!({"account": address, "ledger_index": "validated"}),
            )
            .await?;
        let drops: u64 = result
            .pointer("/account_data/Balance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("account_info: missing Balance")?;
        Ok(XrpBalance { drops })
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u32, ApiError> {
        let result = self
            .call(
                "account_info",
                json!({"account": address, "ledger_index": "current"}),
            )
            .await?;
        result
            .pointer("/account_data/Sequence")
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .or_decode("account_info: missing or invalid Sequence")
    }

    /// The validated ledger's base reserve, in drops: what an account must
    /// hold for the ledger to create it.
    pub(crate) async fn fetch_base_reserve(&self) -> Result<u64, ApiError> {
        let result = self.call("server_state", json!({})).await?;
        result
            .pointer("/state/validated_ledger/reserve_base")
            .and_then(Value::as_u64)
            .or_decode("server_state: missing reserve_base")
    }

    /// An account's reserve as the validated ledger holds it, from one
    /// verified node: whether the account exists, its balance, how many
    /// objects it owns, and the base and per-object reserves, in drops.
    pub(crate) async fn fetch_reserve_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<XrpReserveState, ApiError> {
        use crate::api::http::race;
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint.clone()]));
            node.verify_network(chain).await?;
            let state = node.call("server_state", json!({})).await?;
            let reserve = |field: &str| {
                state
                    .pointer(&format!("/state/validated_ledger/{field}"))
                    .and_then(Value::as_u64)
                    .or_decode("server_state: missing reserve")
            };
            let (exists, balance_drops, owner_count) =
                match node.account_root(&endpoint, address).await? {
                    None => (false, 0, 0),
                    Some(root) => (true, balance_of(&root)?, owner_count_of(&root)?),
                };
            Ok(XrpReserveState {
                exists,
                balance_drops,
                owner_count,
                reserve_base: reserve("reserve_base")?,
                reserve_increment: reserve("reserve_inc")?,
            })
        })
        .await
    }

    /// What deleting `address` into `destination` depends on, from one
    /// verified node's validated ledger: both accounts, the objects that
    /// would block the deletion, the ledger index and the reserves.
    pub(crate) async fn fetch_deletion_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
        destination: &str,
    ) -> Result<XrpDeletionState, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint.clone()]));
            node.verify_network(chain).await?;
            let state = node.call("server_state", json!({})).await?;
            let validated = |field: &str| {
                state
                    .pointer(&format!("/state/validated_ledger/{field}"))
                    .and_then(Value::as_u64)
                    .or_decode("server_state: missing validated ledger")
            };
            let source = match node.account_root(&endpoint, address).await? {
                None => None,
                Some(root) => {
                    let field = |name: &str| root.get(name).and_then(Value::as_u64);
                    let blockers = node
                        .call(
                            "account_objects",
                            json!({
                                "account": address,
                                "ledger_index": "validated",
                                "deletion_blockers_only": true,
                                "limit": 10,
                            }),
                        )
                        .await?
                        .get("account_objects")
                        .and_then(Value::as_array)
                        .or_decode("account_objects: missing list")?
                        .len();
                    Some(XrpDeletableAccount {
                        balance_drops: balance_of(&root)?,
                        sequence: field("Sequence")
                            .and_then(|sequence| u32::try_from(sequence).ok())
                            .or_decode("account_info: missing Sequence")?,
                        owner_count: owner_count_of(&root)?,
                        minted_nft_sequence: field("MintedNFTokens")
                            .map(|minted| minted + field("FirstNFTokenSequence").unwrap_or(0)),
                        blockers: blockers as u64,
                    })
                }
            };
            let destination_flags = match node.account_root(&endpoint, destination).await? {
                None => None,
                Some(root) => Some(
                    root.get("Flags")
                        .and_then(Value::as_u64)
                        .and_then(|flags| u32::try_from(flags).ok())
                        .or_decode("account_info: missing Flags")?,
                ),
            };
            Ok(XrpDeletionState {
                source,
                destination_flags,
                ledger_index: u32::try_from(validated("seq")?)
                    .map_err(|_| ApiError::decode("server_state: ledger index out of range"))?,
                reserve_base: validated("reserve_base")?,
                reserve_increment: validated("reserve_inc")?,
            })
        })
        .await
    }

    /// An account's root as the validated ledger holds it on `endpoint`, or
    /// `None` when the ledger has no such account. The unfunded case is an
    /// `actNotFound` error, read here before generic decoding turns it into
    /// a refusal.
    async fn account_root(&self, endpoint: &str, address: &str) -> Result<Option<Value>, ApiError> {
        let body = json!({"method": "account_info", "params": [{"account": address, "ledger_index": "validated"}]});
        let response: Value = self
            .client
            .post_json(endpoint, &body, crate::api::http::RetryProfile::ChainRead)
            .await?;
        let result = response
            .get("result")
            .or_decode("account_info: missing result")?;
        match result.get("error").and_then(Value::as_str) {
            Some("actNotFound") => Ok(None),
            Some(error) => Err(ApiError::rejected(format!("account_info: {error}"))),
            None => Ok(Some(
                result
                    .get("account_data")
                    .cloned()
                    .or_decode("account_info: missing account_data")?,
            )),
        }
    }

    pub async fn fetch_fee(&self) -> Result<u64, ApiError> {
        let result = self.call("fee", json!({})).await?;
        result
            .pointer("/drops/open_ledger_fee")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("fee: missing open_ledger_fee")
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<XrpHistoryEntry>, ApiError> {
        Ok(self.fetch_history_page(address, None).await?.items)
    }

    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<XrpHistoryEntry>, ApiError> {
        let mut params = json!({"account": address, "limit": 50, "ledger_index_min": -1, "ledger_index_max": -1, "forward": false});
        if let Some(cursor) = cursor {
            params["marker"] = serde_json::from_str(cursor)?;
        }
        let result = self.call("account_tx", params).await?;
        let txs = result
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let next_cursor = result
            .get("marker")
            .filter(|value| !value.is_null())
            .map(Value::to_string);
        Ok(crate::api::HistoryPage {
            items: xrp_history_from_transactions(&txs, address)?,
            next_cursor,
        })
    }
}

fn xrp_transaction_status(
    response: Value,
    hash: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(ApiError::rejected(format!("XRP transaction: {error}")));
    }
    let result = response
        .get("result")
        .or_decode("XRP transaction: missing result")?;
    if result.get("status").and_then(Value::as_str) == Some("error") {
        return match result.get("error").and_then(Value::as_str) {
            Some("txnNotFound") => Ok(TransactionStatus::Pending),
            _ => Err(ApiError::rejected(format!("XRP transaction: {result}"))),
        };
    }
    if result.get("validated").and_then(Value::as_bool) != Some(true) {
        return Ok(TransactionStatus::Pending);
    }
    let actual = result
        .get("hash")
        .or_else(|| result.pointer("/tx_json/hash"))
        .and_then(Value::as_str)
        .or_decode("XRP transaction: missing hash")?;
    if !actual.eq_ignore_ascii_case(hash) {
        return Err(ApiError::decode("XRP returned a different transaction"));
    }
    let ledger = result
        .get("ledger_index")
        .or_else(|| result.pointer("/tx_json/ledger_index"))
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .or_decode("XRP transaction: missing validated ledger")?;
    let outcome = result
        .pointer("/meta/TransactionResult")
        .and_then(Value::as_str)
        .or_decode("XRP transaction: missing execution result")?;
    let succeeded = if outcome == "tesSUCCESS" {
        true
    } else if outcome.starts_with("tec") {
        false
    } else {
        return Err(ApiError::decode(
            "XRP transaction: invalid validated result code",
        ));
    };
    Ok(TransactionStatus::Confirmed {
        succeeded,
        block: Some(ledger),
    })
}

/// The XRP each successful payment moved in or out of `address`, fee excluded.
///
/// The amount is the account's own balance change from the transaction's
/// metadata, not the payment's `Amount` field: `Amount` is an object for an
/// issued currency, which read as 0 XRP, and for a partial payment it is an
/// upper bound larger than what was delivered. A payment that moved no XRP
/// for this account — an issued currency passing through — yields no entry,
/// and neither does a failed one, which only burned its fee.
fn xrp_history_from_transactions(
    txs: &[Value],
    address: &str,
) -> Result<Vec<XrpHistoryEntry>, ApiError> {
    let drops = |value: Option<&Value>| -> Option<i128> {
        value.and_then(Value::as_str).and_then(|s| s.parse().ok())
    };
    let mut entries = Vec::new();
    for item in txs {
        let tx = item.get("tx").unwrap_or(&Value::Null);
        let meta = item.get("meta").unwrap_or(&Value::Null);
        if tx.get("TransactionType").and_then(Value::as_str) != Some("Payment")
            || meta.get("TransactionResult").and_then(Value::as_str) != Some("tesSUCCESS")
        {
            continue;
        }
        let from = tx
            .get("Account")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let to = tx
            .get("Destination")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let fee_drops = drops(tx.get("Fee")).unwrap_or(0);
        let balance_change: i128 = meta
            .get("AffectedNodes")
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .filter_map(|node| {
                        let (kind, node) = node.as_object()?.iter().next()?;
                        if node.get("LedgerEntryType").and_then(Value::as_str)
                            != Some("AccountRoot")
                        {
                            return None;
                        }
                        let fields = if kind == "CreatedNode" {
                            node.get("NewFields")?
                        } else {
                            node.get("FinalFields")?
                        };
                        if fields.get("Account").and_then(Value::as_str) != Some(address) {
                            return None;
                        }
                        let after = drops(fields.get("Balance"))?;
                        let before = match kind.as_str() {
                            "CreatedNode" => 0,
                            _ => drops(node.pointer("/PreviousFields/Balance")).unwrap_or(after),
                        };
                        Some(after - before)
                    })
                    .sum()
            })
            .unwrap_or(0);
        let transfer = if from == address {
            balance_change + fee_drops
        } else {
            balance_change
        };
        if transfer == 0 {
            continue;
        }
        let Ok(amount_drops) = u64::try_from(transfer.unsigned_abs()) else {
            continue;
        };
        let txid = tx
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // `account_tx` up to the validated ledger lists only validated
        // transactions. XRP epoch: 2000-01-01, Unix epoch difference = 946684800.
        let timestamp = crate::api::time::confirmed_history_time(
            tx.get("date")
                .and_then(Value::as_u64)
                .map(|d| d + 946_684_800),
            &txid,
        )?;
        entries.push(XrpHistoryEntry {
            txid,
            ledger_index: tx.get("ledger_index").and_then(Value::as_u64).unwrap_or(0),
            timestamp,
            from,
            to,
            amount_drops,
            fee_drops: u64::try_from(fee_drops).unwrap_or(0),
            is_incoming: transfer > 0,
        });
    }
    Ok(entries)
}

impl XrplClient {
    /// Submit a pre-signed transaction blob (for rebroadcast).
    pub async fn submit_signed_blob(&self, tx_blob_hex: &str) -> Result<XrpSendResult, ApiError> {
        let result = self.call("submit", json!({"tx_blob": tx_blob_hex})).await?;
        let engine_result = result
            .get("engine_result")
            .and_then(Value::as_str)
            .or_decode("submit: missing engine_result")?;
        let accepted = result
            .get("accepted")
            .and_then(Value::as_bool)
            .or_decode("submit: missing accepted")?;
        if !accepted || !matches!(engine_result, "tesSUCCESS" | "terQUEUED") {
            let message = result
                .get("engine_result_message")
                .and_then(Value::as_str)
                .unwrap_or("transaction was not accepted");
            return Err(ApiError::rejected(format!(
                "submit: {engine_result}: {message}"
            )));
        }
        let txid = result
            .get("tx_json")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .filter(|hash| !hash.is_empty())
            .or_decode("submit: missing transaction hash")?
            .to_string();
        Ok(XrpSendResult {
            txid,
            tx_blob_hex: tx_blob_hex.to_string(),
        })
    }
}

fn balance_of(root: &Value) -> Result<u64, ApiError> {
    root.get("Balance")
        .and_then(Value::as_str)
        .and_then(|drops| drops.parse().ok())
        .or_decode("account_info: missing Balance")
}

fn owner_count_of(root: &Value) -> Result<u64, ApiError> {
    root.get("OwnerCount")
        .and_then(Value::as_u64)
        .or_decode("account_info: missing OwnerCount")
}

#[cfg(test)]
#[path = "tests/xrpl_json_rpc.rs"]
mod tests;

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method},
    };

    #[tokio::test]
    async fn hash_lookup_requires_validation_and_retains_committed_failure() {
        let hash = "AB".repeat(32);
        for (result, expected) in [
            (
                json!({"status":"error","error":"txnNotFound"}),
                TransactionStatus::Pending,
            ),
            (
                json!({"validated":false,"hash":hash,"meta":{"TransactionResult":"tesSUCCESS"}}),
                TransactionStatus::Pending,
            ),
            (
                json!({"validated":true,"hash":hash,"ledger_index":120,"meta":{"TransactionResult":"tesSUCCESS"}}),
                TransactionStatus::Confirmed {
                    succeeded: true,
                    block: Some(120),
                },
            ),
            (
                json!({"validated":true,"hash":hash.to_lowercase(),"ledger_index":121,"meta":{"TransactionResult":"tecUNFUNDED_PAYMENT"}}),
                TransactionStatus::Confirmed {
                    succeeded: false,
                    block: Some(121),
                },
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_json(
                    json!({"method":"tx","params":[{"transaction":hash,"binary":false}]}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":result})))
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                XrplClient::new(Arc::new(vec![server.uri()]))
                    .fetch_transaction_status(&hash)
                    .await
                    .unwrap(),
                expected
            );
        }
        for result in [
            json!({"validated":true,"hash":"CD".repeat(32),"ledger_index":120,"meta":{"TransactionResult":"tesSUCCESS"}}),
            json!({"validated":true,"hash":hash,"ledger_index":120}),
            json!({"validated":true,"hash":hash,"meta":{"TransactionResult":"tesSUCCESS"}}),
            json!({"validated":true,"hash":hash,"ledger_index":120,"meta":{"TransactionResult":"tefPAST_SEQ"}}),
            json!({"status":"error","error":"invalidParams"}),
        ] {
            assert!(xrp_transaction_status(json!({"result":result}), &hash).is_err());
        }
    }

    #[tokio::test]
    async fn server_network_identity_refuses_wrong_and_unknown_networks() {
        use crate::registry::Chain;
        for (actual, selected, expected_ok) in [
            (json!(0), Chain::Xrp, true),
            (json!(1), Chain::XrpTestnet, true),
            (json!(1), Chain::Xrp, false),
            (Value::Null, Chain::Xrp, false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_json(json!({"method":"server_info","params":[{}]})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"result":{"info":{"network_id":actual}}})),
                )
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                XrplClient::new(Arc::new(vec![server.uri()]))
                    .verify_network(selected)
                    .await
                    .is_ok(),
                expected_ok
            );
        }
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "rMeMeMeMeMeMeMeMeMeMeMeMeMeMeMeMe1";
    const THEM: &str = "rThemThemThemThemThemThemThemThem2";

    fn account_root(account: &str, before: &str, after: &str) -> Value {
        json!({"ModifiedNode": {
            "LedgerEntryType": "AccountRoot",
            "FinalFields": {"Account": account, "Balance": after},
            "PreviousFields": {"Balance": before}
        }})
    }

    fn payment(
        hash: &str,
        from: &str,
        to: &str,
        amount: Value,
        result: &str,
        nodes: Value,
    ) -> Value {
        json!({
            "tx": {
                "TransactionType": "Payment", "hash": hash, "Account": from,
                "Destination": to, "Amount": amount, "Fee": "12", "date": 800_000_000u64
            },
            "meta": {"TransactionResult": result, "AffectedNodes": nodes}
        })
    }

    #[test]
    fn amounts_are_the_accounts_own_balance_change() {
        let txs = [
            // An outgoing payment: the fee is not part of the transfer.
            payment(
                "sent",
                ME,
                THEM,
                json!("5000000"),
                "tesSUCCESS",
                json!([
                    account_root(ME, "20000000", "14999988"),
                    account_root(THEM, "0", "5000000")
                ]),
            ),
            // A partial payment claims 1,000 XRP and delivers 1.
            payment(
                "partial",
                THEM,
                ME,
                json!("1000000000"),
                "tesSUCCESS",
                json!([account_root(ME, "14999988", "15999988")]),
            ),
            // An issued currency moves no XRP for this account.
            payment(
                "iou",
                THEM,
                ME,
                json!({"currency": "USD", "issuer": THEM, "value": "5"}),
                "tesSUCCESS",
                json!([account_root(THEM, "100", "88")]),
            ),
            // A failed payment only burned its fee.
            payment(
                "failed",
                ME,
                THEM,
                json!("5000000"),
                "tecUNFUNDED_PAYMENT",
                json!([account_root(ME, "15999988", "15999976")]),
            ),
        ];
        let entries = xrp_history_from_transactions(&txs, ME).unwrap();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].txid, "sent");
        assert!(!entries[0].is_incoming);
        assert_eq!(entries[0].amount_drops, 5_000_000);
        assert_eq!(entries[1].txid, "partial");
        assert!(entries[1].is_incoming);
        assert_eq!(entries[1].amount_drops, 1_000_000);
    }
}

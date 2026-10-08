//! The Nearblocks adapter: a NEAR account's transfers, which a NEAR node
//! does not index.

use crate::api::error::{ApiError, OrDecode};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::HttpClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearHistoryEntry {
    pub txid: String,
    pub timestamp_ns: u64,
    /// The receipt's predecessor and receiver.
    pub from: String,
    pub to: String,
    pub amount_yocto: String,
    pub is_incoming: bool,
}

pub struct NearblocksClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl NearblocksClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// The tokens the account holds, from its complete FT inventory.
    pub async fn fetch_ft_holdings(
        &self,
        account: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        Ok(self
            .fetch_ft_inventory(account)
            .await?
            .into_iter()
            .filter(|token| token.balance_raw > 0)
            .collect())
    }

    /// Every token contract the account's inventory names, held or emptied.
    pub(crate) async fn fetch_ft_contracts(&self, account: &str) -> Result<Vec<String>, ApiError> {
        Ok(self
            .fetch_ft_inventory(account)
            .await?
            .into_iter()
            .map(|token| token.contract)
            .collect())
    }

    /// Read the complete FT inventory through v3's opaque cursor, a zero
    /// balance included. A partial inventory is refused because absence is
    /// interpreted as a zero holding.
    async fn fetch_ft_inventory(
        &self,
        account: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        let mut held = Vec::new();
        for _ in 0..40 {
            let suffix = cursor
                .as_deref()
                .map(|value| format!("&next={}", crate::api::history_page::query_value(value)))
                .unwrap_or_default();
            let response: Value = self
                .client
                .get_path(
                    &self.endpoints,
                    &format!("/accounts/{account}/assets/fts?limit=250{suffix}"),
                )
                .await?;
            let rows = response["data"]
                .as_array()
                .or_decode("NEAR inventory: missing data")?;
            if rows.len() > 250 {
                return Err(ApiError::decode(
                    "NEAR inventory exceeds requested page size",
                ));
            }
            for row in rows {
                let contract = row["contract"]
                    .as_str()
                    .or_decode("NEAR inventory: missing contract")?;
                if !seen.insert(contract.to_string()) {
                    return Err(ApiError::decode("NEAR inventory repeats a contract"));
                }
                let balance_raw = row["amount"]
                    .as_str()
                    .and_then(|value| value.parse::<u128>().ok())
                    .or_decode("NEAR inventory: invalid amount")?;
                let decimals = near_ft_decimals(&row["meta"])?;
                held.push(crate::api::HeldToken {
                    contract: contract.into(),
                    balance_raw,
                    decimals: Some(decimals),
                });
            }
            let next = next_page(&response)?;
            if next.is_none() {
                return Ok(held);
            }
            if next == cursor {
                return Err(ApiError::decode("NEAR inventory repeats its cursor"));
            }
            cursor = next;
        }
        Err(ApiError::Rejected(
            "NEAR inventory exceeds 10000 contracts; cannot read the complete list".into(),
        ))
    }

    pub async fn fetch_ft_history_page(
        &self,
        account: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<Value>, ApiError> {
        let suffix = cursor
            .map(|value| format!("&next={}", crate::api::history_page::query_value(value)))
            .unwrap_or_default();
        let response: Value = self
            .client
            .get_path(
                &self.endpoints,
                &format!("/accounts/{account}/ft-txns?limit=50{suffix}"),
            )
            .await?;
        let rows = response["data"]
            .as_array()
            .or_decode("NEAR FT history: missing data")?;
        if rows.len() > 50 {
            return Err(ApiError::decode(
                "NEAR FT history exceeds requested page size",
            ));
        }
        // Account activities locate origins; complete transaction activities
        // determine amounts even when an origin is split across account pages.
        let mut unique = std::collections::BTreeSet::new();
        for row in rows {
            unique.insert(
                row["transaction_hash"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .or_decode("NEAR FT history: missing transaction hash")?
                    .to_string(),
            );
        }
        let account = account.to_string();
        let batches: Vec<Result<Vec<Value>, ApiError>> = stream::iter(unique).map(|hash| {
            let client = self.client.clone(); let endpoints = self.endpoints.clone(); let account = account.clone();
            async move {
                let full: Value = client.get_path(&endpoints, &format!("/txns/{hash}/fts")).await?;
                let activities = full["data"].as_array().or_decode("NEAR FT transaction: missing activities")?;
                if activities.len() > 10000 || full.pointer("/meta/next_page").is_some_and(|next| !next.is_null()) { return Err(ApiError::Rejected("NEAR FT transaction activity list is incomplete or exceeds 10000 events".into())); }
                near_ft_history(activities, &account, &hash)
            }
        }).buffer_unordered(4).collect().await;
        let mut items = Vec::new();
        for batch in batches {
            items.extend(batch?);
        }
        Ok(crate::api::HistoryPage {
            items,
            next_cursor: next_page(&response)?,
        })
    }

    /// The account's NEAR transfers, newest first, from Nearblocks' receipt
    /// list.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<NearHistoryEntry>, ApiError> {
        Ok(self.fetch_history_page(address, None).await?.items)
    }

    pub async fn fetch_history_page(
        &self,
        account_id: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<NearHistoryEntry>, ApiError> {
        let suffix = cursor
            .map(|value| format!("&next={}", crate::api::history_page::query_value(value)))
            .unwrap_or_default();
        let page: Value = self
            .client
            .get_path(
                &self.endpoints,
                &format!("/accounts/{account_id}/receipts?limit=50{suffix}"),
            )
            .await?;
        let receipts = page["data"]
            .as_array()
            .or_decode("NEAR history: response has no data")?;
        if receipts.len() > 50 {
            return Err(ApiError::Decode(
                "NEAR history exceeds requested page size".into(),
            ));
        }
        let next_cursor = next_page(&page)?;
        let hashes: std::collections::BTreeSet<String> = receipts
            .iter()
            .map(|receipt| {
                receipt["transaction_hash"]
                    .as_str()
                    .filter(|hash| !hash.is_empty())
                    .map(str::to_string)
                    .or_decode("NEAR history: missing origin transaction")
            })
            .collect::<Result<_, _>>()?;
        let fetched: Vec<_> = stream::iter(hashes)
            .map(|hash| {
                let client = self.client.clone();
                let endpoints = self.endpoints.clone();
                let account = account_id.to_string();
                async move {
                    let detail: Value = client
                        .get_path(&endpoints, &format!("/txns/{hash}/receipts"))
                        .await?;
                    near_history_from_transaction(&detail["data"], &hash, &account)
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;
        let mut items = Vec::new();
        for entry in fetched {
            if let Some(entry) = entry? {
                items.push(entry);
            }
        }
        Ok(crate::api::HistoryPage { items, next_cursor })
    }
}

fn next_page(response: &Value) -> Result<Option<String>, ApiError> {
    if response.get("meta").is_none() && response["data"].as_array().is_some_and(Vec::is_empty) {
        return Ok(None);
    }
    let meta = response["meta"]
        .as_object()
        .or_decode("NEAR response: missing pagination metadata")?;
    match meta.get("next_page") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(ApiError::decode("NEAR response: invalid next cursor")),
    }
}

fn near_ft_decimals(meta: &Value) -> Result<u8, ApiError> {
    let value = meta["decimals"]
        .as_u64()
        .or_else(|| meta["decimals"].as_str()?.parse().ok())
        .or_decode("NEAR FT: missing precision")?;
    crate::api::checked_token_decimals(u128::from(value))
}

#[derive(Default)]
struct NearFtTotal {
    received: u128,
    spent: u128,
    decimals: u8,
    timestamp: u64,
    senders: std::collections::BTreeSet<String>,
    recipients: std::collections::BTreeSet<String>,
}

fn near_ft_history(rows: &[Value], account: &str, txid: &str) -> Result<Vec<Value>, ApiError> {
    let mut totals = std::collections::BTreeMap::<String, NearFtTotal>::new();
    let mut events = std::collections::HashSet::new();
    for row in rows {
        if row["affected_account_id"].as_str() != Some(account) {
            continue;
        }
        if !matches!(row["cause"].as_str(), Some("TRANSFER" | "MINT" | "BURN")) {
            continue;
        }
        let raw = row["delta_amount"]
            .as_str()
            .or_decode("NEAR FT history: missing amount")?;
        let incoming = !raw.starts_with('-');
        let amount: u128 = raw
            .strip_prefix('-')
            .unwrap_or(raw)
            .parse()
            .map_err(ApiError::decode)?;
        if amount == 0 {
            continue;
        }
        let contract = row["contract_account_id"]
            .as_str()
            .or_decode("NEAR FT history: missing contract")?;
        if row["meta"]["contract"]
            .as_str()
            .is_some_and(|value| value != contract)
        {
            return Err(ApiError::decode("NEAR FT metadata contract mismatch"));
        }
        let receipt = row["receipt_id"]
            .as_str()
            .filter(|value| !value.is_empty())
            .or_decode("NEAR FT history: missing receipt identity")?;
        let index = row["event_index"]
            .as_u64()
            .or_decode("NEAR FT history: missing event index")?;
        let event_type = row["event_type"]
            .as_u64()
            .or_decode("NEAR FT history: missing event type")?;
        if !events.insert((receipt, index, event_type, contract)) {
            return Err(ApiError::decode("NEAR FT transaction repeats an activity"));
        }
        let timestamp = crate::api::time::confirmed_history_time(
            row["block_timestamp"]
                .as_str()
                .and_then(|value| value.parse().ok()),
            txid,
        )?;
        let decimals = near_ft_decimals(&row["meta"])?;
        let total = totals
            .entry(contract.into())
            .or_insert_with(|| NearFtTotal {
                decimals,
                timestamp,
                ..Default::default()
            });
        if total.decimals != decimals {
            return Err(ApiError::decode(
                "NEAR FT transaction has inconsistent precision",
            ));
        }
        total.timestamp = total.timestamp.max(timestamp);
        let leg = if incoming {
            &mut total.received
        } else {
            &mut total.spent
        };
        *leg = leg
            .checked_add(amount)
            .or_decode("NEAR FT transaction amount overflow")?;
        if let Some(other) = row["involved_account_id"]
            .as_str()
            .filter(|other| *other != account)
        {
            if incoming {
                total.senders.insert(other.into());
            } else {
                total.recipients.insert(other.into());
            }
        }
    }
    Ok(totals.into_iter().filter(|(_, total)| total.received != total.spent).map(|(contract,total)| {
        let incoming = total.received > total.spent;
        let peers = if incoming { total.senders } else { total.recipients };
        let other = if peers.len() == 1 { peers.into_iter().next().unwrap_or_default() } else { String::new() };
        serde_json::json!({"txid":txid,"timestamp_ns":total.timestamp,"amount_display":crate::decimal::from_units(total.received.abs_diff(total.spent),u32::from(total.decimals)),"contract":contract,"from":if incoming { other.as_str() } else { account },"to":if incoming { account } else { other.as_str() },"is_incoming":incoming})
    }).collect())
}

/// Read every receipt in the originating transaction before presenting its
/// net amount. The account cursor may split one origin across pages; repeating
/// that origin still returns the same complete amount rather than partial legs.
fn near_history_from_transaction(
    root: &Value,
    txid: &str,
    account: &str,
) -> Result<Option<NearHistoryEntry>, ApiError> {
    let mut pending = vec![root];
    let mut seen = std::collections::HashSet::new();
    let mut received = 0u128;
    let mut sent = 0u128;
    let mut incoming_from = std::collections::BTreeSet::new();
    let mut outgoing_to = std::collections::BTreeSet::new();
    let timestamp = root
        .pointer("/block/block_timestamp")
        .and_then(Value::as_str)
        .and_then(|time| time.parse().ok());
    let timestamp_ns = crate::api::time::confirmed_history_time(timestamp, txid)?;
    while let Some(receipt) = pending.pop() {
        let id = receipt["receipt_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .or_decode("NEAR detail: missing receipt id")?;
        if !seen.insert(id) {
            continue;
        }
        if seen.len() > 1000 {
            return Err(ApiError::Rejected(
                "NEAR transaction exceeds 1000 receipts".into(),
            ));
        }
        let children = receipt["receipts"]
            .as_array()
            .or_decode("NEAR detail: incomplete receipt tree")?;
        pending.extend(children);
        let success = receipt
            .pointer("/outcome/status")
            .and_then(Value::as_bool)
            .or_decode("NEAR detail: receipt has no execution outcome")?;
        let predecessor = receipt["predecessor_account_id"]
            .as_str()
            .or_decode("NEAR detail: missing predecessor")?;
        let receiver = receipt["receiver_account_id"]
            .as_str()
            .or_decode("NEAR detail: missing receiver")?;
        if !success || predecessor == "system" || (predecessor == account) == (receiver == account)
        {
            continue;
        }
        let actions = receipt["actions"]
            .as_array()
            .or_decode("NEAR detail: missing actions")?;
        let mut deposit = 0u128;
        for action in actions {
            if !matches!(
                action["action"].as_str(),
                Some("TRANSFER" | "FUNCTION_CALL")
            ) {
                continue;
            }
            let amount = action
                .pointer("/args/deposit")
                .and_then(Value::as_str)
                .and_then(|amount| amount.parse::<u128>().ok())
                .or_decode("NEAR detail: malformed attached deposit")?;
            deposit = deposit
                .checked_add(amount)
                .or_decode("NEAR attached deposit overflow")?;
        }
        if receiver == account {
            received = received
                .checked_add(deposit)
                .or_decode("NEAR received amount overflow")?;
            if deposit > 0 {
                incoming_from.insert(predecessor.to_string());
            }
        } else {
            sent = sent
                .checked_add(deposit)
                .or_decode("NEAR sent amount overflow")?;
            if deposit > 0 {
                outgoing_to.insert(receiver.to_string());
            }
        }
    }
    if received == sent {
        return Ok(None);
    }
    let is_incoming = received > sent;
    Ok(Some(NearHistoryEntry {
        txid: txid.into(),
        timestamp_ns,
        from: if is_incoming && incoming_from.len() == 1 {
            incoming_from.into_iter().next().unwrap()
        } else if !is_incoming {
            account.into()
        } else {
            String::new()
        },
        to: if !is_incoming && outgoing_to.len() == 1 {
            outgoing_to.into_iter().next().unwrap()
        } else if is_incoming {
            account.into()
        } else {
            String::new()
        },
        amount_yocto: received.abs_diff(sent).to_string(),
        is_incoming,
    }))
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use serde_json::json;

    fn receipt(
        id: &str,
        from: &str,
        to: &str,
        amount: &str,
        success: bool,
        children: Vec<Value>,
    ) -> Value {
        json!({"receipt_id":id,"predecessor_account_id":from,"receiver_account_id":to,"actions":[{"action":"TRANSFER","args":{"deposit":amount}}],"outcome":{"status":success},"receipts":children,"block":{"block_timestamp":"1790508277775082937"}})
    }
    #[test]
    fn complete_origin_receipts_are_netted_exactly_without_refunds() {
        let root = receipt(
            "root",
            "me.near",
            "contract.near",
            "1",
            true,
            vec![
                receipt(
                    "received",
                    "contract.near",
                    "me.near",
                    "9007199254740993000001",
                    true,
                    vec![],
                ),
                receipt("refund", "system", "me.near", "999999", true, vec![]),
                receipt("failed", "me.near", "them.near", "100", false, vec![]),
            ],
        );
        let row = near_history_from_transaction(&root, "origin", "me.near")
            .unwrap()
            .unwrap();
        assert_eq!(row.amount_yocto, "9007199254740993000000");
        assert!(row.is_incoming);
        assert_eq!(row.from, "contract.near");
        // Re-reading the same origin on another provider page is identical.
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            serde_json::to_value(
                near_history_from_transaction(&root, "origin", "me.near")
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
        );
        let mut incomplete = root;
        incomplete["receipts"][0]["outcome"] = Value::Null;
        assert!(near_history_from_transaction(&incomplete, "origin", "me.near").is_err());
    }

    #[test]
    fn ft_history_uses_signed_integer_deltas_and_requires_real_precision() {
        let mut row = json!({"cause":"TRANSFER","affected_account_id":"me.near","involved_account_id":"them.near","delta_amount":"-9007199254740993000000","transaction_hash":"hash","receipt_id":"receipt","event_index":1,"event_type":1,"block_timestamp":"1791130826972666500","contract_account_id":"token.near","meta":{"decimals":6}});
        let entries = near_ft_history(&[row.clone()], "me.near", "hash").unwrap();
        assert_eq!(entries[0]["amount_display"], "9007199254740993");
        assert_eq!(entries[0]["is_incoming"], false);
        row["meta"]["decimals"] = json!(39);
        assert!(near_ft_history(&[row], "me.near", "hash").is_err());
    }
}

//! XRP (Ripple) chain client.
//!
//! Uses the XRP Ledger JSON-RPC / REST API (rippled / Clio).
//! Transactions are serialized using XRP's binary codec (STObject)
//! and signed with secp256k1.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::fetch::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpBalance {
    /// XRP drops (1 XRP = 1_000_000 drops).
    pub drops: u64,
    pub xrp_display: String,
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

pub struct XrpClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl XrpClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        crate::fetch::json_rpc::call(
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

impl XrpClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<XrpBalance, String> {
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
            .ok_or("account_info: missing Balance")?;
        Ok(XrpBalance {
            drops,
            xrp_display: format_xrp(drops),
        })
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u32, String> {
        let result = self
            .call(
                "account_info",
                json!({"account": address, "ledger_index": "current"}),
            )
            .await?;
        result
            .pointer("/account_data/Sequence")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .ok_or_else(|| "account_info: missing Sequence".to_string())
    }

    pub async fn fetch_fee(&self) -> Result<u64, String> {
        let result = self.call("fee", json!({})).await?;
        result
            .pointer("/drops/open_ledger_fee")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "fee: missing open_ledger_fee".to_string())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<XrpHistoryEntry>, String> {
        let result = self
            .call(
                "account_tx",
                json!({"account": address, "limit": 50, "ledger_index_min": -1, "ledger_index_max": -1}),
            )
            .await?;
        let txs = result
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        xrp_history_from_transactions(&txs, address)
    }
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
) -> Result<Vec<XrpHistoryEntry>, String> {
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
        let timestamp = super::confirmed_history_time(
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

fn format_xrp(drops: u64) -> String {
    let whole = drops / 1_000_000;
    let frac = drops % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
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

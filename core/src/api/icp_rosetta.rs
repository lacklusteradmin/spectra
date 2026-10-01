//! The ICP Rosetta adapter: ledger balances and history, the construction
//! calls that return what to sign, and submission of a signed envelope.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile, race};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpBalance {
    /// E8s (1 ICP = 100_000_000 e8s).
    pub e8s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpHistoryEntry {
    pub block_index: u64,
    pub timestamp_ns: u64,
    pub from: String,
    pub to: String,
    pub amount_e8s: u64,
    pub fee_e8s: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpSendResult {
    pub txid: String,
}

// ── Client (Rosetta-based for read, direct for write)

pub struct IcpClient {
    /// Rosetta API endpoint (https://rosetta-api.internetcomputer.org).
    rosetta_endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl IcpClient {
    pub fn new(rosetta_endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            rosetta_endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn rosetta_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, ApiError> {
        let is_submit = path == "/construction/submit";
        let path = path.to_string();
        let body = std::sync::Arc::new(body.clone());
        race(&self.rosetta_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = std::sync::Arc::clone(&body);
            async move {
                client
                    .post_json(
                        &url,
                        &*body,
                        if is_submit {
                            RetryProfile::ChainWrite
                        } else {
                            RetryProfile::ChainRead
                        },
                    )
                    .await
            }
        })
        .await
    }
}
// ICP fetch paths (via Rosetta): balance and history.

use serde_json::json;

/// Rosetta's name for the ICP ledger, sent with every construction call.
pub(crate) fn network_identifier() -> Value {
    json!({"blockchain":"Internet Computer","network":crate::registry::Chain::Icp.icp_ledger_id().expect("ICP registry")})
}

impl IcpClient {
    pub async fn fetch_balance(&self, account_address: &str) -> Result<IcpBalance, ApiError> {
        let resp: Value = self
            .rosetta_post(
                "/account/balance",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address}
                }),
            )
            .await?;
        let e8s: u64 = resp
            .pointer("/balances/0/value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Ok(IcpBalance { e8s })
    }

    pub async fn fetch_history(
        &self,
        account_address: &str,
    ) -> Result<Vec<IcpHistoryEntry>, ApiError> {
        let resp: Value = self
            .rosetta_post(
                "/search/transactions",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address},
                    "limit": 50
                }),
            )
            .await?;

        let txs = resp
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        icp_history_from_transactions(&txs, account_address)
    }
}

/// What each ledger transaction moved into or out of `account_address`,
/// fee excluded.
///
/// Read as the account's own balance change across the transfer, mint and
/// burn operations, so an approval — which moves nothing — is not a send.
///
/// Every ledger block has a time, so a transaction without one was misread.
fn icp_history_from_transactions(
    txs: &[Value],
    account_address: &str,
) -> Result<Vec<IcpHistoryEntry>, ApiError> {
    let mut entries = Vec::new();
    for item in txs {
        let block_index: u64 = item
            .pointer("/block_identifier/index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let timestamp = item
            .pointer("/transaction/metadata/timestamp")
            .and_then(Value::as_u64);
        let mut delta: i128 = 0;
        let mut counterparty = String::new();
        let mut fee_e8s: u64 = 0;
        for op in item
            .pointer("/transaction/operations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let addr = op
                .pointer("/account/address")
                .and_then(Value::as_str)
                .unwrap_or("");
            let value: i128 = op
                .pointer("/amount/value")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            match op.get("type").and_then(Value::as_str).unwrap_or("") {
                "TRANSACTION" | "MINT" | "BURN" if addr == account_address => delta += value,
                "TRANSACTION" => counterparty = addr.to_string(),
                "FEE" => fee_e8s = u64::try_from(value.unsigned_abs()).unwrap_or(0),
                _ => {}
            }
        }
        if delta == 0 {
            continue;
        }
        let Ok(amount_e8s) = u64::try_from(delta.unsigned_abs()) else {
            continue;
        };
        let timestamp_ns =
            crate::api::time::confirmed_history_time(timestamp, &block_index.to_string())?;
        let is_incoming = delta > 0;
        let (from, to) = if is_incoming {
            (counterparty, account_address.to_string())
        } else {
            (account_address.to_string(), counterparty)
        };
        entries.push(IcpHistoryEntry {
            block_index,
            timestamp_ns,
            from,
            to,
            amount_e8s,
            fee_e8s,
            is_incoming,
        });
    }
    Ok(entries)
}

impl IcpClient {
    pub(crate) async fn submit_signed_transaction(
        &self,
        payload: &str,
    ) -> Result<IcpSendResult, ApiError> {
        let body: Value = serde_json::from_str(payload).map_err(ApiError::invalid)?;
        let submit: Value = self.rosetta_post("/construction/submit", &body).await?;
        let txid = submit
            .pointer("/transaction_identifier/hash")
            .and_then(Value::as_str)
            .or_decode("submit: missing transaction hash")?;
        if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::Decode("submit: invalid transaction hash".into()));
        }
        Ok(IcpSendResult {
            txid: txid.to_lowercase(),
        })
    }
}

impl IcpClient {
    pub(crate) async fn verify_network(&self) -> Result<(), ApiError> {
        let response: Value = self
            .rosetta_post("/network/list", &json!({"metadata":{}}))
            .await?;
        if !response["network_identifiers"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row == &network_identifier()))
        {
            return Err(ApiError::Decode(
                "ICP endpoint does not serve the configured ledger".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "d4685b31b51450508aff0331584df7692a84467b680326f5c5f7d30ae711682f";
    const THEM: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn tx(index: u64, ops: Value) -> Value {
        json!({
            "block_identifier": {"index": index},
            "transaction": {"metadata": {"timestamp": 1u64}, "operations": ops}
        })
    }
    fn op(kind: &str, address: &str, value: &str) -> Value {
        json!({"type": kind, "account": {"address": address}, "amount": {"value": value}})
    }

    /// Rosetta `/search/transactions` operation shapes.
    #[test]
    fn transfers_mints_and_burns_are_the_accounts_own_change() {
        let txs = [
            tx(
                1,
                json!([
                    op("TRANSACTION", ME, "-150000000"),
                    op("TRANSACTION", THEM, "150000000"),
                    op("FEE", ME, "-10000")
                ]),
            ),
            tx(
                2,
                json!([
                    op("TRANSACTION", THEM, "-5"),
                    op("TRANSACTION", ME, "5"),
                    op("FEE", THEM, "-10000")
                ]),
            ),
            tx(3, json!([op("MINT", ME, "700")])),
            tx(4, json!([op("APPROVE", ME, "0"), op("FEE", ME, "-10000")])),
            tx(
                5,
                json!([op("TRANSACTION", THEM, "-9"), op("TRANSACTION", THEM, "9")]),
            ),
        ];
        let entries = icp_history_from_transactions(&txs, ME).unwrap();
        let got: Vec<(u64, bool, u64, &str)> = entries
            .iter()
            .map(|e| {
                let other = if e.is_incoming { &e.from } else { &e.to };
                (e.block_index, e.is_incoming, e.amount_e8s, other.as_str())
            })
            .collect();
        assert_eq!(
            got,
            [
                (1, false, 150_000_000, THEM),
                (2, true, 5, THEM),
                (3, true, 700, "")
            ]
        );
    }
}

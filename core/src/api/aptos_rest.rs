//! The Aptos REST adapter: account resources, token balances and decimals
//! through view functions, gas price, history, simulation and submission of
//! a signed body.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile, race};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosBalance {
    /// Octas (1 APT = 100_000_000 octas).
    pub octas: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosHistoryEntry {
    pub txid: String,
    pub version: u64,
    pub timestamp_us: u64,
    pub from: String,
    pub to: String,
    pub amount_octas: u64,
    pub gas_used: u64,
    pub gas_unit_price: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosSendResult {
    pub txid: String,
    pub version: Option<u64>,
    /// JSON-encoded signed transaction body — stored for rebroadcast.
    pub signed_body_json: String,
}

/// What an Aptos token identifier names.
///
/// A fungible asset (AIP-21), which every catalog token is, is named by the
/// address of its metadata object; a legacy coin by its Move type,
/// `0xADDR::module::Name`. Those are the two shapes `aptosTokenType` accepts,
/// and an address never contains `::`. The catalog's `standard` cannot tell
/// them apart: the network has one, and a custom token takes it either way.
#[derive(Debug, PartialEq)]
enum AptosToken<'a> {
    Asset { metadata: &'a str },
    Coin { coin_type: &'a str },
}

impl<'a> AptosToken<'a> {
    fn parse(identifier: &'a str) -> Self {
        let identifier = identifier.trim();
        if identifier.contains("::") {
            AptosToken::Coin {
                coin_type: identifier,
            }
        } else {
            AptosToken::Asset {
                metadata: identifier,
            }
        }
    }
}

/// The type argument naming a fungible asset by its metadata object.
const FUNGIBLE_ASSET_METADATA: &str = "0x1::fungible_asset::Metadata";

/// A view function's first return value as a number. The node writes a
/// `u64` as a decimal string and a `u8` as a JSON number.
fn returned_number(values: &[Value]) -> Option<u64> {
    match values.first()? {
        Value::String(s) => s.parse().ok(),
        other => other.as_u64(),
    }
}

// ── Client

pub struct AptosClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl AptosClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    pub(crate) async fn post_val(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let path = path.to_string();
        let body = std::sync::Arc::new(body.clone());
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = std::sync::Arc::clone(&body);
            async move {
                client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await
            }
        })
        .await
    }
}
// Aptos fetch paths: balance, token balance and decimals, account info,
// ledger info, gas price, history.

impl AptosClient {
    /// APT is a coin like any other to `0x1::coin::balance`. An account that
    /// holds it only as the migrated fungible asset, as accounts created
    /// since the migration do, has no `CoinStore<AptosCoin>` to read.
    pub async fn fetch_balance(&self, address: &str) -> Result<AptosBalance, ApiError> {
        let octas = self
            .fetch_token_balance(address, "0x1::aptos_coin::AptosCoin")
            .await?;
        Ok(AptosBalance { octas })
    }

    /// The values a Move view function returns, in order.
    async fn view(
        &self,
        function: &str,
        type_arguments: &[&str],
        arguments: &[&str],
    ) -> Result<Vec<Value>, ApiError> {
        let body = serde_json::json!({
            "function": function,
            "type_arguments": type_arguments,
            "arguments": arguments,
        });
        match self.post_val("/view", &body).await? {
            Value::Array(values) => Ok(values),
            other => Err(ApiError::Decode(format!(
                "view {function}: expected an array, got {other}"
            ))),
        }
    }

    /// What `owner` holds of a token, in its smallest unit.
    ///
    /// A fungible asset is read from the owner's primary store, which reads
    /// as zero when the owner has none. A coin is read through
    /// `0x1::coin::balance`, which adds the `CoinStore<T>` and the store of
    /// the asset the coin was migrated to: an account that holds the coin
    /// only as an asset has no `CoinStore` to read.
    pub async fn fetch_token_balance(
        &self,
        owner: &str,
        identifier: &str,
    ) -> Result<u64, ApiError> {
        let values = match AptosToken::parse(identifier) {
            AptosToken::Asset { metadata } => {
                self.view(
                    "0x1::primary_fungible_store::balance",
                    &[FUNGIBLE_ASSET_METADATA],
                    &[owner, metadata],
                )
                .await?
            }
            AptosToken::Coin { coin_type } => {
                self.view("0x1::coin::balance", &[coin_type], &[owner])
                    .await?
            }
        };
        returned_number(&values)
            .ok_or_else(|| ApiError::Decode(format!("aptos: no balance returned for {identifier}")))
    }

    /// A token's own decimals, from its metadata object or its `CoinInfo<T>`.
    /// `None` when it is unreadable.
    pub async fn fetch_token_decimals(&self, identifier: &str) -> Option<u8> {
        let values = match AptosToken::parse(identifier) {
            AptosToken::Asset { metadata } => {
                self.view(
                    "0x1::fungible_asset::decimals",
                    &[FUNGIBLE_ASSET_METADATA],
                    &[metadata],
                )
                .await
            }
            AptosToken::Coin { coin_type } => {
                self.view("0x1::coin::decimals", &[coin_type], &[]).await
            }
        };
        u8::try_from(returned_number(&values.ok()?)?).ok()
    }

    pub async fn fetch_account_info(&self, address: &str) -> Result<(u64, u64), ApiError> {
        let resp: Value = self.get(&format!("/accounts/{address}")).await?;
        let sequence: u64 = resp
            .get("sequence_number")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("account: missing sequence_number")?;
        Ok((sequence, 0))
    }

    pub async fn fetch_ledger_info(&self) -> Result<(u64, String), ApiError> {
        let resp: Value = self.get("/").await?;
        let chain_id: u64 = resp
            .get("chain_id")
            .and_then(|v| v.as_u64())
            .or_decode("ledger: missing chain_id")?;
        let ledger_version: String = resp
            .get("ledger_version")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        Ok((chain_id, ledger_version))
    }

    pub async fn fetch_gas_price(&self) -> Result<u64, ApiError> {
        let resp: Value = self.get("/estimate_gas_price").await?;
        resp.get("gas_estimate")
            .and_then(|v| v.as_u64())
            .or_decode("estimate_gas_price: missing gas_estimate")
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<AptosHistoryEntry>, ApiError> {
        let txs: Vec<Value> = self
            .get(&format!("/accounts/{address}/transactions?limit=50"))
            .await?;

        aptos_history_from_transactions(&txs, address)
    }
}

/// The recipient and octas of a successful user transaction that moves APT
/// through one of the framework's transfer entry functions.
///
/// Only those functions count: a token's transfer is not APT, and a
/// fungible-asset transfer's first argument is the asset, not the recipient.
fn aptos_native_transfer(tx: &Value) -> Option<(String, u64)> {
    if tx.get("type").and_then(Value::as_str) != Some("user_transaction")
        || tx.get("success").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let payload = tx.get("payload")?;
    let function = payload.get("function")?.as_str()?;
    let args = payload.get("arguments")?.as_array()?;
    let type_args: Vec<&str> = payload
        .get("type_arguments")
        .and_then(Value::as_array)
        .map(|args| args.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let is_apt_coin = type_args.as_slice() == ["0x1::aptos_coin::AptosCoin"];
    let is_apt_asset = |metadata: &Value| {
        metadata
            .get("inner")
            .and_then(Value::as_str)
            .map(|inner| inner.trim_start_matches("0x").trim_start_matches('0') == "a")
            .unwrap_or(false)
    };
    let (to, amount) = match function {
        "0x1::aptos_account::transfer" => (args.first()?, args.get(1)?),
        "0x1::aptos_account::transfer_coins" | "0x1::coin::transfer" if is_apt_coin => {
            (args.first()?, args.get(1)?)
        }
        "0x1::primary_fungible_store::transfer"
        | "0x1::aptos_account::transfer_fungible_assets"
            if is_apt_asset(args.first()?) =>
        {
            (args.get(1)?, args.get(2)?)
        }
        _ => return None,
    };
    Some((to.as_str()?.to_string(), amount.as_str()?.parse().ok()?))
}

/// Committed user transactions only, so each has a time.
fn aptos_history_from_transactions(
    txs: &[Value],
    address: &str,
) -> Result<Vec<AptosHistoryEntry>, ApiError> {
    let number = |tx: &Value, field: &str| -> u64 {
        tx.get(field)
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };
    let mut entries = Vec::new();
    for tx in txs {
        let Some((to, amount_octas)) = aptos_native_transfer(tx) else {
            continue;
        };
        let Some(txid) = tx.get("hash").and_then(Value::as_str) else {
            continue;
        };
        if amount_octas == 0 {
            continue;
        }
        let timestamp_us =
            crate::api::time::confirmed_history_time(Some(number(tx, "timestamp")), txid)?;
        entries.push(AptosHistoryEntry {
            txid: txid.to_string(),
            version: number(tx, "version"),
            timestamp_us,
            from: tx
                .get("sender")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            is_incoming: to.eq_ignore_ascii_case(address),
            to,
            amount_octas,
            gas_used: number(tx, "gas_used"),
            gas_unit_price: number(tx, "gas_unit_price"),
        });
    }
    Ok(entries)
}

impl AptosClient {
    pub async fn submit_signed_body(&self, signed_json: &str) -> Result<AptosSendResult, ApiError> {
        let body: Value = serde_json::from_str(signed_json)
            .map_err(|e| ApiError::InvalidInput(format!("invalid Aptos transaction: {e}")))?;
        let response = self.post_val("/transactions", &body).await?;
        let txid = response["hash"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_decode("Aptos submit: missing hash")?
            .to_string();
        let version = response["version"].as_str().and_then(|s| s.parse().ok());
        Ok(AptosSendResult {
            txid,
            version,
            signed_body_json: signed_json.into(),
        })
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use serde_json::json;

    const ME: &str = "0x6a2c9b3d1f64c2ae7c0e79c56b4b2f8e07c1b5f2aa7ec8bb0e0f2e7c4b6d8e1a";
    const THEM: &str = "0x1f64c2ae7c0e79c56b4b2f8e07c1b5f2aa7ec8bb0e0f2e7c4b6d8e1a6a2c9b3d";

    fn tx(
        hash: &str,
        function: &str,
        type_arguments: Value,
        arguments: Value,
        success: bool,
    ) -> Value {
        json!({
            "type": "user_transaction", "hash": hash, "version": "1", "timestamp": "1",
            "sender": ME, "success": success, "gas_used": "10", "gas_unit_price": "100",
            "payload": {"function": function, "type_arguments": type_arguments, "arguments": arguments}
        })
    }

    #[test]
    fn only_apt_moved_by_a_framework_transfer_is_an_entry() {
        let usdc = "0x5e156f1207d0ebfa19a9eeff00d62a282278fb8719f4fab3a586a0a2c0fffbea::coin::T";
        let txs = [
            tx(
                "apt",
                "0x1::aptos_account::transfer",
                json!([]),
                json!([THEM, "150000000"]),
                true,
            ),
            tx(
                "coin",
                "0x1::coin::transfer",
                json!(["0x1::aptos_coin::AptosCoin"]),
                json!([THEM, "2"]),
                true,
            ),
            tx(
                "fa",
                "0x1::primary_fungible_store::transfer",
                json!(["0x1::fungible_asset::Metadata"]),
                json!([{"inner": "0xa"}, THEM, "3"]),
                true,
            ),
            tx(
                "token",
                "0x1::coin::transfer",
                json!([usdc]),
                json!([THEM, "5000000"]),
                true,
            ),
            tx(
                "other-fa",
                "0x1::primary_fungible_store::transfer",
                json!(["0x1::fungible_asset::Metadata"]),
                json!([{"inner": "0xbae207659db88bea0cbead6da0ed00aac12edcdda169e591cd41c94180b46f3b"}, THEM, "7"]),
                true,
            ),
            tx(
                "failed",
                "0x1::aptos_account::transfer",
                json!([]),
                json!([THEM, "9"]),
                false,
            ),
            tx(
                "nft",
                "0x4::aptos_token::transfer",
                json!([]),
                json!([{"inner": "0x1"}, THEM]),
                true,
            ),
        ];
        let entries = aptos_history_from_transactions(&txs, ME).unwrap();
        let hashes: Vec<&str> = entries.iter().map(|e| e.txid.as_str()).collect();
        assert_eq!(hashes, ["apt", "coin", "fa"]);
        assert!(entries.iter().all(|e| e.to == THEM && !e.is_incoming));
        assert_eq!(entries[0].amount_octas, 150_000_000);
        assert_eq!(entries[2].amount_octas, 3);
    }
}

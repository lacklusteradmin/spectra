//! The Sui JSON-RPC adapter: balances, coin objects, gas price, transaction
//! history and execution of a signed transaction.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiBalance {
    /// MIST (1 SUI = 1_000_000_000 MIST).
    pub mist: u64,
}

/// One transaction's effect on an address's SUI, fee excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiHistoryEntry {
    pub digest: String,
    /// `None` until the transaction is in a checkpoint, which is what dates it.
    pub timestamp_ms: Option<u64>,
    pub is_incoming: bool,
    pub amount_mist: u64,
    /// The sender when incoming; the largest other recipient when outgoing.
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiSendResult {
    /// Base64 tx bytes — stored for rebroadcast.
    pub tx_bytes_b64: String,
    /// Base64 signature — stored for rebroadcast.
    pub sig_b64: String,
    pub digest: String,
}

// ── Client

pub struct SuiClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl SuiClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            crate::EndpointApi::SuiJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }
}

// Sui fetch paths: native balance, per-coin balance, history.

impl SuiClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<SuiBalance, ApiError> {
        let result = self
            .call("suix_getBalance", json!([address, "0x2::sui::SUI"]))
            .await?;
        let mist: u64 = result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("suix_getBalance: missing totalBalance")?;
        Ok(SuiBalance { mist })
    }

    /// The address's SUI transfers, newest first.
    ///
    /// Queried as sender and as recipient, since neither filter alone sees
    /// both directions, and read from each transaction's balance changes.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<SuiHistoryEntry>, ApiError> {
        let mut blocks = Vec::new();
        for filter in ["FromAddress", "ToAddress"] {
            let result = self
                .call(
                    "suix_queryTransactionBlocks",
                    json!([
                        {
                            "filter": {filter: address},
                            "options": {
                                "showInput": true,
                                "showEffects": true,
                                "showBalanceChanges": true
                            }
                        },
                        null,
                        25,
                        true
                    ]),
                )
                .await?;
            blocks.extend(
                result
                    .get("data")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        sui_history_from_blocks(&blocks, address)
    }

    /// A coin type's own decimals, as the node reports them.
    ///
    /// `None` when the type has no metadata — a caller then falls back to what
    /// it was told, which is the only case where a catalog number is used.
    pub async fn fetch_coin_decimals(&self, coin_type: &str) -> Option<u8> {
        self.call("suix_getCoinMetadata", json!([coin_type]))
            .await
            .ok()?
            .get("decimals")?
            .as_u64()
            .map(|d| d as u8)
    }

    /// Every coin type the address holds, as the node reports it.
    ///
    /// `suix_getAllBalances` returns coin types and totals but no decimals; a
    /// caller reads those for the coin types it needs.
    pub async fn fetch_all_coin_balances(
        &self,
        address: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        let result = self.call("suix_getAllBalances", json!([address])).await?;
        let mut held: Vec<(String, u128)> = Vec::new();
        for entry in result.as_array().map(|v| v.as_slice()).unwrap_or_default() {
            let Some(coin_type) = entry.get("coinType").and_then(|v| v.as_str()) else {
                continue;
            };
            if coin_type.ends_with("::sui::SUI") {
                continue;
            }
            let raw = entry
                .get("totalBalance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            if raw == 0 {
                continue;
            }
            held.push((coin_type.to_string(), raw));
        }

        Ok(held
            .into_iter()
            .map(|(contract, balance_raw)| crate::api::HeldToken {
                contract,
                balance_raw,
                decimals: None,
            })
            .collect())
    }

    /// Fetch the balance for a specific coin type (e.g. `0x5d4b...::coin::COIN`).
    /// Returns the raw balance in the coin's smallest unit.
    pub async fn fetch_coin_balance(
        &self,
        address: &str,
        coin_type: &str,
    ) -> Result<u64, ApiError> {
        let result = self
            .call("suix_getBalance", json!([address, coin_type]))
            .await?;
        result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                ApiError::Decode(format!(
                    "suix_getBalance: missing totalBalance for {coin_type}"
                ))
            })
    }
}

const SUI_COIN_TYPE: &str = "0x2::sui::SUI";

/// Each transaction block's SUI transfer for `address`, deduplicated by
/// digest and newest first.
///
/// A balance change for the gas owner includes the gas, which is a fee and not
/// a transfer, so it is added back: a transaction that only paid gas moved
/// nothing and yields no entry. Gas can be negative when a storage rebate
/// exceeds the cost, and the same arithmetic holds.
fn sui_history_from_blocks(
    blocks: &[Value],
    address: &str,
) -> Result<Vec<SuiHistoryEntry>, ApiError> {
    let address = address.to_lowercase();
    let owner_of = |change: &Value| {
        change
            .pointer("/owner/AddressOwner")
            .and_then(Value::as_str)
            .map(str::to_lowercase)
    };
    let int = |value: Option<&Value>| -> i128 {
        value
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for block in blocks {
        let Some(digest) = block.get("digest").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(digest.to_string()) {
            continue;
        }
        let sui_changes: Vec<(String, i128)> = block
            .get("balanceChanges")
            .and_then(Value::as_array)
            .map(|changes| {
                changes
                    .iter()
                    .filter(|c| c.get("coinType").and_then(Value::as_str) == Some(SUI_COIN_TYPE))
                    .filter_map(|c| Some((owner_of(c)?, int(c.get("amount")))))
                    .collect()
            })
            .unwrap_or_default();
        let net: i128 = sui_changes
            .iter()
            .filter(|(owner, _)| *owner == address)
            .map(|(_, amount)| amount)
            .sum();
        let gas_owner = block
            .pointer("/transaction/data/gasData/owner")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let gas = if gas_owner.as_deref() == Some(address.as_str()) {
            let used = block.pointer("/effects/gasUsed");
            int(used.and_then(|u| u.get("computationCost")))
                + int(used.and_then(|u| u.get("storageCost")))
                - int(used.and_then(|u| u.get("storageRebate")))
        } else {
            0
        };
        let transfer = net + gas;
        if transfer == 0 {
            continue;
        }
        let Ok(amount_mist) = u64::try_from(transfer.unsigned_abs()) else {
            continue;
        };
        let sender = block
            .pointer("/transaction/data/sender")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let is_incoming = transfer > 0;
        let (from, to) = if is_incoming {
            (sender, address.clone())
        } else {
            let recipient = sui_changes
                .iter()
                .filter(|(owner, amount)| *owner != address && *amount > 0)
                .max_by_key(|(_, amount)| *amount)
                .map(|(owner, _)| owner.clone())
                .unwrap_or_default();
            (address.clone(), recipient)
        };
        let timestamp_ms = crate::api::time::history_time(
            block.get("checkpoint").is_some_and(|c| !c.is_null()),
            block
                .get("timestampMs")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok()),
            digest,
        )?;
        entries.push(SuiHistoryEntry {
            digest: digest.to_string(),
            timestamp_ms,
            is_incoming,
            amount_mist,
            from,
            to,
        });
    }
    // Undated transactions are the newest.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms.unwrap_or(u64::MAX)));
    Ok(entries)
}

impl SuiClient {
    pub async fn execute_signed_tx(
        &self,
        tx_bytes_b64: &str,
        sig_b64: &str,
    ) -> Result<SuiSendResult, ApiError> {
        let result = self
            .call(
                "sui_executeTransactionBlock",
                json!([tx_bytes_b64,[sig_b64],{"showEffects":true},"WaitForLocalExecution"]),
            )
            .await?;
        if result
            .pointer("/effects/status/status")
            .and_then(Value::as_str)
            != Some("success")
        {
            return Err(ApiError::Rejected(format!(
                "Sui execution did not succeed: {result}"
            )));
        }
        let digest = result["digest"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_decode("missing Sui transaction digest")?
            .to_string();
        Ok(SuiSendResult {
            digest,
            tx_bytes_b64: tx_bytes_b64.into(),
            sig_b64: sig_b64.into(),
        })
    }
}

/// One SUI coin object, as gas or as the amount sent.
pub struct SuiCoin {
    pub object_id: String,
    pub version: u64,
    pub digest: [u8; 32],
    pub balance: u64,
}

/// A page of `suix_getCoins`; `next_cursor` is `None` on the last page.
pub struct SuiCoinPage {
    pub coins: Vec<SuiCoin>,
    pub next_cursor: Option<String>,
}

impl SuiClient {
    pub async fn fetch_reference_gas_price(&self) -> Result<u64, ApiError> {
        self.call("suix_getReferenceGasPrice", json!([]))
            .await?
            .as_str()
            .and_then(|s| s.parse().ok())
            .or_decode("missing Sui reference gas price")
    }

    /// Up to 50 of `owner`'s SUI coins, from `cursor` on.
    pub async fn fetch_sui_coins_page(
        &self,
        owner: &str,
        cursor: Option<&str>,
    ) -> Result<SuiCoinPage, ApiError> {
        let page = self
            .call("suix_getCoins", json!([owner, "0x2::sui::SUI", cursor, 50]))
            .await?;
        let coins = page["data"]
            .as_array()
            .or_decode("missing Sui coins")?
            .iter()
            .map(|row| {
                Ok(SuiCoin {
                    object_id: row["coinObjectId"]
                        .as_str()
                        .or_decode("missing Sui coin id")?
                        .to_string(),
                    version: row["version"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_decode("missing Sui coin version")?,
                    digest: bs58::decode(
                        row["digest"]
                            .as_str()
                            .or_decode("missing Sui coin digest")?,
                    )
                    .into_vec()
                    .map_err(|_| ApiError::Decode("invalid Sui coin digest".into()))?
                    .try_into()
                    .map_err(|_| ApiError::Decode("Sui digest must be 32 bytes".into()))?,
                    balance: row["balance"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_decode("invalid Sui coin balance")?,
                })
            })
            .collect::<Result<_, ApiError>>()?;
        let next_cursor = if page["hasNextPage"].as_bool() == Some(false) {
            None
        } else {
            Some(
                page.get("nextCursor")
                    .and_then(Value::as_str)
                    .or_decode("missing Sui coin cursor")?
                    .to_string(),
            )
        };
        Ok(SuiCoinPage { coins, next_cursor })
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "0x7ab9a6a7109dcb9cb357a109f32dfcc78a7aa2d6029084eb924d95133fc71cec";
    const THEM: &str = "0xac5bceec1b789ff840d7d4e6ce4ce61c90d190a7f8c4f4ddf0bff6ee2413c33c";

    fn block(digest: &str, sender: &str, gas: [&str; 3], changes: Value) -> Value {
        json!({
            "digest": digest,
            "timestampMs": "1790242671829",
            "checkpoint": "1",
            "transaction": {"data": {"sender": sender, "gasData": {"owner": sender}}},
            "effects": {"gasUsed": {
                "computationCost": gas[0], "storageCost": gas[1], "storageRebate": gas[2]
            }},
            "balanceChanges": changes,
        })
    }

    fn change(owner: &str, coin: &str, amount: &str) -> Value {
        json!({"owner": {"AddressOwner": owner}, "coinType": coin, "amount": amount})
    }

    /// Shapes taken from mainnet `suix_queryTransactionBlocks` responses.
    #[test]
    fn transfers_are_read_from_balance_changes_without_gas() {
        let received = block(
            "in",
            THEM,
            ["100000", "988000", "978120"],
            json!([
                change(ME, SUI_COIN_TYPE, "4770000000000"),
                change(THEM, SUI_COIN_TYPE, "-4770000109880"),
            ]),
        );
        let sent = block(
            "out",
            ME,
            ["100000", "1976000", "978120"],
            json!([
                change(THEM, SUI_COIN_TYPE, "2500100000000000"),
                change(ME, SUI_COIN_TYPE, "-2500100001097880"),
            ]),
        );
        let entries = sui_history_from_blocks(&[received, sent.clone(), sent], ME).unwrap();
        assert_eq!(
            entries.len(),
            2,
            "a block seen by both queries is one entry"
        );
        let incoming = entries.iter().find(|e| e.digest == "in").unwrap();
        assert!(incoming.is_incoming);
        assert_eq!(incoming.amount_mist, 4_770_000_000_000);
        assert_eq!(incoming.from, THEM);
        let outgoing = entries.iter().find(|e| e.digest == "out").unwrap();
        assert!(!outgoing.is_incoming);
        assert_eq!(
            outgoing.amount_mist, 2_500_100_000_000_000,
            "gas is not part of it"
        );
        assert_eq!(outgoing.to, THEM);
    }

    /// A block not yet in a checkpoint has no time and is undated; one in a
    /// checkpoint without a time was read wrongly.
    #[test]
    fn only_an_uncheckpointed_block_is_undated() {
        let mut pending = block(
            "pending",
            THEM,
            ["0", "0", "0"],
            json!([change(ME, SUI_COIN_TYPE, "5")]),
        );
        let fields = pending.as_object_mut().unwrap();
        fields.remove("timestampMs");
        fields.remove("checkpoint");
        let entries = sui_history_from_blocks(std::slice::from_ref(&pending), ME).unwrap();
        assert_eq!(entries[0].timestamp_ms, None);
        pending["checkpoint"] = json!("9");
        assert!(sui_history_from_blocks(&[pending], ME).is_err());
    }

    /// A transaction whose only SUI effect is gas — here a net storage rebate
    /// while another coin moved — transfers no SUI.
    #[test]
    fn gas_only_and_other_coin_transactions_yield_nothing() {
        let rebate_only = block(
            "swap",
            ME,
            ["102000", "3663200", "30111048"],
            json!([
                change(ME, SUI_COIN_TYPE, "26345848"),
                change(ME, "0x6::cetus::CETUS", "-2699970876000000"),
                change(THEM, "0x6::cetus::CETUS", "2699970876000000"),
            ]),
        );
        assert!(
            sui_history_from_blocks(&[rebate_only], ME)
                .unwrap()
                .is_empty()
        );
    }
}

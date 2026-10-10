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
    pub amount_mist: String,
    pub contract: Option<String>,
    pub amount_display: Option<String>,
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
    pub(crate) async fn fetch_transaction_status(
        &self,
        digest: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        crate::api::transaction_status::validate_base58_hash(digest, 32)?;
        let response = self
            .call(
                "sui_getTransactionBlock",
                json!([digest, {"showEffects":true}]),
            )
            .await?;
        sui_transaction_status(&response, digest)
    }

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

fn sui_transaction_status(
    response: &Value,
    digest: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    if response["digest"].as_str() != Some(digest) {
        return Err(ApiError::decode("Sui status: transaction digest mismatch"));
    }
    let Some(checkpoint) = response.get("checkpoint").filter(|value| !value.is_null()) else {
        return Ok(TransactionStatus::Pending);
    };
    let checkpoint = checkpoint
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .or_decode("Sui status: invalid checkpoint")?;
    let succeeded = match response
        .pointer("/effects/status/status")
        .and_then(Value::as_str)
    {
        Some("success") => true,
        Some("failure") => false,
        _ => return Err(ApiError::decode("Sui status: missing execution result")),
    };
    Ok(TransactionStatus::Confirmed {
        succeeded,
        block: Some(checkpoint),
    })
}

// Sui fetch paths: native balance, per-coin balance, history.

impl SuiClient {
    pub async fn verify_network(&self, chain: crate::registry::Chain) -> Result<(), ApiError> {
        let (identifier, genesis) = chain
            .sui_network_identity()
            .or_decode("Unsupported Sui network")?;
        let actual = self.call("sui_getChainIdentifier", json!([])).await?;
        if actual.as_str() != Some(identifier) {
            return Err(ApiError::invalid("Sui endpoint is on the wrong network"));
        }
        let checkpoint = self.call("sui_getCheckpoint", json!(["0"])).await?;
        if checkpoint["sequenceNumber"].as_str() != Some("0")
            || checkpoint["digest"].as_str() != Some(genesis)
        {
            return Err(ApiError::invalid(
                "Sui endpoint has the wrong genesis checkpoint",
            ));
        }
        Ok(())
    }

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

    /// A page of the address's SUI transfers, newest first.
    ///
    /// Queried as sender and as recipient, since neither filter alone sees
    /// both directions, and read from each transaction's balance changes.
    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<SuiHistoryEntry>, ApiError> {
        #[derive(Default, serde::Serialize, serde::Deserialize)]
        struct Cursor {
            from: Option<Value>,
            to: Option<Value>,
            from_done: bool,
            to_done: bool,
        }
        let mut position: Cursor = cursor
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        let mut blocks = Vec::new();
        for filter in ["FromAddress", "ToAddress"] {
            let (after, done) = if filter == "FromAddress" {
                (&mut position.from, &mut position.from_done)
            } else {
                (&mut position.to, &mut position.to_done)
            };
            if *done {
                continue;
            }
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
                        after.clone(),
                        25,
                        true
                    ]),
                )
                .await?;
            let count = result
                .get("data")
                .and_then(Value::as_array)
                .or_decode("Sui history: missing page rows")?
                .len();
            *done = result
                .get("hasNextPage")
                .and_then(Value::as_bool)
                .map(|more| !more)
                .unwrap_or(count < 25);
            if !*done {
                *after = Some(
                    result
                        .get("nextCursor")
                        .filter(|value| !value.is_null())
                        .or_decode("Sui history: missing next cursor")?
                        .clone(),
                );
            }
            blocks.extend(
                result
                    .get("data")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        let next_cursor = (!(position.from_done && position.to_done))
            .then(|| serde_json::to_string(&position))
            .transpose()?;
        let mut items = sui_history_from_blocks(&blocks, address)?;
        if items
            .iter()
            .filter_map(|entry| entry.contract.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 100
        {
            return Err(ApiError::Rejected(
                "Sui history page exceeds 100 token types".into(),
            ));
        }
        let mut decimals = std::collections::HashMap::new();
        for entry in &mut items {
            if let Some(contract) = &entry.contract {
                let precision = if let Some(precision) = decimals.get(contract) {
                    *precision
                } else {
                    let metadata = self.call("suix_getCoinMetadata", json!([contract])).await?;
                    let precision = crate::api::checked_token_decimals(
                        metadata["decimals"]
                            .as_u64()
                            .map(u128::from)
                            .or_decode("Sui token history: missing precision")?,
                    )?;
                    decimals.insert(contract.clone(), precision);
                    precision
                };
                entry.amount_display =
                    crate::decimal::from_unit_digits(&entry.amount_mist, u32::from(precision));
                if entry.amount_display.is_none() {
                    return Err(ApiError::Decode("Sui token history: invalid amount".into()));
                }
            }
        }
        Ok(crate::api::HistoryPage { items, next_cursor })
    }

    /// A coin type's own decimals, as the node reports them.
    ///
    /// `None` when metadata is absent or exceeds core's supported precision.
    pub async fn fetch_coin_decimals(&self, coin_type: &str) -> Option<u8> {
        self.call("suix_getCoinMetadata", json!([coin_type]))
            .await
            .ok()?
            .get("decimals")?
            .as_u64()
            .and_then(|d| crate::api::checked_token_decimals(u128::from(d)).ok())
    }

    /// Each coin type the address holds, SUI included: its type, how many
    /// objects hold it and their total, as the node reports them.
    pub(crate) async fn fetch_coin_object_counts(
        &self,
        address: &str,
    ) -> Result<Vec<(String, u64, u128)>, ApiError> {
        self.call("suix_getAllBalances", json!([address]))
            .await?
            .as_array()
            .or_decode("suix_getAllBalances: missing list")?
            .iter()
            .map(|entry| {
                Ok((
                    entry["coinType"]
                        .as_str()
                        .or_decode("suix_getAllBalances: missing coin type")?
                        .to_string(),
                    entry["coinObjectCount"]
                        .as_u64()
                        .or_decode("suix_getAllBalances: missing object count")?,
                    entry["totalBalance"]
                        .as_str()
                        .and_then(|total| total.parse().ok())
                        .or_decode("suix_getAllBalances: missing total")?,
                ))
            })
            .collect()
    }

    /// What a transaction would cost, from a dry run: computation, storage
    /// and the storage rebate, in MIST. Refuses one that would not succeed.
    pub(crate) async fn dry_run_gas(&self, bytes: &[u8]) -> Result<(u64, u64, u64), ApiError> {
        use base64::Engine;
        let result = self
            .call(
                "sui_dryRunTransactionBlock",
                json!([base64::engine::general_purpose::STANDARD.encode(bytes)]),
            )
            .await?;
        if result["effects"]["status"]["status"].as_str() != Some("success") {
            return Err(ApiError::invalid(format!(
                "Sui dry run refused: {}",
                result["effects"]["status"]
            )));
        }
        let cost = |field: &str| {
            result["effects"]["gasUsed"][field]
                .as_str()
                .and_then(|value| value.parse().ok())
                .or_decode("Sui dry run: missing gas used")
        };
        Ok((
            cost("computationCost")?,
            cost("storageCost")?,
            cost("storageRebate")?,
        ))
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
    let int = |value: Option<&Value>| -> Result<i128, ApiError> {
        value
            .and_then(Value::as_str)
            .and_then(|amount| amount.parse().ok())
            .or_decode("Sui history: malformed integer amount")
    };
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for block in blocks {
        let digest = block
            .get("digest")
            .and_then(Value::as_str)
            .filter(|digest| !digest.is_empty())
            .or_decode("Sui history: missing digest")?;
        if !seen.insert(digest.to_string()) {
            continue;
        }
        let coin_types: std::collections::BTreeSet<_> = block["balanceChanges"]
            .as_array()
            .or_decode("Sui history: missing balance changes")?
            .iter()
            .filter(|change| owner_of(change).as_deref() == Some(address.as_str()))
            .map(|change| {
                change["coinType"]
                    .as_str()
                    .or_decode("Sui history: missing coin type")
            })
            .collect::<Result<_, _>>()?;
        for coin_type in coin_types {
            let mut sui_changes = Vec::new();
            for change in block["balanceChanges"]
                .as_array()
                .or_decode("Sui history: missing balance changes")?
            {
                if change["coinType"].as_str() != Some(coin_type) {
                    continue;
                }
                if let Some(owner) = owner_of(change) {
                    sui_changes.push((owner, int(change.get("amount"))?));
                }
            }
            let net = sui_changes
                .iter()
                .filter(|(owner, _)| owner == &address)
                .try_fold(0i128, |total, (_, amount)| {
                    total
                        .checked_add(*amount)
                        .or_decode("Sui history amount overflow")
                })?;
            let gas_owner = block
                .pointer("/transaction/data/gasData/owner")
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            let gas =
                if coin_type == SUI_COIN_TYPE && gas_owner.as_deref() == Some(address.as_str()) {
                    let used = block.pointer("/effects/gasUsed");
                    int(used.and_then(|u| u.get("computationCost")))?
                        .checked_add(int(used.and_then(|u| u.get("storageCost")))?)
                        .and_then(|cost| {
                            cost.checked_sub(int(used.and_then(|u| u.get("storageRebate"))).ok()?)
                        })
                        .or_decode("Sui history gas amount overflow")?
                } else {
                    0
                };
            let transfer = net
                .checked_add(gas)
                .or_decode("Sui history net amount overflow")?;
            if transfer == 0 {
                continue;
            }
            let amount_mist = transfer.unsigned_abs().to_string();
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
                amount_display: if coin_type == SUI_COIN_TYPE {
                    crate::decimal::from_unit_digits(&amount_mist, 9)
                } else {
                    None
                },
                amount_mist,
                contract: (coin_type != SUI_COIN_TYPE).then(|| coin_type.to_string()),
                from,
                to,
            });
        }
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
        self.fetch_coins_page(owner, "0x2::sui::SUI", cursor).await
    }

    pub async fn fetch_coins_page(
        &self,
        owner: &str,
        coin_type: &str,
        cursor: Option<&str>,
    ) -> Result<SuiCoinPage, ApiError> {
        let page = self
            .call("suix_getCoins", json!([owner, coin_type, cursor, 50]))
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
        assert_eq!(incoming.amount_mist, "4770000000000");
        assert_eq!(incoming.from, THEM);
        let outgoing = entries.iter().find(|e| e.digest == "out").unwrap();
        assert!(!outgoing.is_incoming);
        assert_eq!(
            outgoing.amount_mist, "2500100000000000",
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
    fn gas_rebate_is_excluded_but_other_coin_transfers_are_kept() {
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
        let entries = sui_history_from_blocks(&[rebate_only], ME).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].contract.as_deref(), Some("0x6::cetus::CETUS"));
        assert_eq!(entries[0].amount_mist, "2699970876000000");
        assert!(!entries[0].is_incoming);
    }
}

// Validator-directory response data; staking owns the projection.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiSystemStateSummary {
    pub active_validators: Vec<SuiValidatorSummary>,
}
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SuiValidatorSummary {
    pub sui_address: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub project_url: String,
    pub commission_rate: String,          // basis points, "500" = 5%
    pub staking_pool_sui_balance: String, // MIST string
}

impl SuiClient {
    pub async fn fetch_staking_validators(&self) -> Result<SuiSystemStateSummary, ApiError> {
        let value = self
            .call("suix_getLatestSuiSystemState", serde_json::json!([]))
            .await?;
        serde_json::from_value(value).map_err(ApiError::from)
    }

    pub(crate) async fn validate_staking_system(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let (_, id, version) = chain.sui_staking_system()?;
        let value = self
            .call(
                "sui_getObject",
                json!([id,{"showOwner":true,"showType":true}]),
            )
            .await?;
        if value["data"]["owner"]["Shared"]["initial_shared_version"]
            .as_u64()
            .or_else(|| {
                value["data"]["owner"]["Shared"]["initial_shared_version"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
            })
            != Some(version)
            || value["data"]["type"].as_str() != Some("0x3::sui_system::SuiSystemState")
        {
            return Err(ApiError::invalid("Unexpected Sui staking system identity"));
        }
        Ok(())
    }

    pub(crate) async fn simulate_staking(&self, bytes: &[u8]) -> Result<(), ApiError> {
        use base64::Engine;
        let result = self
            .call(
                "sui_dryRunTransactionBlock",
                json!([base64::engine::general_purpose::STANDARD.encode(bytes)]),
            )
            .await?;
        if result["effects"]["status"]["status"].as_str() != Some("success") {
            return Err(ApiError::invalid(format!(
                "Sui staking simulation refused: {}",
                result["effects"]["status"]
            )));
        }
        Ok(())
    }

    pub(crate) async fn fetch_delegated_stakes(
        &self,
        owner: &str,
    ) -> Result<Vec<SuiDelegatedStake>, ApiError> {
        let result = self.call("suix_getStakes", json!([owner])).await?;
        let groups: Vec<SuiDelegatedStake> = serde_json::from_value(result)?;
        let mut ids = std::collections::HashSet::new();
        for group in &groups {
            for stake in &group.stakes {
                if !ids.insert(&stake.staked_sui_id)
                    || stake.principal.parse::<u64>().is_err()
                    || !matches!(stake.status.as_str(), "Active" | "Pending")
                    || stake
                        .estimated_reward
                        .as_ref()
                        .is_some_and(|r| r.parse::<u64>().is_err())
                {
                    return Err(ApiError::decode(
                        "Sui stake: malformed or duplicate position",
                    ));
                }
            }
        }
        Ok(groups)
    }

    pub(crate) async fn fetch_staked_object(
        &self,
        id: &str,
        owner: &str,
    ) -> Result<SuiStakedObject, ApiError> {
        let result = self
            .call(
                "sui_getObject",
                json!([id,{"showType":true,"showOwner":true,"showContent":true}]),
            )
            .await?;
        let data = &result["data"];
        let fields = &data["content"]["fields"];
        if data["owner"]["AddressOwner"].as_str() != Some(owner)
            || data["type"].as_str() != Some("0x3::staking_pool::StakedSui")
            || data["objectId"].as_str() != Some(id)
        {
            return Err(ApiError::invalid(
                "Sui stake object identity or owner differs from wallet",
            ));
        }
        let version = data["version"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_decode("Sui stake: missing version")?;
        let digest: [u8; 32] = bs58::decode(
            data["digest"]
                .as_str()
                .or_decode("Sui stake: missing digest")?,
        )
        .into_vec()
        .map_err(ApiError::decode)?
        .try_into()
        .map_err(|_| ApiError::decode("Sui stake: invalid digest"))?;
        let principal = fields["principal"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_decode("Sui stake: missing principal")?;
        let pool_id = fields["pool_id"]
            .as_str()
            .or_decode("Sui stake: missing pool")?
            .to_string();
        let activation_epoch = fields["stake_activation_epoch"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_decode("Sui stake: missing activation epoch")?;
        Ok(SuiStakedObject {
            version,
            digest,
            principal,
            pool_id,
            activation_epoch,
        })
    }

    pub(crate) async fn validate_owned_sui_coin(
        &self,
        id: &str,
        owner: &str,
        version: u64,
        digest: &[u8; 32],
        balance: u64,
    ) -> Result<(), ApiError> {
        let result = self
            .call(
                "sui_getObject",
                json!([id,{"showOwner":true,"showType":true,"showContent":true}]),
            )
            .await?;
        let data = &result["data"];
        if data["objectId"].as_str() != Some(id)
            || data["owner"]["AddressOwner"].as_str() != Some(owner)
            || data["type"].as_str() != Some("0x2::coin::Coin<0x2::sui::SUI>")
            || data["version"].as_str().and_then(|s| s.parse::<u64>().ok()) != Some(version)
            || data["digest"].as_str() != Some(bs58::encode(digest).into_string().as_str())
            || data["content"]["fields"]["balance"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                != Some(balance)
        {
            return Err(ApiError::invalid(
                "Reviewed Sui gas object changed or is not owned by this wallet",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SuiDelegatedStake {
    pub validator_address: String,
    pub staking_pool: String,
    pub stakes: Vec<SuiStake>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SuiStake {
    pub staked_sui_id: String,
    pub stake_active_epoch: String,
    pub principal: String,
    pub status: String,
    pub estimated_reward: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct SuiStakedObject {
    pub version: u64,
    pub digest: [u8; 32],
    pub principal: u64,
    pub pool_id: String,
    pub activation_epoch: u64,
}

#[cfg(test)]
mod network_identity_tests {
    use super::*;
    use crate::registry::Chain;
    use std::sync::Arc;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};

    #[tokio::test]
    async fn custom_nodes_require_the_selected_network_and_complete_genesis_checkpoint() {
        for chain in [Chain::Sui, Chain::SuiTestnet] {
            let (identifier, genesis) = chain.sui_network_identity().unwrap();
            for valid in [true, false] {
                let server = MockServer::start().await;
                Mock::given(body_partial_json(
                    json!({"method":"sui_getChainIdentifier"}),
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":identifier})),
                )
                .mount(&server)
                .await;
                Mock::given(body_partial_json(
                    json!({"method":"sui_getCheckpoint","params":["0"]}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"jsonrpc":"2.0","id":1,"result":{
                        "sequenceNumber":"0","digest":if valid {genesis} else {"wrong genesis"}
                    }}),
                ))
                .mount(&server)
                .await;
                let client = SuiClient::new(Arc::new(vec![server.uri()]));
                assert_eq!(client.verify_network(chain).await.is_ok(), valid);
            }
            let server = MockServer::start().await;
            let other = if chain == Chain::Sui {
                Chain::SuiTestnet
            } else {
                Chain::Sui
            };
            Mock::given(body_partial_json(
                json!({"method":"sui_getChainIdentifier"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"jsonrpc":"2.0","id":1,"result":other.sui_network_identity().unwrap().0}),
            ))
            .expect(1)
            .mount(&server)
            .await;
            let client = SuiClient::new(Arc::new(vec![server.uri()]));
            assert!(
                client
                    .verify_network(chain)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("wrong network")
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
}

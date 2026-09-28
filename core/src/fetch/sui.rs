//! Sui chain client.
//!
//! Uses the Sui JSON-RPC API (sui_getBalance, sui_getCoins,
//! sui_queryTransactionBlocks, unsafe_transferSui / sui_executeTransactionBlock).
//! Signing uses Ed25519 via ed25519-dalek.
//! Sui addresses are 32-byte Blake2b-256 hashes of the public key,
//! prefixed with a flag byte (0x00 for Ed25519).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::fetch::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiBalance {
    /// MIST (1 SUI = 1_000_000_000 MIST).
    pub mist: u64,
    pub sui_display: String,
}

/// One transaction's effect on an address's SUI, fee excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiHistoryEntry {
    pub digest: String,
    pub timestamp_ms: u64,
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

impl super::SignedSubmission for SuiSendResult {
    fn submission_id(&self) -> &str {
        &self.digest
    }
    fn signed_payload(&self) -> &str {
        &self.tx_bytes_b64
    }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat {
        super::SignedPayloadFormat::Base64
    }
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

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        crate::fetch::json_rpc::call(
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
    pub async fn fetch_balance(&self, address: &str) -> Result<SuiBalance, String> {
        let result = self
            .call("suix_getBalance", json!([address, "0x2::sui::SUI"]))
            .await?;
        let mist: u64 = result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or("suix_getBalance: missing totalBalance")?;
        Ok(SuiBalance {
            mist,
            sui_display: format_sui(mist),
        })
    }

    /// The address's SUI transfers, newest first.
    ///
    /// Queried as sender and as recipient, since neither filter alone sees
    /// both directions, and read from each transaction's balance changes.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<SuiHistoryEntry>, String> {
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
        Ok(sui_history_from_blocks(&blocks, address))
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
    /// `suix_getAllBalances` returns coin types and totals but no decimals, so
    /// each type's metadata is read concurrently; a type whose metadata is
    /// missing is reported unnamed rather than dropped.
    pub async fn fetch_all_coin_balances(
        &self,
        address: &str,
    ) -> Result<Vec<super::HeldToken>, String> {
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

        let metadata = futures::future::join_all(
            held.iter()
                .map(|(coin_type, _)| self.fetch_coin_decimals(coin_type)),
        )
        .await;
        Ok(held
            .into_iter()
            .zip(metadata)
            .map(|((contract, balance_raw), decimals)| super::HeldToken {
                contract,
                balance_raw,
                decimals,
                symbol: None,
            })
            .collect())
    }

    /// Fetch the balance for a specific coin type (e.g. `0x5d4b...::coin::COIN`).
    /// Returns the raw balance in the coin's smallest unit.
    pub async fn fetch_coin_balance(&self, address: &str, coin_type: &str) -> Result<u64, String> {
        let result = self
            .call("suix_getBalance", json!([address, coin_type]))
            .await?;
        result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("suix_getBalance: missing totalBalance for {coin_type}"))
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
fn sui_history_from_blocks(blocks: &[Value], address: &str) -> Vec<SuiHistoryEntry> {
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
        entries.push(SuiHistoryEntry {
            digest: digest.to_string(),
            timestamp_ms: block
                .get("timestampMs")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            is_incoming,
            amount_mist,
            from,
            to,
        });
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
    entries
}

fn format_sui(mist: u64) -> String {
    let whole = mist / 1_000_000_000;
    let frac = mist % 1_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:09}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 {
        &trimmed[..6]
    } else {
        trimmed
    };
    format!("{}.{}", whole, capped)
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
        let entries = sui_history_from_blocks(&[received, sent.clone(), sent], ME);
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
        assert!(sui_history_from_blocks(&[rebate_only], ME).is_empty());
    }
}

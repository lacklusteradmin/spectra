//! The Blockscout adapter: the Etherscan-compatible `module=account` API that
//! Blockscout and Routescan serve. EVM nodes cannot index by address, so EVM
//! history and the list of tokens an address holds come from here.

use crate::api::error::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile};
use crate::registry::EvmHistorySource;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmHistoryEntry {
    pub status: String,
    pub txid: String,
    pub block_number: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    /// Value in wei (string to avoid u128 overflow in JSON).
    pub value_wei: String,
    pub fee_wei: String,
    pub is_incoming: bool,
}

/// One ERC-20 token transfer returned by Etherscan `tokentx`. The amount is
/// the raw integer, and the token is only its contract: the tracked token's
/// own decimals scale it and name it, never the explorer's `tokenDecimal`,
/// which an explorer may leave empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmTokenTransferEntry {
    pub contract: String,
    pub from: String,
    pub to: String,
    /// Raw integer amount (base units), as string.
    pub amount_raw: String,
    pub txid: String,
    pub block_number: u64,
    pub log_index: u32,
    pub timestamp: u64,
}

/// One ERC-721 or ERC-1155 transfer, from `tokennfttx` or `token1155tx`: a
/// token id and a whole quantity, never a decimal amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmNftTransferEntry {
    pub standard: crate::api::evm_nft::NftStandard,
    pub contract: String,
    pub token_id: String,
    pub quantity: String,
    pub symbol: String,
    pub collection: String,
    pub from: String,
    pub to: String,
    pub txid: String,
    pub block_number: u64,
    pub timestamp: u64,
}

/// One NFT an address holds, as an explorer's inventory lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmNftHolding {
    pub standard: crate::api::evm_nft::NftStandard,
    pub contract: String,
    pub token_id: String,
    pub quantity: String,
    pub collection: String,
    pub symbol: String,
    /// The token's own name from its metadata, when it has one.
    pub name: Option<String>,
}

/// Routescan serves the Etherscan wire contract under `/etherscan`.
fn is_routescan(base: &str) -> bool {
    base.trim_end_matches('/').ends_with("/etherscan")
}

/// Whether the explorer at `base` lists the NFTs an address holds: a
/// Blockscout instance does, through its REST v2 API; Routescan's Etherscan
/// interface has transfer lists but no inventory.
pub fn serves_nft_inventory(base: &str) -> bool {
    !is_routescan(base)
}

/// Build a query for a configured keyless explorer, refusing unavailable history.
pub fn explorer_query_url(source: EvmHistorySource<'_>, params: &str) -> Result<String, ApiError> {
    match source {
        EvmHistorySource::Open(base) => {
            let base = base.trim_end_matches('/');
            let suffix = if is_routescan(base) || base.ends_with("/api") {
                ""
            } else {
                "/api"
            };
            Ok(format!("{base}{suffix}?{params}"))
        }
        EvmHistorySource::Unavailable => Err(ApiError::InvalidInput(
            "no explorer serves this chain's transaction history".into(),
        )),
    }
}

/// Distinguish empty history from explorer refusal by the shape of `result`.
/// Both can have status "0": an empty array is empty history, while a
/// non-array result is an error. Do not rely on explorer message wording.
fn etherscan_result_rows(
    status: &str,
    message: &str,
    result: Value,
) -> Result<Vec<Value>, ApiError> {
    match result {
        Value::Array(rows) => Ok(rows),
        _ if status == "1" => Err(ApiError::Decode(format!(
            "explorer returned a non-list result: {message}"
        ))),
        _ => Err(ApiError::Rejected(format!("explorer refused: {message}"))),
    }
}

/// `keccak256("Approval(address,address,uint256)")`, the topic an ERC-20
/// approval (and an ERC-721 one, which indexes a fourth topic) is logged
/// under.
pub(crate) const APPROVAL_TOPIC: &str =
    "0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925";

/// The most rows one `getLogs` answer holds; a full page means there may be
/// more from its last block on.
const LOG_PAGE: usize = 1000;
/// How many pages one scan reads before saying it stopped short.
const LOG_PAGES: usize = 20;

/// An ERC-20 approval an owner gave: the token and the spender it named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalLog {
    pub token: String,
    pub spender: String,
}

/// The approvals an owner has logged, oldest first, and whether the scan
/// reached the end of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalLogs {
    pub logs: Vec<ApprovalLog>,
    pub complete: bool,
}

pub struct BlockscoutClient {
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl Default for BlockscoutClient {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockscoutClient {
    pub fn new() -> Self {
        Self {
            client: HttpClient::shared(),
        }
    }

    /// Every ERC-20 `Approval` log `owner` emitted, through the explorer's
    /// Etherscan-compatible `getLogs`, paged by block. An ERC-721 approval,
    /// which indexes its token id as a fourth topic, is not one.
    pub async fn fetch_approval_logs(
        &self,
        owner: &str,
        source: EvmHistorySource<'_>,
    ) -> Result<ApprovalLogs, ApiError> {
        #[derive(Deserialize)]
        struct ApiResp {
            status: String,
            #[serde(default)]
            message: String,
            result: Value,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Log {
            address: String,
            topics: Vec<Option<String>>,
            block_number: String,
            transaction_hash: String,
            log_index: String,
        }
        let owner_topic = format!(
            "0x{:0>64}",
            owner.trim_start_matches("0x").to_ascii_lowercase()
        );
        let mut logs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut from_block = 0u64;
        for _ in 0..LOG_PAGES {
            let url = explorer_query_url(
                source,
                &format!(
                    "module=logs&action=getLogs&fromBlock={from_block}&toBlock=latest\
                     &topic0={APPROVAL_TOPIC}&topic1={owner_topic}&topic0_1_opr=and"
                ),
            )?;
            let response: ApiResp = self.client.get_json(&url, RetryProfile::ChainRead).await?;
            let rows = etherscan_result_rows(&response.status, &response.message, response.result)?;
            let full = rows.len() >= LOG_PAGE;
            let mut last_block = from_block;
            for row in rows {
                let log: Log = serde_json::from_value(row)
                    .map_err(|e| ApiError::Decode(format!("approval log: {e}")))?;
                last_block = crate::api::evm_json_rpc::parse_hex_u64(&log.block_number)?;
                if !seen.insert((log.transaction_hash.clone(), log.log_index.clone())) {
                    continue;
                }
                let topic = |index: usize| log.topics.get(index).cloned().flatten();
                if topic(0).as_deref() != Some(APPROVAL_TOPIC)
                    || topic(1).as_deref() != Some(owner_topic.as_str())
                    || topic(3).is_some()
                {
                    continue;
                }
                let Some(spender) = topic(2).filter(|t| t.len() == 66) else {
                    continue;
                };
                logs.push(ApprovalLog {
                    token: log.address.to_ascii_lowercase(),
                    spender: format!("0x{}", &spender[26..]).to_ascii_lowercase(),
                });
            }
            if !full {
                return Ok(ApprovalLogs {
                    logs,
                    complete: true,
                });
            }
            // A full page may cut its last block short: read from that block
            // again, and the seen set drops what repeats.
            from_block = last_block;
        }
        Ok(ApprovalLogs {
            logs,
            complete: false,
        })
    }

    pub async fn fetch_history(
        &self,
        address: &str,
        source: EvmHistorySource<'_>,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<EvmHistoryEntry>, ApiError> {
        let addr_lower = address.to_lowercase();
        let page = page.max(1);
        let page_size = page_size.clamp(1, 500);
        let url = explorer_query_url(
            source,
            &format!(
                "module=account&action=txlist&address={addr_lower}&sort=desc&page={page}&offset={page_size}"
            ),
        )?;

        #[derive(Deserialize)]
        struct ApiResp {
            status: String,
            #[serde(default)]
            message: String,
            result: Value,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TxItem {
            #[serde(default)]
            is_error: Option<String>,
            #[serde(default, rename = "txreceipt_status")]
            receipt_status: Option<String>,
            hash: String,
            block_number: String,
            time_stamp: String,
            from: String,
            to: String,
            value: String,
            gas_price: String,
            gas_used: String,
        }

        let resp: ApiResp = self.client.get_json(&url, RetryProfile::ChainRead).await?;
        let rows = etherscan_result_rows(&resp.status, &resp.message, resp.result)?;
        let items: Vec<TxItem> = serde_json::from_value(Value::Array(rows))
            .map_err(|e| ApiError::Decode(format!("history parse: {e}")))?;

        let addr_norm = address.to_lowercase();

        items
            .into_iter()
            .map(|tx| {
                let status = match (tx.is_error.as_deref(), tx.receipt_status.as_deref()) {
                    (Some("1"), _) | (_, Some("0")) => "failed",
                    (Some("0"), _) | (_, Some("1")) => "confirmed",
                    _ => return Err(ApiError::Decode("history missing execution status".into())),
                };
                let fee_wei = tx
                    .gas_price
                    .parse::<u128>()
                    .unwrap_or(0)
                    .saturating_mul(tx.gas_used.parse::<u128>().unwrap_or(0))
                    .to_string();
                // The explorer lists only mined transactions.
                let timestamp =
                    crate::api::time::confirmed_history_time(tx.time_stamp.parse().ok(), &tx.hash)?;
                Ok(EvmHistoryEntry {
                    status: status.into(),
                    txid: tx.hash,
                    block_number: tx.block_number.parse().unwrap_or(0),
                    timestamp,
                    from: tx.from.clone(),
                    to: tx.to.clone(),
                    value_wei: tx.value,
                    fee_wei,
                    is_incoming: tx.to.to_lowercase() == addr_norm,
                })
            })
            .collect()
    }

    /// Every ERC-20 token `address` holds, with the explorer's balance and the
    /// contract's decimals.
    ///
    /// The two explorers spell this differently. Blockscout answers
    /// `tokenlist` with the whole list at once; Routescan does not serve that
    /// action and pages `addresstokenbalance` instead. A list cut short would
    /// read every token past the cut as a zero balance, so a holder with more
    /// pages than this will read is refused rather than truncated.
    pub async fn fetch_token_holdings(
        &self,
        address: &str,
        base: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        const PAGE_SIZE: usize = 1000;
        const MAX_PAGES: usize = 10;
        let address = address.to_lowercase();
        let source = EvmHistorySource::Open(base);
        if !is_routescan(base) {
            let url = explorer_query_url(
                source,
                &format!("module=account&action=tokenlist&address={address}"),
            )?;
            return parse_token_list(self.client.get_json(&url, RetryProfile::ChainRead).await?);
        }
        let mut held = Vec::new();
        for page in 1..=MAX_PAGES {
            let url = explorer_query_url(
                source,
                &format!(
                    "module=account&action=addresstokenbalance&address={address}&page={page}&offset={PAGE_SIZE}"
                ),
            )?;
            let rows =
                parse_token_list(self.client.get_json(&url, RetryProfile::ChainRead).await?)?;
            let last = rows.len() < PAGE_SIZE;
            held.extend(rows);
            if last {
                return Ok(held);
            }
        }
        Err(ApiError::Rejected(format!(
            "holds more than {} tokens; the list cannot be read whole",
            PAGE_SIZE * MAX_PAGES
        )))
    }

    /// One page of `address`'s ERC-20 transfers via Etherscan `tokentx`, and
    /// whether the page was full, so another may follow.
    pub async fn fetch_token_transfers(
        &self,
        address: &str,
        source: EvmHistorySource<'_>,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<EvmTokenTransferEntry>, bool), ApiError> {
        let addr_lower = address.to_lowercase();
        let safe_page = page.max(1);
        let safe_size = page_size.clamp(1, 500);
        let url = explorer_query_url(
            source,
            &format!(
                "module=account&action=tokentx&address={addr_lower}&page={safe_page}&offset={safe_size}&sort=desc"
            ),
        )?;

        #[derive(Deserialize)]
        struct ApiResp {
            status: String,
            #[serde(default)]
            message: String,
            result: serde_json::Value,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TxItem {
            block_number: String,
            time_stamp: String,
            hash: String,
            from: String,
            to: String,
            contract_address: String,
            value: String,
            #[serde(default)]
            log_index: String,
            /// Present on an NFT's transfer, which is no fungible amount.
            #[serde(default, rename = "tokenID")]
            token_id: Option<String>,
        }

        let resp: ApiResp = self.client.get_json(&url, RetryProfile::ChainRead).await?;
        let rows = etherscan_result_rows(&resp.status, &resp.message, resp.result)?;
        let full = rows.len() >= safe_size as usize;

        let items: Vec<TxItem> = serde_json::from_value(serde_json::Value::Array(rows))
            .map_err(|e| ApiError::Decode(format!("token transfer parse: {e}")))?;

        let entries = items
            .into_iter()
            // A transfer of a token id moves no fungible amount.
            .filter(|tx| tx.token_id.as_deref().is_none_or(str::is_empty))
            .map(|tx| {
                if tx.value.is_empty() || !tx.value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ApiError::Decode(format!(
                        "token transfer {}: malformed value",
                        tx.hash
                    )));
                }
                let timestamp =
                    crate::api::time::confirmed_history_time(tx.time_stamp.parse().ok(), &tx.hash)?;
                Ok(EvmTokenTransferEntry {
                    contract: tx.contract_address.to_lowercase(),
                    from: tx.from.to_lowercase(),
                    to: tx.to.to_lowercase(),
                    amount_raw: tx.value,
                    txid: tx.hash,
                    block_number: tx.block_number.parse().unwrap_or(0),
                    log_index: tx.log_index.parse().unwrap_or(0),
                    timestamp,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        Ok((entries, full))
    }
}

impl BlockscoutClient {
    /// One page of `address`'s ERC-721 (`tokennfttx`) or ERC-1155
    /// (`token1155tx`) transfers, newest first, and whether the page was
    /// full, so another may follow.
    pub async fn fetch_nft_transfers(
        &self,
        address: &str,
        source: EvmHistorySource<'_>,
        standard: crate::api::evm_nft::NftStandard,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<EvmNftTransferEntry>, bool), ApiError> {
        use crate::api::evm_nft::{NftStandard, canonical_uint256};
        #[derive(Deserialize)]
        struct ApiResp {
            status: String,
            #[serde(default)]
            message: String,
            result: Value,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TxItem {
            block_number: String,
            time_stamp: String,
            hash: String,
            from: String,
            to: String,
            contract_address: String,
            // Blockscout writes `null` for a collection with no name.
            #[serde(default)]
            token_name: Option<String>,
            #[serde(default)]
            token_symbol: Option<String>,
            #[serde(rename = "tokenID")]
            token_id: String,
            #[serde(default)]
            token_value: Option<String>,
        }
        let page_size = page_size.clamp(1, 500);
        let action = match standard {
            NftStandard::Erc721 => "tokennfttx",
            NftStandard::Erc1155 => "token1155tx",
        };
        let url = explorer_query_url(
            source,
            &format!(
                "module=account&action={action}&address={}&page={}&offset={page_size}&sort=desc",
                address.to_lowercase(),
                page.max(1),
            ),
        )?;
        let resp: ApiResp = self.client.get_json(&url, RetryProfile::ChainRead).await?;
        let rows = etherscan_result_rows(&resp.status, &resp.message, resp.result)?;
        let full = rows.len() >= page_size as usize;
        let items: Vec<TxItem> = serde_json::from_value(Value::Array(rows))
            .map_err(|e| ApiError::Decode(format!("NFT transfer parse: {e}")))?;
        let entries = items
            .into_iter()
            .map(|tx| {
                let token_id = canonical_uint256(&tx.token_id)
                    .ok_or_else(|| ApiError::decode("NFT transfer: invalid token id"))?;
                // An ERC-721 transfer moves the one token; an ERC-1155
                // transfer states its whole-number quantity.
                let quantity = match standard {
                    NftStandard::Erc721 => "1".to_string(),
                    NftStandard::Erc1155 => {
                        tx.token_value
                            .as_deref()
                            .and_then(canonical_uint256)
                            .ok_or_else(|| ApiError::decode("NFT transfer: invalid quantity"))?
                    }
                };
                Ok(EvmNftTransferEntry {
                    standard,
                    contract: tx.contract_address.to_lowercase(),
                    token_id,
                    quantity,
                    symbol: tx.token_symbol.unwrap_or_default(),
                    collection: tx.token_name.unwrap_or_default(),
                    from: tx.from.to_lowercase(),
                    to: tx.to.to_lowercase(),
                    block_number: tx.block_number.parse().unwrap_or(0),
                    timestamp: crate::api::time::confirmed_history_time(
                        tx.time_stamp.parse().ok(),
                        &tx.hash,
                    )?,
                    txid: tx.hash,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        Ok((entries, full))
    }

    /// Every ERC-721 and ERC-1155 token `address` holds, from a Blockscout
    /// instance's inventory, and whether the list is complete. Only an
    /// explorer [`serves_nft_inventory`] is asked.
    pub async fn fetch_nft_inventory(
        &self,
        address: &str,
        base: &str,
    ) -> Result<(Vec<EvmNftHolding>, bool), ApiError> {
        if !serves_nft_inventory(base) {
            return Err(ApiError::invalid("This explorer keeps no NFT inventory"));
        }
        let root = base.trim_end_matches('/').trim_end_matches("/api");
        let address = address.to_lowercase();
        let mut holdings = Vec::new();
        let mut complete = true;
        let mut cursor: Option<Value> = None;
        // Fifty a page; twenty pages are a thousand tokens, and a list past
        // that is reported as not complete rather than read as all there is.
        for _ in 0..20 {
            let mut url = format!("{root}/api/v2/addresses/{address}/nft?type=ERC-721%2CERC-1155");
            if let Some(next) = cursor.as_ref().and_then(Value::as_object) {
                for (key, value) in next {
                    let value = match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    url.push_str(&format!(
                        "&{key}={}",
                        crate::api::history_page::query_value(&value)
                    ));
                }
            }
            let page: Value = self.client.get_json(&url, RetryProfile::ChainRead).await?;
            let (read, whole) = parse_nft_inventory(&page)?;
            holdings.extend(read);
            complete &= whole;
            match page.get("next_page_params").filter(|next| !next.is_null()) {
                Some(next) => cursor = Some(next.clone()),
                None => return Ok((holdings, complete)),
            }
        }
        Ok((holdings, false))
    }
}

/// One page of a Blockscout `/api/v2/addresses/{address}/nft` answer, and
/// whether every row on it was read. A row of another kind (ERC-404, which
/// a type filter can let through) is not an NFT here, nor is a quantity of
/// zero; a row whose contract, id or quantity does not read is left out and
/// the page reported short, rather than one indexer row hiding the others.
fn parse_nft_inventory(page: &Value) -> Result<(Vec<EvmNftHolding>, bool), ApiError> {
    use crate::api::evm_nft::{NftStandard, canonical_uint256};
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::decode("NFT inventory: missing items"))?;
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
    let mut whole = true;
    let mut holdings = Vec::new();
    for item in items {
        let Some(standard) = item
            .get("token_type")
            .and_then(Value::as_str)
            .and_then(NftStandard::parse)
        else {
            continue;
        };
        let token = item.get("token").unwrap_or(&Value::Null);
        let contract =
            text(token.get("address_hash").or_else(|| token.get("address"))).filter(|contract| {
                contract.len() == 42
                    && contract.starts_with("0x")
                    && contract[2..].bytes().all(|b| b.is_ascii_hexdigit())
            });
        let token_id = text(item.get("id")).as_deref().and_then(canonical_uint256);
        let quantity = match standard {
            NftStandard::Erc721 => Some("1".to_string()),
            NftStandard::Erc1155 => text(item.get("value"))
                .as_deref()
                .and_then(canonical_uint256),
        };
        let (Some(contract), Some(token_id), Some(quantity)) = (contract, token_id, quantity)
        else {
            whole = false;
            continue;
        };
        // A row for an ERC-1155 id the address no longer holds.
        if quantity == "0" {
            continue;
        }
        holdings.push(EvmNftHolding {
            standard,
            contract: contract.to_lowercase(),
            token_id,
            quantity,
            collection: text(token.get("name")).unwrap_or_default(),
            symbol: text(token.get("symbol")).unwrap_or_default(),
            name: text(item.pointer("/metadata/name"))
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty()),
        });
    }
    Ok((holdings, whole))
}

/// The fungible holdings in a `tokenlist` (Blockscout) or
/// `addresstokenbalance` (Routescan) answer. NFTs are not balances; a row
/// whose balance does not parse is somebody's spam, not a reason to drop the
/// whole list.
fn parse_token_list(response: Value) -> Result<Vec<crate::api::HeldToken>, ApiError> {
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let message = response
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let result = response.get("result").cloned().unwrap_or(Value::Null);
    let field = |row: &Value, names: &[&str]| {
        names
            .iter()
            .find_map(|name| row.get(*name).and_then(Value::as_str))
            .map(str::to_owned)
    };
    Ok(etherscan_result_rows(&status, &message, result)?
        .iter()
        .filter(|row| field(row, &["type"]).is_none_or(|kind| kind == "ERC-20"))
        .filter_map(|row| {
            let contract = field(row, &["contractAddress", "TokenAddress"])?.to_lowercase();
            let balance_raw = field(row, &["balance", "TokenQuantity"])?.parse().ok()?;
            let decimals = field(row, &["decimals", "TokenDivisor"])
                .and_then(|d| d.parse::<u128>().ok())
                .and_then(|d| crate::api::checked_token_decimals(d).ok());
            Some(crate::api::HeldToken {
                contract,
                balance_raw,
                decimals,
            })
        })
        .collect())
}

#[cfg(test)]
mod a_token_list_reads_both_dialects {
    use super::parse_token_list;
    use serde_json::json;

    /// Shapes captured from `eth.blockscout.com` and Routescan.
    #[test]
    fn fungible_rows_are_kept_and_the_rest_are_not() {
        let blockscout = parse_token_list(json!({"message":"OK","status":"1","result":[
            {"balance":"2000000","contractAddress":"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
             "decimals":"6","name":"USD Coin","symbol":"USDC","type":"ERC-20"},
            {"balance":"1","contractAddress":"0x0000000000696760e15f265e828db644a0c242eb",
             "decimals":"","name":"Wei Name Service","symbol":"WEI","type":"ERC-721"},
            {"balance":"not a number","contractAddress":"0x1111111111111111111111111111111111111111",
             "decimals":"18","name":"Spam","symbol":"SPAM","type":"ERC-20"}
        ]}))
        .expect("a list");
        assert_eq!(blockscout.len(), 1);
        assert_eq!(
            blockscout[0].contract,
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
        );
        assert_eq!(blockscout[0].balance_raw, 2_000_000);
        assert_eq!(blockscout[0].decimals, Some(6));

        let routescan = parse_token_list(json!({"status":"1","message":"OK","result":[
            {"TokenAddress":"0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7","TokenName":"TetherToken",
             "TokenSymbol":"USDt","TokenQuantity":"2002129","TokenDivisor":"6"}
        ]}))
        .expect("a list");
        assert_eq!(routescan.len(), 1);
        assert_eq!(routescan[0].balance_raw, 2_002_129);
        assert_eq!(routescan[0].decimals, Some(6));
    }

    /// An address holding nothing is a list; an explorer refusing is not.
    #[test]
    fn an_empty_list_is_data_and_a_refusal_is_an_error() {
        assert!(
            parse_token_list(json!({"status":"0","message":"No tokens found","result":[]}))
                .expect("empty is data")
                .is_empty()
        );
        assert!(
            parse_token_list(json!({"status":"0","message":"NOTOK",
                "result":"Error! Missing Or invalid Action name"}))
            .is_err()
        );
    }
}

#[cfg(test)]
mod a_refusal_is_not_an_empty_history {
    use super::etherscan_result_rows;
    use serde_json::json;

    /// The payloads are the ones these explorers actually return — captured
    /// from `api.etherscan.io` and `base.blockscout.com` rather than guessed.
    ///
    /// All three answer `status: "0"`; each must surface as an error, not as
    /// an empty history.
    #[test]
    fn the_three_shapes_that_all_say_status_zero() {
        // Etherscan V2 with no key. The V2 API has no keyless tier at all, so
        // this is what every EVM history call returned once the key was blank.
        assert!(etherscan_result_rows("0", "NOTOK", json!("Missing/Invalid API Key")).is_err());

        // Blockscout, intermittently — one call in three during this survey.
        assert!(etherscan_result_rows("0", "Something went wrong.", json!(null)).is_err());

        // An address that genuinely has no transactions. The only one of the
        // three that is data.
        assert_eq!(
            etherscan_result_rows("0", "No transactions found", json!([])).expect("empty is data"),
            Vec::<serde_json::Value>::new()
        );
    }

    /// Judged on the shape, so it does not depend on an explorer's wording.
    #[test]
    fn a_populated_result_passes_whatever_the_message_says() {
        let rows = etherscan_result_rows("1", "OK", json!([{"hash": "0x1"}])).expect("rows");
        assert_eq!(rows.len(), 1);
    }

    /// `status: "1"` with a non-list result is malformed, not empty.
    #[test]
    fn success_with_the_wrong_shape_is_still_a_failure() {
        assert!(etherscan_result_rows("1", "OK", json!("not a list")).is_err());
    }
}

#[cfg(test)]
mod every_evm_chain_says_where_its_history_comes_from {
    use super::explorer_query_url;
    use crate::registry::{Chain, EvmHistorySource};

    /// A keyless source carries the chain in its base, so no `chainid` and no
    /// key go on the query.
    #[test]
    fn an_open_source_sends_neither_a_chainid_nor_a_key() {
        let url = explorer_query_url(
            EvmHistorySource::Open("https://eth.blockscout.com"),
            "module=account&action=txlist&address=0xabc",
        )
        .expect("open sources never refuse");
        assert_eq!(
            url,
            "https://eth.blockscout.com/api?module=account&action=txlist&address=0xabc"
        );
    }

    #[test]
    fn unavailable_history_is_refused_before_network_access() {
        for chain in [
            Chain::BnbChain,
            Chain::Sonic,
            Chain::OpBnb,
            Chain::Sei,
            Chain::Linea,
            Chain::Hyperliquid,
            Chain::Cronos,
            Chain::XLayer,
            Chain::Berachain,
        ] {
            assert_eq!(chain.evm_history_source(), EvmHistorySource::Unavailable);
            assert!(explorer_query_url(chain.evm_history_source(), "module=account").is_err());
        }
    }

    #[test]
    fn history_sources_match_the_concrete_network_directory() {
        let catalog = crate::endpoints::catalog();
        for chain in Chain::all().filter(|chain| chain.is_evm()) {
            if let EvmHistorySource::Open(url) = chain.evm_history_source() {
                assert!(catalog.records.iter().any(|record| {
                    record.chain_id == chain
                        && record.endpoint == url
                        && record
                            .capabilities
                            .contains(&crate::EndpointCapability::History)
                }));
            }
            // A testnet reads its own explorer or none, never its mainnet's.
            let mainnet = chain.mainnet_counterpart();
            if chain != mainnet && matches!(chain.evm_history_source(), EvmHistorySource::Open(_)) {
                assert_ne!(chain.evm_history_source(), mainnet.evm_history_source());
            }
        }
    }

    #[test]
    fn explicit_history_api_paths_are_not_extended() {
        for base in ["https://example.org/api", "https://example.org/etherscan/"] {
            assert_eq!(
                explorer_query_url(EvmHistorySource::Open(base), "action=txlist").unwrap(),
                format!("{}?action=txlist", base.trim_end_matches('/'))
            );
        }
    }
}

#[cfg(test)]
mod history_page_tests {
    use super::*;
    use serde_json::json;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

    #[tokio::test]
    async fn native_history_obeys_page_and_size_and_propagates_errors() {
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(|request: &Request| {
            let query: std::collections::HashMap<_,_> = request.url.query_pairs().into_owned().collect();
            let page = &query["page"];
            if page == "3" {
                return ResponseTemplate::new(200).set_body_json(json!({"status":"0","message":"NOTOK","result":"rate limited"}));
            }
            ResponseTemplate::new(200).set_body_json(json!({"status":"1","message":"OK","result":[{
                "hash":format!("page-{page}"), "blockNumber":"123", "timeStamp":"456", "from":"from", "to":"to",
                "value":"1", "gasPrice":"1", "gasUsed":"21000", "isError":"0"
            }]}))
        }).mount(&server).await;
        let source = EvmHistorySource::Open(Box::leak(server.uri().into_boxed_str()));
        let client = BlockscoutClient::new();
        for page in [1, 2] {
            let rows = client.fetch_history("FROM", source, page, 7).await.unwrap();
            assert_eq!(rows[0].txid, format!("page-{page}"));
        }
        assert!(client.fetch_history("from", source, 3, 7).await.is_err());
        for request in server.received_requests().await.unwrap() {
            let query: std::collections::HashMap<_, _> =
                request.url.query_pairs().into_owned().collect();
            assert_eq!(query["offset"], "7");
            assert_eq!(query["action"], "txlist");
        }
    }
}

#[cfg(test)]
mod execution_history_regressions {
    use super::*;
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
    #[tokio::test]
    async fn history_preserves_reverts_and_refuses_unknown_execution_status() {
        for (flags, expected) in [
            (
                json!({"isError":"1", "txreceipt_status":"0"}),
                Some("failed"),
            ),
            (json!({"isError":"0"}), Some("confirmed")),
            (json!({}), None),
        ] {
            let server = MockServer::start().await;
            let mut row = json!({"hash":"tx","blockNumber":"1","timeStamp":"1700000000","from":"from","to":"to","value":"100","gasPrice":"1","gasUsed":"1"});
            row.as_object_mut()
                .unwrap()
                .extend(flags.as_object().unwrap().clone());
            Mock::given(any())
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"status":"1","message":"OK","result":[row]})),
                )
                .mount(&server)
                .await;
            let source = EvmHistorySource::Open(Box::leak(server.uri().into_boxed_str()));
            let result = BlockscoutClient::new()
                .fetch_history("from", source, 1, 20)
                .await;
            match expected {
                Some(status) => assert_eq!(result.unwrap()[0].status, status),
                None => assert!(result.is_err()),
            }
        }
    }
}

#[cfg(test)]
mod nft_answers {
    use super::*;
    use crate::api::evm_nft::NftStandard;
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::query_param};

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/blockscout-nft.json")).unwrap()
    }

    /// The inventory page Blockscout answered: both standards, collection
    /// names it could not read as `null`, metadata names where tokens have
    /// them, contracts checksummed.
    #[test]
    fn an_inventory_page_reads_as_tokens_not_balances() {
        let (holdings, whole) = parse_nft_inventory(&fixture()["inventory"]).unwrap();
        assert!(whole);
        assert_eq!(holdings.len(), 3);
        assert_eq!(
            holdings[0],
            EvmNftHolding {
                standard: NftStandard::Erc721,
                contract: "0x1338f0bce9fc48373bfa96f0d4d558497b2ea230".into(),
                token_id: "0".into(),
                quantity: "1".into(),
                collection: "QA-Test-NFT".into(),
                symbol: "QANFT".into(),
                name: None,
            }
        );
        assert_eq!(holdings[1].standard, NftStandard::Erc1155);
        assert_eq!(
            (holdings[1].token_id.as_str(), holdings[1].quantity.as_str()),
            ("2", "1")
        );
        assert_eq!(holdings[1].collection, "");
        assert!(holdings[1].name.is_some());

        // Rows that do not read are left out and the page reported short; a
        // row of another standard, or an emptied ERC-1155 id, is no NFT.
        let mut page = fixture()["inventory"].clone();
        let items = page["items"].as_array_mut().unwrap();
        items[0]["id"] = json!("1.5");
        items[1]["value"] = json!("0");
        items[2]["token_type"] = json!("ERC-404");
        let (holdings, whole) = parse_nft_inventory(&page).unwrap();
        assert!(holdings.is_empty() && !whole);
        // Leaving those two out is no gap in the list.
        let mut page = fixture()["inventory"].clone();
        page["items"][1]["value"] = json!("0");
        page["items"][2]["token_type"] = json!("ERC-404");
        let (holdings, whole) = parse_nft_inventory(&page).unwrap();
        assert!(whole);
        assert_eq!(holdings.len(), 1);
        assert!(parse_nft_inventory(&json!({"message": "Not found"})).is_err());
    }

    /// The inventory follows `next_page_params` to its end, and Routescan,
    /// which keeps none, is never asked for one.
    #[tokio::test]
    async fn the_inventory_is_read_to_its_end() {
        let server = MockServer::start().await;
        let next = fixture()["next_page_params"].clone();
        let mut first = fixture()["inventory"].clone();
        first["next_page_params"] = next.clone();
        Mock::given(query_param("token_id", "1"))
            .and(query_param(
                "token_contract_address_hash",
                next["token_contract_address_hash"].as_str().unwrap(),
            ))
            .and(query_param("items_count", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"items": [fixture()["inventory"]["items"][0]], "next_page_params": null}),
            ))
            .mount(&server)
            .await;
        Mock::given(query_param("type", "ERC-721,ERC-1155"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first))
            .mount(&server)
            .await;
        let client = BlockscoutClient::new();
        let (holdings, complete) = client
            .fetch_nft_inventory("0x000000000000000000000000000000000000dEaD", &server.uri())
            .await
            .unwrap();
        assert!(complete);
        assert_eq!(holdings.len(), 4);
        let paths: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| request.url.path().to_string())
            .collect();
        assert_eq!(
            paths,
            vec!["/api/v2/addresses/0x000000000000000000000000000000000000dead/nft"; 2]
        );
        let routescan = "https://api.routescan.io/v2/network/mainnet/evm/43114/etherscan";
        assert!(!serves_nft_inventory(routescan));
        assert!(
            client
                .fetch_nft_inventory("0xabc", routescan)
                .await
                .is_err()
        );
    }

    /// Both transfer lists, as Blockscout and Routescan answer them: token
    /// ids and whole quantities, a collection without a name read as empty.
    #[tokio::test]
    async fn transfers_carry_ids_and_quantities() {
        for (answer, standard, expected) in [
            (
                fixture()["erc721_transfers"].clone(),
                NftStandard::Erc721,
                vec![("0", "1", "QA-Test-NFT")],
            ),
            (
                fixture()["erc1155_transfers"].clone(),
                NftStandard::Erc1155,
                vec![("1", "1", ""), ("2", "1", "")],
            ),
            (
                fixture()["routescan_erc1155_transfers"].clone(),
                NftStandard::Erc1155,
                vec![("1780468422553", "1", "Frqtal FNFT"); 2],
            ),
        ] {
            let server = MockServer::start().await;
            let action = match standard {
                NftStandard::Erc721 => "tokennfttx",
                NftStandard::Erc1155 => "token1155tx",
            };
            Mock::given(query_param("action", action))
                .respond_with(ResponseTemplate::new(200).set_body_json(answer))
                .mount(&server)
                .await;
            let (entries, full) = BlockscoutClient::new()
                .fetch_nft_transfers(
                    "0x000000000000000000000000000000000000dead",
                    EvmHistorySource::Open(&server.uri()),
                    standard,
                    1,
                    2,
                )
                .await
                .unwrap();
            assert_eq!(full, expected.len() == 2);
            assert_eq!(
                entries
                    .iter()
                    .map(|e| (
                        e.token_id.as_str(),
                        e.quantity.as_str(),
                        e.collection.as_str()
                    ))
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(entries.iter().all(|e| e.standard == standard
                && e.to == "0x000000000000000000000000000000000000dead"
                && e.timestamp > 0));
        }
    }

    /// A fungible transfer list never carries an NFT's row as an amount, and
    /// a tracked token's row is kept when the explorer leaves its decimals
    /// out: the tracked token's own decimals scale it.
    #[tokio::test]
    async fn fungible_rows_are_raw_amounts_of_fungible_tokens() {
        let row = |token_id: Option<&str>, decimals: &str| {
            let mut row = json!({"blockNumber": "1", "timeStamp": "1700000000", "hash": "0xh",
                "from": "0xA", "to": "0xB", "contractAddress": "0xC", "tokenName": "T",
                "tokenSymbol": "T", "tokenDecimal": decimals, "value": "1000", "logIndex": "0"});
            if let Some(id) = token_id {
                row["tokenID"] = json!(id);
            }
            row
        };
        let server = MockServer::start().await;
        Mock::given(query_param("action", "tokentx"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "1", "message": "OK",
                "result": [row(None, "6"), row(Some("7"), ""), row(None, "")]})))
            .mount(&server)
            .await;
        let (entries, full) = BlockscoutClient::new()
            .fetch_token_transfers("0xa", EvmHistorySource::Open(&server.uri()), 1, 3)
            .await
            .unwrap();
        // The page was full: an NFT row on it still counts towards that.
        assert!(full);
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|e| e.amount_raw == "1000" && e.contract == "0xc")
        );
    }
}

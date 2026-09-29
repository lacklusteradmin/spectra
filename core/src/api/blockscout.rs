//! The Blockscout adapter: the Etherscan-compatible `module=account` API that
//! Blockscout and Routescan serve. EVM nodes cannot index by address, so EVM
//! history comes from here.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::evm_json_rpc::format_evm_decimals;
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

/// One ERC-20 token transfer returned by Etherscan `tokentx`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmTokenTransferEntry {
    pub contract: String,
    pub symbol: String,
    pub token_name: String,
    pub decimals: u8,
    pub from: String,
    pub to: String,
    /// Raw integer amount (base units), as string.
    pub amount_raw: String,
    /// Human-readable amount (raw / 10^decimals), up to 6 decimal places.
    pub amount_display: String,
    pub txid: String,
    pub block_number: u64,
    pub log_index: u32,
    pub timestamp: u64,
}

/// Build a query for a configured keyless explorer, refusing unavailable history.
pub fn explorer_query_url(source: EvmHistorySource<'_>, params: &str) -> Result<String, String> {
    match source {
        EvmHistorySource::Open(base) => {
            let base = base.trim_end_matches('/');
            // Routescan exposes the same wire contract at /etherscan.
            let suffix = if base.ends_with("/etherscan") || base.ends_with("/api") {
                ""
            } else {
                "/api"
            };
            Ok(format!("{base}{suffix}?{params}"))
        }
        EvmHistorySource::Unavailable => {
            Err("no explorer serves this chain's transaction history".to_string())
        }
    }
}

/// Distinguish empty history from explorer refusal by the shape of `result`.
/// Both can have status "0": an empty array is empty history, while a
/// non-array result is an error. Do not rely on explorer message wording.
fn etherscan_result_rows(status: &str, message: &str, result: Value) -> Result<Vec<Value>, String> {
    match result {
        Value::Array(rows) => Ok(rows),
        _ if status == "1" => Err(format!("explorer returned a non-list result: {message}")),
        _ => Err(format!("explorer refused: {message}")),
    }
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

    pub async fn fetch_history(
        &self,
        address: &str,
        source: EvmHistorySource<'_>,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<EvmHistoryEntry>, String> {
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
            .map_err(|e| format!("history parse: {e}"))?;

        let addr_norm = address.to_lowercase();

        items
            .into_iter()
            .map(|tx| {
                let status = match (tx.is_error.as_deref(), tx.receipt_status.as_deref()) {
                    (Some("1"), _) | (_, Some("0")) => "failed",
                    (Some("0"), _) | (_, Some("1")) => "confirmed",
                    _ => return Err("history missing execution status".to_string()),
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

    /// Fetch ERC-20 token transfer history for `address` via Etherscan `tokentx`.
    pub async fn fetch_token_transfers(
        &self,
        address: &str,
        source: EvmHistorySource<'_>,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<EvmTokenTransferEntry>, String> {
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
            token_name: String,
            token_symbol: String,
            token_decimal: String,
            value: String,
            #[serde(default)]
            log_index: String,
        }

        let resp: ApiResp = self.client.get_json(&url, RetryProfile::ChainRead).await?;
        let rows = etherscan_result_rows(&resp.status, &resp.message, resp.result)?;

        let items: Vec<TxItem> = serde_json::from_value(serde_json::Value::Array(rows))
            .map_err(|e| format!("token transfer parse: {e}"))?;

        items
            .into_iter()
            .map(|tx| {
                let decimals: u8 = tx.token_decimal.parse().unwrap_or(18);
                let amount_display = format_evm_decimals(&tx.value, decimals);
                let timestamp =
                    crate::api::time::confirmed_history_time(tx.time_stamp.parse().ok(), &tx.hash)?;
                Ok(EvmTokenTransferEntry {
                    contract: tx.contract_address.to_lowercase(),
                    symbol: tx.token_symbol.clone(),
                    token_name: tx.token_name.clone(),
                    decimals,
                    from: tx.from.to_lowercase(),
                    to: tx.to.to_lowercase(),
                    amount_raw: tx.value,
                    amount_display,
                    txid: tx.hash,
                    block_number: tx.block_number.parse().unwrap_or(0),
                    log_index: tx.log_index.parse().unwrap_or(0),
                    timestamp,
                })
            })
            .collect()
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
        let catalog = crate::app_core::endpoint_catalog().unwrap();
        for chain in Chain::all().filter(|chain| chain.is_evm()) {
            if let EvmHistorySource::Open(url) = chain.evm_history_source() {
                assert!(catalog.endpoint_records.iter().any(|record| {
                    record.chain_id == chain.str_id()
                        && record.endpoint == url
                        && record
                            .capabilities
                            .contains(&crate::EndpointCapability::History)
                }));
            }
            if chain != chain.mainnet_counterpart() {
                assert_eq!(chain.evm_history_source(), EvmHistorySource::Unavailable);
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

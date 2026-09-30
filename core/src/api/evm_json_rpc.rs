//! The EVM JSON-RPC adapter, for every EVM chain: balances, nonces, fee
//! history, gas estimates, ERC-20 reads through `eth_call`, receipts and raw
//! transaction broadcast. Address history is not a node method; that is
//! `blockscout`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::{HttpClient, RetryProfile, race};

// ── ERC-20 4-byte function selectors  (keccak256(signature)[..4])
pub(crate) const SEL_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31]; // balanceOf(address)
pub(crate) const SEL_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67]; // decimals()
pub(crate) const SEL_SYMBOL: [u8; 4] = [0x95, 0xd8, 0x9b, 0x41]; // symbol()
pub(crate) const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)

// ── Internal helpers shared by derive/fetch/send

/// Strip the `0x` prefix from a hex string and decode to bytes.
pub(crate) fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(stripped).map_err(|e| format!("hex decode: {e}"))
}

/// Parse a `0x`-prefixed hex integer (as returned by JSON-RPC) into u128.
pub(crate) fn parse_hex_u128(s: &str) -> Result<u128, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u128::from_str_radix(stripped, 16).map_err(|e| format!("hex u128 parse: {e}"))
}

/// Parse a `0x`-prefixed hex integer into u64.
pub(crate) fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(stripped, 16).map_err(|e| format!("hex u64 parse: {e}"))
}

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmBalance {
    /// Native token balance in the chain's smallest unit (wei for ETH).
    pub balance_wei: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmFeeEstimate {
    /// EIP-1559 base fee (wei).
    pub base_fee_wei: u128,
    /// Suggested priority fee / miner tip (wei).
    pub priority_fee_wei: u128,
    /// Max total fee per gas to set on the transaction.
    pub max_fee_per_gas_wei: u128,
    /// Estimated total fee for a standard 21,000-gas transfer (wei).
    pub estimated_fee_wei: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmSendResult {
    pub txid: String,
    /// Nonce actually used for this transaction. Set by the signing path
    /// (either the caller's override or the fetched pending nonce).
    #[serde(default)]
    pub nonce: u64,
    /// Signed transaction bytes as a 0x-prefixed hex string — callers that
    /// need to re-broadcast or log the raw envelope can use this directly.
    /// Empty on `broadcast_raw` paths where the raw hex was already supplied.
    #[serde(default)]
    pub raw_tx_hex: String,
    /// Gas limit used for the transaction.
    #[serde(default)]
    pub gas_limit: u64,
    /// EIP-1559 max fee per gas (wei, decimal string).
    #[serde(default)]
    pub max_fee_per_gas_wei: String,
    /// EIP-1559 max priority fee per gas (wei, decimal string).
    #[serde(default)]
    pub max_priority_fee_per_gas_wei: String,
}

/// Balance of an ERC-20 token held at a given address.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct Erc20Balance {
    /// Token contract (checksummed lowercase hex, 0x-prefixed).
    pub contract: String,
    /// Holder address.
    pub holder: String,
    /// Raw balance in the token's smallest unit (u256 encoded as decimal string).
    pub balance_raw: String,
    /// The balance scaled by `decimals`, exactly.
    pub balance_display: String,
    /// Token decimals (cached from the contract).
    pub decimals: u8,
    /// Token symbol.
    pub symbol: String,
}

/// Lightweight ERC-20 metadata (symbol + decimals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Erc20Metadata {
    pub symbol: String,
    pub decimals: u8,
}

/// Transaction receipt returned by `eth_getTransactionReceipt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmReceipt {
    pub tx_hash: String,
    /// Block number where the transaction was included, or `None` if still pending.
    pub block_number: Option<u64>,
    /// `"0x1"` = success, `"0x0"` = reverted. `None` = legacy (pre-Byzantium) chains.
    pub status: Option<String>,
    /// Actual gas consumed (decimal string).
    pub gas_used: Option<String>,
    /// Effective gas price in wei (decimal string).
    pub effective_gas_price_wei: Option<String>,
    /// `true` when the transaction has been included in a block.
    pub is_confirmed: bool,
    /// `true` when status == "0x0" (execution failed / reverted).
    pub is_failed: bool,
}

// ── EVM client

pub struct EvmClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) chain_id: u64,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl EvmClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>, chain_id: u64) -> Self {
        Self {
            endpoints,
            chain_id,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        crate::api::json_rpc::call(
            crate::EndpointApi::EvmJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }

    /// Send a JSON-RPC 2.0 batch request. Returns one result per request in
    /// the same order as `requests` regardless of how the server orders the
    /// batch response.
    pub(crate) async fn call_batch(
        &self,
        requests: Vec<(&str, Value)>,
    ) -> Result<Vec<Value>, String> {
        let n = requests.len();
        if n == 0 {
            return Ok(vec![]);
        }
        let batch: Value = Value::Array(
            requests
                .into_iter()
                .enumerate()
                .map(|(i, (method, params))| {
                    json!({
                        "jsonrpc": "2.0",
                        "id": (i + 1) as u64,
                        "method": method,
                        "params": params,
                    })
                })
                .collect(),
        );
        let body = std::sync::Arc::new(batch);
        race(&self.endpoints, |url| {
            let client = self.client.clone();
            let body = std::sync::Arc::clone(&body);
            async move {
                let resp: Value = client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await?;
                let arr = resp
                    .as_array()
                    .ok_or_else(|| "batch: expected array response".to_string())?;
                // JSON-RPC 2.0 allows out-of-order batch responses; re-order by id.
                let mut indexed: Vec<(usize, Value)> = arr
                    .iter()
                    .map(|v| {
                        let id = v["id"].as_u64().unwrap_or(0) as usize;
                        (id, v.clone())
                    })
                    .collect();
                indexed.sort_unstable_by_key(|(id, _)| *id);
                if indexed.len() != n {
                    return Err(format!(
                        "batch: expected {n} responses, got {}",
                        indexed.len()
                    ));
                }
                indexed
                    .into_iter()
                    .map(|(_, v)| {
                        if let Some(err) = v.get("error") {
                            return Err(format!("rpc error: {err}"));
                        }
                        v.get("result")
                            .cloned()
                            .ok_or_else(|| "batch: missing result field".to_string())
                    })
                    .collect()
            }
        })
        .await
    }
}

// EVM fetch paths: native balance, nonce, fee estimate, gas estimate, code,
// receipts, tx nonce by hash, ENS resolution, ERC-20 balance + metadata,
// Keyless explorer history + token transfers.

fn parse_fee_history(result: &Value) -> Result<EvmFeeEstimate, String> {
    // Base fee of the *next* block is the last entry in baseFeePerGas.
    let base_fees = result
        .get("baseFeePerGas")
        .and_then(|v| v.as_array())
        .ok_or("feeHistory: missing baseFeePerGas")?;
    let base_fee_hex = base_fees
        .last()
        .and_then(|v| v.as_str())
        .ok_or("feeHistory: empty baseFeePerGas")?;
    let base_fee_wei = parse_hex_u128(base_fee_hex)?;

    // 25th-percentile reward from the most recent block as priority fee.
    //
    // Two different things look alike here and only one of them is a failure.
    // A node that answers without a `reward` array did not answer the question
    // this call asked — the request always names percentiles — and there is
    // nothing to infer from that, so it is an error. A node that answers with
    // an *empty* sample list for the block did answer: no transaction in it
    // paid a priority fee, which is the normal state of a quiet L2 or testnet.
    // The priority fee there is zero, not a number picked to stand in for one:
    // `max_fee` still covers the base fee, so the transaction lands, and a
    // fabricated gwei would only overpay. Refusing that case instead left the
    // send screen unable to quote a fee at all on those chains.
    let samples = result
        .get("reward")
        .and_then(|r| r.as_array())
        .and_then(|blocks| blocks.last())
        .and_then(|block| block.as_array())
        .ok_or("feeHistory: missing reward")?;
    let priority_fee_wei: u128 = match samples.first() {
        Some(sample) => parse_hex_u128(
            sample
                .as_str()
                .ok_or("feeHistory: reward sample is not a hex string")?,
        )?,
        None => 0,
    };

    // maxFeePerGas = 2 * baseFee + priorityFee (EIP-1559 recommended).
    let max_fee_per_gas_wei = base_fee_wei
        .saturating_mul(2)
        .saturating_add(priority_fee_wei);
    let estimated_fee_wei = max_fee_per_gas_wei.saturating_mul(21_000);

    Ok(EvmFeeEstimate {
        base_fee_wei,
        priority_fee_wei,
        max_fee_per_gas_wei,
        estimated_fee_wei,
    })
}

impl EvmClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<EvmBalance, String> {
        let result = self
            .call("eth_getBalance", json!([address, "latest"]))
            .await?;
        let hex = result.as_str().ok_or("eth_getBalance: expected string")?;
        let wei = parse_hex_u128(hex)?;
        Ok(EvmBalance {
            balance_wei: wei.to_string(),
        })
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u64, String> {
        let result = self
            .call("eth_getTransactionCount", json!([address, "pending"]))
            .await?;
        let hex = result
            .as_str()
            .ok_or("eth_getTransactionCount: expected string")?;
        parse_hex_u64(hex)
    }

    pub async fn fetch_fee_estimate(&self) -> Result<EvmFeeEstimate, String> {
        let result = self
            .call("eth_feeHistory", json!([4, "latest", [25, 75]]))
            .await?;
        parse_fee_history(&result)
    }

    pub async fn estimate_gas(
        &self,
        from: &str,
        to: &str,
        value_wei: u128,
        data: Option<&str>,
    ) -> Result<u64, String> {
        let mut obj = json!({
            "from": from,
            "to": to,
            "value": format!("0x{:x}", value_wei),
        });
        if let Some(d) = data {
            obj["data"] = json!(d);
        }
        let result = self.call("eth_estimateGas", json!([obj])).await?;
        let hex = result.as_str().ok_or("eth_estimateGas: expected string")?;
        parse_hex_u64(hex)
    }

    /// Fetch an ERC-20 `balanceOf(holder)` and normalize to display form.
    pub async fn fetch_erc20_balance(
        &self,
        contract: &str,
        holder: &str,
    ) -> Result<Erc20Balance, String> {
        let raw = self.fetch_erc20_balance_of(contract, holder).await?;
        let metadata = self.fetch_erc20_metadata(contract).await?;
        let balance_display = crate::decimal::from_units(raw, u32::from(metadata.decimals));
        Ok(Erc20Balance {
            contract: contract.to_lowercase(),
            holder: holder.to_lowercase(),
            balance_raw: raw.to_string(),
            balance_display,
            decimals: metadata.decimals,
            symbol: metadata.symbol,
        })
    }

    /// Raw `balanceOf` call — cheapest way to refresh a known-token balance.
    pub async fn fetch_erc20_balance_of(
        &self,
        contract: &str,
        holder: &str,
    ) -> Result<u128, String> {
        let data = encode_erc20_balance_of(holder)?;
        let result = self
            .call(
                "eth_call",
                json!([
                    {
                        "to": contract,
                        "data": format!("0x{}", hex::encode(&data)),
                    },
                    "latest"
                ]),
            )
            .await?;
        let hex_str = result
            .as_str()
            .ok_or("eth_call balanceOf: expected string")?;
        parse_hex_u128(hex_str)
    }

    /// Resolve an ENS name to a checksummed Ethereum address via the ENS Ideas API.
    pub async fn resolve_ens(&self, name: &str) -> Result<Option<String>, String> {
        let normalized = name.trim().to_lowercase();
        if normalized.is_empty() || !normalized.ends_with(".eth") || normalized.contains(' ') {
            return Ok(None);
        }
        let encoded = percent_encode(&normalized);
        let url = format!("https://api.ensideas.com/ens/resolve/{encoded}");
        let resp: Value = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await
            .map_err(|e| format!("ENS resolve: {e}"))?;
        let address = match resp.get("address").and_then(|v| v.as_str()) {
            Some(a) if !a.is_empty() => a.to_string(),
            _ => return Ok(None),
        };
        // Basic EVM address validation: 0x + 40 hex chars.
        let norm = address.trim().to_lowercase();
        if norm.len() == 42
            && norm.starts_with("0x")
            && norm[2..].chars().all(|c| c.is_ascii_hexdigit())
        {
            Ok(Some(norm))
        } else {
            Ok(None)
        }
    }

    /// Fetch a transaction receipt by hash. Returns `None` when the
    /// transaction is not yet mined (pending). Returns an error only on
    /// RPC failure.
    pub async fn fetch_receipt(&self, tx_hash: &str) -> Result<Option<EvmReceipt>, String> {
        let result = self
            .call("eth_getTransactionReceipt", json!([tx_hash]))
            .await?;
        if result.is_null() {
            return Ok(None);
        }
        let block_number = result
            .get("blockNumber")
            .and_then(|v| v.as_str())
            .filter(|s| *s != "0x" && !s.is_empty())
            .map(parse_hex_u64)
            .transpose()?;
        let status = result
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let gas_used = result
            .get("gasUsed")
            .and_then(|v| v.as_str())
            .map(parse_hex_u128)
            .transpose()?
            .map(|n| n.to_string());
        let effective_gas_price_wei = result
            .get("effectiveGasPrice")
            .and_then(|v| v.as_str())
            .map(parse_hex_u128)
            .transpose()?
            .map(|n| n.to_string());
        let is_confirmed = block_number.is_some();
        let is_failed = status.as_deref() == Some("0x0");
        Ok(Some(EvmReceipt {
            tx_hash: tx_hash.to_string(),
            block_number,
            status,
            gas_used,
            effective_gas_price_wei,
            is_confirmed,
            is_failed,
        }))
    }

    /// Fetch the bytecode deployed at `address` (eth_getCode).
    pub async fn fetch_code(&self, address: &str) -> Result<String, String> {
        let result = self.call("eth_getCode", json!([address, "latest"])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "eth_getCode: expected string".to_string())
    }

    /// Fetch the nonce of an already-submitted transaction by hash.
    pub async fn fetch_tx_nonce(&self, tx_hash: &str) -> Result<u64, String> {
        let result = self
            .call("eth_getTransactionByHash", json!([tx_hash]))
            .await?;
        let nonce_hex = result
            .get("nonce")
            .and_then(|v| v.as_str())
            .ok_or("eth_getTransactionByHash: missing nonce")?;
        parse_hex_u64(nonce_hex)
    }

    /// Fetch token metadata (symbol + decimals) in a single batch request.
    pub async fn fetch_erc20_metadata(&self, contract: &str) -> Result<Erc20Metadata, String> {
        let results = self
            .call_batch(vec![
                (
                    "eth_call",
                    json!([{"to": contract, "data": format!("0x{}", hex::encode(SEL_DECIMALS))}, "latest"]),
                ),
                (
                    "eth_call",
                    json!([{"to": contract, "data": format!("0x{}", hex::encode(SEL_SYMBOL))}, "latest"]),
                ),
            ])
            .await?;
        let decimals = crate::api::checked_token_decimals(parse_hex_u128(
            results[0]
                .as_str()
                .ok_or("eth_call decimals: expected string")?,
        )?)?;
        let symbol = results[1]
            .as_str()
            .and_then(decode_abi_string_or_bytes32)
            .unwrap_or_default();
        Ok(Erc20Metadata { symbol, decimals })
    }
}

// ── ERC-20 ABI helpers (shared with send.rs)

/// The `transfer(address,uint256)` selector as its hex spelling, for
/// [`is_erc20_transfer`] and the assembler in [`crate::send::ethereum`].
pub(crate) fn erc20_transfer_selector_hex() -> String {
    hex::encode(SEL_TRANSFER)
}

/// Whether this calldata is an ERC-20 `transfer(address,uint256)`.
///
/// The inverse of the assembler: a transfer is addressed *to the token
/// contract*, so a caller holding both the destination and the calldata can
/// name the token being moved from this alone, without being told.
pub(crate) fn is_erc20_transfer(data_hex: &str) -> bool {
    let body = data_hex.strip_prefix("0x").unwrap_or(data_hex);
    body.get(..8)
        .is_some_and(|selector| selector.eq_ignore_ascii_case(&erc20_transfer_selector_hex()))
}

/// Encode a `balanceOf(address)` call.
pub fn encode_erc20_balance_of(holder: &str) -> Result<Vec<u8>, String> {
    let holder_bytes = decode_hex(holder)?;
    if holder_bytes.len() != 20 {
        return Err(format!("invalid EVM holder length: {}", holder_bytes.len()));
    }
    let mut out = Vec::with_capacity(4 + 32);
    out.extend_from_slice(&SEL_BALANCE_OF);
    out.extend_from_slice(&[0u8; 12]); // left-pad 20-byte address to 32 bytes
    out.extend_from_slice(&holder_bytes);
    Ok(out)
}

/// Decode an ABI-encoded `string` return value, or fall back to a
/// `bytes32`-style null-terminated ASCII name (MKR, DAI-era tokens).
pub fn decode_abi_string_or_bytes32(hex_str: &str) -> Option<String> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(stripped).ok()?;
    if bytes.is_empty() {
        return None;
    }

    // ABI `string` layout: offset (32) | length (32) | data
    if bytes.len() >= 64 {
        let offset = u64::from_be_bytes(bytes[24..32].try_into().ok()?) as usize;
        if offset == 32 && bytes.len() >= offset + 32 {
            let len_start = offset;
            let len_end = offset + 32;
            let len = u64::from_be_bytes(bytes[len_start + 24..len_end].try_into().ok()?) as usize;
            let data_start = len_end;
            let data_end = data_start.checked_add(len)?;
            if bytes.len() >= data_end {
                let slice = &bytes[data_start..data_end];
                if let Ok(s) = std::str::from_utf8(slice) {
                    let trimmed = s.trim_end_matches(char::from(0));
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }

    // Fallback: treat as bytes32, trim trailing null bytes.
    let trimmed: Vec<u8> = bytes
        .iter()
        .take(32)
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    let s = String::from_utf8(trimmed).ok()?;
    if s.is_empty() { None } else { Some(s) }
}

/// Percent-encode a string for use in a URL path component.
/// Only encodes characters that are not safe in a path segment.
fn percent_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                let byte = c as u8;
                format!("%{:02X}", byte).chars().collect()
            }
        })
        .collect()
}

impl EvmClient {
    /// Broadcast a pre-signed raw transaction hex (0x-prefixed).
    pub async fn broadcast_raw(&self, hex_tx: &str) -> Result<EvmSendResult, String> {
        let result = self.call("eth_sendRawTransaction", json!([hex_tx])).await?;
        let txid = result
            .as_str()
            .ok_or("eth_sendRawTransaction: expected string")?
            .to_string();
        Ok(EvmSendResult {
            txid,
            nonce: 0,
            raw_tx_hex: hex_tx.to_string(),
            gas_limit: 0,
            max_fee_per_gas_wei: String::new(),
            max_priority_fee_per_gas_wei: String::new(),
        })
    }
}

#[cfg(test)]
mod fee_history_tests {
    use super::*;

    /// The reward array is per block, and each block's entry is one sample per
    /// requested percentile. The last block's 25th percentile is the one this
    /// reads.
    #[test]
    fn the_last_blocks_lowest_percentile_is_the_priority_fee() {
        let fees = parse_fee_history(&json!({
            "baseFeePerGas": ["0x3b9aca00", "0x77359400"],
            "reward": [["0x1", "0x2"], ["0x5f5e100", "0xbebc200"]],
        }))
        .unwrap();
        assert_eq!(fees.base_fee_wei, 0x7735_9400);
        assert_eq!(fees.priority_fee_wei, 0x5f5_e100);
        assert_eq!(fees.max_fee_per_gas_wei, 0x7735_9400 * 2 + 0x5f5_e100);
        assert_eq!(fees.estimated_fee_wei, fees.max_fee_per_gas_wei * 21_000);
    }

    /// A block nobody transacted in reports no sample, and the honest priority
    /// fee for it is zero — `max_fee` still covers the base fee, so the
    /// transaction lands. Refusing this case left the send screen unable to
    /// quote a fee at all on a quiet L2 or testnet.
    #[test]
    fn an_empty_reward_sample_is_a_zero_priority_fee() {
        let fees = parse_fee_history(&json!({
            "baseFeePerGas": ["0x3b9aca00"],
            "reward": [[]],
        }))
        .unwrap();
        assert_eq!(fees.priority_fee_wei, 0);
        assert_eq!(fees.base_fee_wei, 0x3b9a_ca00);
        assert_eq!(fees.max_fee_per_gas_wei, 0x3b9a_ca00 * 2);
    }

    /// No `reward` array at all is a node that did not answer what was asked —
    /// the request always names percentiles — so there is nothing to infer.
    #[test]
    fn a_missing_reward_array_is_refused() {
        for body in [
            json!({"baseFeePerGas": ["0x3b9aca00"]}),
            json!({"baseFeePerGas": ["0x3b9aca00"], "reward": []}),
            json!({"baseFeePerGas": ["0x3b9aca00"], "reward": Value::Null}),
            json!({"baseFeePerGas": ["0x3b9aca00"], "reward": ["0x1"]}),
        ] {
            assert!(parse_fee_history(&body).is_err(), "{body}");
        }
    }

    /// A malformed response is still a refusal — the base fee is the estimate
    /// and there is nothing to fall back to.
    #[test]
    fn a_malformed_response_is_refused() {
        for body in [
            json!({}),
            json!({"baseFeePerGas": []}),
            json!({"baseFeePerGas": ["zz"]}),
            json!({"baseFeePerGas": [1]}),
            json!({"baseFeePerGas": ["0x1"], "reward": [["zz"]]}),
            json!({"baseFeePerGas": ["0x1"], "reward": [[7]]}),
            json!({"baseFeePerGas": ["0x1"], "reward": [["0x1"], "nope"]}),
        ] {
            assert!(parse_fee_history(&body).is_err(), "{body}");
        }
    }
}

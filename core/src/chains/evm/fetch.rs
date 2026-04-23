//! EVM fetch paths: native balance, nonce, fee estimate, gas estimate, code,
//! receipts, tx nonce by hash, ENS resolution, ERC-20 balance + metadata,
//! Etherscan V2 history + token transfers.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::http::RetryProfile;

use super::{
    parse_hex_u128, parse_hex_u64, Erc20Balance, Erc20Metadata, EvmBalance, EvmClient,
    EvmFeeEstimate, EvmHistoryEntry, EvmReceipt, EvmTokenTransferEntry,
};

impl EvmClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<EvmBalance, String> {
        let result = self
            .call("eth_getBalance", json!([address, "latest"]))
            .await?;
        let hex = result.as_str().ok_or("eth_getBalance: expected string")?;
        let wei = parse_hex_u128(hex)?;
        let balance_display = format_ether(wei);
        Ok(EvmBalance {
            balance_wei: wei.to_string(),
            balance_display,
        })
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u64, String> {
        let result = self
            .call("eth_getTransactionCount", json!([address, "latest"]))
            .await?;
        let hex = result
            .as_str()
            .ok_or("eth_getTransactionCount: expected string")?;
        parse_hex_u64(hex)
    }

    pub async fn fetch_fee_estimate(&self) -> Result<EvmFeeEstimate, String> {
        // eth_feeHistory returns base fees and reward percentiles.
        let result = self
            .call("eth_feeHistory", json!([4, "latest", [25, 75]]))
            .await?;

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
        let priority_fee_wei: u128 = result
            .get("reward")
            .and_then(|r| r.as_array())
            .and_then(|arr| arr.last())
            .and_then(|r| r.as_array())
            .and_then(|r| r.first())
            .and_then(|v| v.as_str())
            .map(parse_hex_u128)
            .transpose()?
            .unwrap_or(1_000_000_000); // 1 gwei fallback

        // maxFeePerGas = 2 * baseFee + priorityFee (EIP-1559 recommended).
        let max_fee_per_gas_wei = base_fee_wei.saturating_mul(2).saturating_add(priority_fee_wei);
        let estimated_fee_wei = max_fee_per_gas_wei.saturating_mul(21_000);

        Ok(EvmFeeEstimate {
            base_fee_wei,
            priority_fee_wei,
            max_fee_per_gas_wei,
            estimated_fee_wei,
        })
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
        let balance_display = format_token_amount(raw, metadata.decimals);
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
        let hex_str = result.as_str().ok_or("eth_call balanceOf: expected string")?;
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

    /// Fetch token metadata (symbol + decimals).
    pub async fn fetch_erc20_metadata(&self, contract: &str) -> Result<Erc20Metadata, String> {
        let decimals_result = self
            .call(
                "eth_call",
                json!([
                    { "to": contract, "data": "0x313ce567" },
                    "latest"
                ]),
            )
            .await?;
        let decimals_hex = decimals_result
            .as_str()
            .ok_or("eth_call decimals: expected string")?;
        let decimals = parse_hex_u128(decimals_hex)? as u8;

        let symbol_result = self
            .call(
                "eth_call",
                json!([
                    { "to": contract, "data": "0x95d89b41" },
                    "latest"
                ]),
            )
            .await?;
        let symbol_hex = symbol_result
            .as_str()
            .ok_or("eth_call symbol: expected string")?;
        let symbol = decode_abi_string_or_bytes32(symbol_hex).unwrap_or_default();

        Ok(Erc20Metadata { symbol, decimals })
    }

    // ----------------------------------------------------------------
    // History (Etherscan V2 multi-chain endpoint)
    // ----------------------------------------------------------------

    pub async fn fetch_history(
        &self,
        address: &str,
        etherscan_api_base: &str,
        api_key: Option<&str>,
        etherscan_chain_id: u64,
    ) -> Result<Vec<EvmHistoryEntry>, String> {
        let addr_lower = address.to_lowercase();
        let mut url = format!(
            "{}/v2/api?chainid={}&module=account&action=txlist&address={}&sort=desc&page=1&offset=50",
            etherscan_api_base, etherscan_chain_id, addr_lower
        );
        if let Some(key) = api_key {
            url.push_str(&format!("&apikey={key}"));
        }

        #[derive(Deserialize)]
        struct ApiResp {
            status: String,
            result: Value,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TxItem {
            hash: String,
            block_number: String,
            time_stamp: String,
            from: String,
            to: String,
            value: String,
            gas_price: String,
            gas_used: String,
        }

        let resp: ApiResp = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await?;

        if resp.status != "1" {
            // Empty history returns status "0" — not an error.
            return Ok(vec![]);
        }

        let items: Vec<TxItem> = serde_json::from_value(resp.result)
            .map_err(|e| format!("history parse: {e}"))?;

        let addr_norm = address.to_lowercase();
        let entries = items
            .into_iter()
            .map(|tx| {
                let fee_wei = tx
                    .gas_price
                    .parse::<u128>()
                    .unwrap_or(0)
                    .saturating_mul(tx.gas_used.parse::<u128>().unwrap_or(0))
                    .to_string();
                EvmHistoryEntry {
                    txid: tx.hash,
                    block_number: tx.block_number.parse().unwrap_or(0),
                    timestamp: tx.time_stamp.parse().unwrap_or(0),
                    from: tx.from.clone(),
                    to: tx.to.clone(),
                    value_wei: tx.value,
                    fee_wei,
                    is_incoming: tx.to.to_lowercase() == addr_norm,
                }
            })
            .collect();

        Ok(entries)
    }

    /// Fetch ERC-20 token transfer history for `address` via Etherscan `tokentx`.
    pub async fn fetch_token_transfers(
        &self,
        address: &str,
        etherscan_api_base: &str,
        api_key: Option<&str>,
        etherscan_chain_id: u64,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<EvmTokenTransferEntry>, String> {
        let addr_lower = address.to_lowercase();
        let safe_page = page.max(1);
        let safe_size = page_size.max(1).min(500);

        let mut url = format!(
            "{}/v2/api?chainid={}&module=account&action=tokentx&address={}&page={}&offset={}&sort=desc",
            etherscan_api_base, etherscan_chain_id, addr_lower, safe_page, safe_size
        );
        if let Some(key) = api_key {
            url.push_str(&format!("&apikey={key}"));
        }

        #[derive(Deserialize)]
        struct ApiResp {
            status: String,
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

        let resp: ApiResp = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await?;

        if resp.status != "1" {
            // status "0" = empty history or rate limit — treat as empty.
            return Ok(vec![]);
        }

        let items: Vec<TxItem> = serde_json::from_value(resp.result)
            .map_err(|e| format!("token transfer parse: {e}"))?;

        let entries = items
            .into_iter()
            .map(|tx| {
                let decimals: u8 = tx.token_decimal.parse().unwrap_or(18);
                let amount_display = format_evm_decimals(&tx.value, decimals);
                EvmTokenTransferEntry {
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
                    timestamp: tx.time_stamp.parse().unwrap_or(0),
                }
            })
            .collect();

        Ok(entries)
    }
}

// ----------------------------------------------------------------
// ERC-20 ABI helpers (shared with send.rs)
// ----------------------------------------------------------------

/// Encode a `balanceOf(address)` call: `0x70a08231 || pad20(holder)`.
pub fn encode_erc20_balance_of(holder: &str) -> Result<Vec<u8>, String> {
    let holder_bytes = super::decode_hex(holder)?;
    if holder_bytes.len() != 20 {
        return Err(format!("invalid EVM holder length: {}", holder_bytes.len()));
    }
    let mut out = Vec::with_capacity(4 + 32);
    out.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]); // balanceOf(address)
    out.extend_from_slice(&[0u8; 12]); // left pad 20-byte address to 32 bytes
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
    let trimmed: Vec<u8> = bytes.iter().take(32).copied().take_while(|&b| b != 0).collect();
    let s = String::from_utf8(trimmed).ok()?;
    if s.is_empty() { None } else { Some(s) }
}

// ----------------------------------------------------------------
// Formatting
// ----------------------------------------------------------------

/// Format a raw `u128` token amount with the given decimals, trimming trailing
/// zeros and capping to 6 fractional digits for display.
pub fn format_token_amount(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let scale: u128 = 10u128.pow(decimals as u32);
    let whole = raw / scale;
    let frac = raw % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

/// Format a raw integer token amount string by `decimals`.
/// Equivalent to `format_token_amount` but accepts a decimal string as input
/// (Etherscan returns large values as strings to avoid JSON number overflow).
pub fn format_evm_decimals(raw_str: &str, decimals: u8) -> String {
    let raw: u128 = raw_str.parse().unwrap_or(0);
    format_token_amount(raw, decimals)
}

/// Format wei as a decimal ETH string with up to 6 significant decimal places.
pub fn format_ether(wei: u128) -> String {
    let whole = wei / 1_000_000_000_000_000_000u128;
    let frac = wei % 1_000_000_000_000_000_000u128;
    if frac == 0 {
        return whole.to_string();
    }
    // 18-digit fractional, trim trailing zeros.
    let frac_str = format!("{:018}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    // Show at most 6 decimal places.
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
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

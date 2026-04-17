//! EVM-compatible chain client (Ethereum, Arbitrum, Optimism, Avalanche, Base,
//! Hyperliquid, Ethereum Classic, etc.)
//!
//! Implements JSON-RPC over HTTPS using the shared `HttpClient`. Builds and
//! signs EIP-1559 transactions in Rust using secp256k1 + RLP encoding.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Internal helpers
// ----------------------------------------------------------------

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

// ----------------------------------------------------------------
// JSON-RPC helpers
// ----------------------------------------------------------------

/// Build a JSON-RPC 2.0 request body.
fn rpc(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    })
}

/// Strip the `0x` prefix from a hex string and decode to bytes.
fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(stripped).map_err(|e| format!("hex decode: {e}"))
}

/// Parse a `0x`-prefixed hex integer (as returned by JSON-RPC) into u128.
fn parse_hex_u128(s: &str) -> Result<u128, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u128::from_str_radix(stripped, 16).map_err(|e| format!("hex u128 parse: {e}"))
}

/// Parse a `0x`-prefixed hex integer into u64.
fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(stripped, 16).map_err(|e| format!("hex u64 parse: {e}"))
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmBalance {
    /// Native token balance in the chain's smallest unit (wei for ETH).
    pub balance_wei: String,
    /// Human-readable balance (18 decimal places).
    pub balance_display: String,
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
    /// Human-readable balance scaled by `decimals`, up to 6 fractional digits.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmHistoryEntry {
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

// ----------------------------------------------------------------
// EVM client
// ----------------------------------------------------------------

pub struct EvmClient {
    endpoints: Vec<String>,
    chain_id: u64,
    client: std::sync::Arc<HttpClient>,
}

impl EvmClient {
    pub fn new(endpoints: Vec<String>, chain_id: u64) -> Self {
        Self {
            endpoints,
            chain_id,
            client: HttpClient::shared(),
        }
    }

    // ----------------------------------------------------------------
    // Core JSON-RPC call
    // ----------------------------------------------------------------

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = rpc(method, params);
        with_fallback(&self.endpoints, |url| {
            let client = self.client.clone();
            let body = body.clone();
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainRead)
                    .await?;
                if let Some(err) = resp.get("error") {
                    return Err(format!("rpc error: {err}"));
                }
                resp.get("result")
                    .cloned()
                    .ok_or_else(|| "missing result field".to_string())
            }
        })
        .await
    }

    // ----------------------------------------------------------------
    // Fetch
    // ----------------------------------------------------------------

    pub async fn fetch_balance(&self, address: &str) -> Result<EvmBalance, String> {
        let result = self
            .call("eth_getBalance", json!([address, "latest"]))
            .await?;
        let hex = result
            .as_str()
            .ok_or("eth_getBalance: expected string")?;
        let wei = parse_hex_u128(hex)?;
        let balance_display = format_ether(wei);
        Ok(EvmBalance {
            balance_wei: wei.to_string(),
            balance_display,
        })
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u64, String> {
        let result = self
            .call(
                "eth_getTransactionCount",
                json!([address, "latest"]),
            )
            .await?;
        let hex = result.as_str().ok_or("eth_getTransactionCount: expected string")?;
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

    /// Bump a base fee by +10% (the minimum EIP-1559 replacement rule).
    /// Used by the UI to compute "speed up" / "cancel" suggested fees.
    pub fn bumped_for_replacement(&self, base: u128) -> u128 {
        // base * 110 / 100, saturating.
        base.saturating_mul(110) / 100
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

    // ----------------------------------------------------------------
    // Transaction building + signing + broadcast
    // ----------------------------------------------------------------

    /// Sign and broadcast an EIP-1559 ETH transfer.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        value_wei: u128,
        private_key_bytes: &[u8],
    ) -> Result<EvmSendResult, String> {
        self.sign_and_broadcast_with_overrides(
            from_address,
            to_address,
            value_wei,
            private_key_bytes,
            EvmSendOverrides::default(),
        )
        .await
    }

    /// Sign and broadcast an EIP-1559 ETH transfer with optional overrides
    /// for nonce, gas limit, and fee fields. Used for "speed up" and "cancel"
    /// (replacement-by-fee) flows and power-user custom fee edits.
    ///
    /// When `overrides.nonce` is Some(n), `n` is used directly without
    /// fetching the pending nonce — this is what makes replacement-by-fee
    /// work (reuse the stuck tx's nonce).
    pub async fn sign_and_broadcast_with_overrides(
        &self,
        from_address: &str,
        to_address: &str,
        value_wei: u128,
        private_key_bytes: &[u8],
        overrides: EvmSendOverrides,
    ) -> Result<EvmSendResult, String> {
        let nonce = match overrides.nonce {
            Some(n) => n,
            None => self.fetch_nonce(from_address).await?,
        };
        let (max_fee, max_priority) = resolve_fees(self, &overrides).await?;
        let gas_limit = overrides.gas_limit.unwrap_or(21_000);

        let raw_tx = build_eip1559_tx(
            self.chain_id,
            nonce,
            max_fee,
            max_priority,
            gas_limit,
            to_address,
            value_wei,
            &[],
            private_key_bytes,
        )?;

        let hex_tx = format!("0x{}", hex::encode(&raw_tx));
        let result = self
            .call("eth_sendRawTransaction", json!([hex_tx.clone()]))
            .await?;
        let txid = result
            .as_str()
            .ok_or("eth_sendRawTransaction: expected string")?
            .to_string();
        Ok(EvmSendResult {
            txid,
            nonce,
            raw_tx_hex: hex_tx,
            gas_limit,
            max_fee_per_gas_wei: max_fee.to_string(),
            max_priority_fee_per_gas_wei: max_priority.to_string(),
        })
    }

    // ----------------------------------------------------------------
    // ERC-20
    // ----------------------------------------------------------------

    /// Fetch an ERC-20 `balanceOf(holder)` and normalize to display form.
    ///
    /// This issues three `eth_call`s (balanceOf, decimals, symbol). For a
    /// hot path that repeats this query, prefer caching metadata via
    /// `fetch_erc20_metadata` and using `fetch_erc20_balance_of` directly.
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
        let hex_str = result
            .as_str()
            .ok_or("eth_call balanceOf: expected string")?;
        parse_hex_u128(hex_str)
    }

    /// Resolve an ENS name to a checksummed Ethereum address via the ENS Ideas API.
    ///
    /// Returns `None` when the name is syntactically invalid, not `.eth`-suffixed,
    /// or the API returned no address. Returns an error on network failure.
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
        if norm.len() == 42 && norm.starts_with("0x") && norm[2..].chars().all(|c| c.is_ascii_hexdigit()) {
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
    /// Returns the raw hex string (e.g. "0x" for EOAs, "0x608060…" for contracts).
    pub async fn fetch_code(&self, address: &str) -> Result<String, String> {
        let result = self
            .call("eth_getCode", json!([address, "latest"]))
            .await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "eth_getCode: expected string".to_string())
    }

    /// Fetch the nonce of an already-submitted transaction by hash.
    /// Used by the replacement-tx flow to pre-fill the nonce field.
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

    /// Sign and broadcast an ERC-20 `transfer(to, amount)` from `from`.
    ///
    /// Uses EIP-1559 pricing. `amount` is in the token's smallest unit
    /// (you must scale by decimals on the caller side).
    pub async fn sign_and_broadcast_erc20(
        &self,
        from_address: &str,
        contract: &str,
        to_address: &str,
        amount_raw: u128,
        private_key_bytes: &[u8],
    ) -> Result<EvmSendResult, String> {
        self.sign_and_broadcast_erc20_with_overrides(
            from_address,
            contract,
            to_address,
            amount_raw,
            private_key_bytes,
            EvmSendOverrides::default(),
        )
        .await
    }

    /// Sign and broadcast an ERC-20 transfer with fee/nonce overrides. See
    /// [`sign_and_broadcast_with_overrides`] for the semantics.
    pub async fn sign_and_broadcast_erc20_with_overrides(
        &self,
        from_address: &str,
        contract: &str,
        to_address: &str,
        amount_raw: u128,
        private_key_bytes: &[u8],
        overrides: EvmSendOverrides,
    ) -> Result<EvmSendResult, String> {
        let nonce = match overrides.nonce {
            Some(n) => n,
            None => self.fetch_nonce(from_address).await?,
        };
        let (max_fee, max_priority) = resolve_fees(self, &overrides).await?;

        let data = encode_erc20_transfer(to_address, amount_raw)?;
        let data_hex = format!("0x{}", hex::encode(&data));

        // Ask the node for the real gas limit unless the caller pinned one.
        let gas_limit = match overrides.gas_limit {
            Some(g) => g,
            None => self
                .estimate_gas(from_address, contract, 0u128, Some(&data_hex))
                .await
                .map(|g| g.saturating_add(g / 5)) // +20% buffer
                .unwrap_or(65_000),
        };

        let raw_tx = build_eip1559_tx(
            self.chain_id,
            nonce,
            max_fee,
            max_priority,
            gas_limit,
            contract,
            0u128,
            &data,
            private_key_bytes,
        )?;

        let hex_tx = format!("0x{}", hex::encode(&raw_tx));
        let result = self
            .call("eth_sendRawTransaction", json!([hex_tx.clone()]))
            .await?;
        let txid = result
            .as_str()
            .ok_or("eth_sendRawTransaction: expected string")?
            .to_string();
        Ok(EvmSendResult {
            txid,
            nonce,
            raw_tx_hex: hex_tx,
            gas_limit,
            max_fee_per_gas_wei: max_fee.to_string(),
            max_priority_fee_per_gas_wei: max_priority.to_string(),
        })
    }

    /// Broadcast a pre-signed raw transaction hex (0x-prefixed).
    pub async fn broadcast_raw(&self, hex_tx: &str) -> Result<EvmSendResult, String> {
        let result = self
            .call("eth_sendRawTransaction", json!([hex_tx]))
            .await?;
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

    // ----------------------------------------------------------------
    // History (via eth_getBlockByNumber + eth_getTransactionReceipt is
    // not practical for full history; we use the Etherscan-compatible
    // account API that all our providers expose).
    // ----------------------------------------------------------------

    pub async fn fetch_history(
        &self,
        address: &str,
        etherscan_api_base: &str,
        api_key: Option<&str>,
    ) -> Result<Vec<EvmHistoryEntry>, String> {
        let addr_lower = address.to_lowercase();
        let mut url = format!(
            "{}/api?module=account&action=txlist&address={}&sort=desc&page=1&offset=50",
            etherscan_api_base, addr_lower
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
    ///
    /// Makes a single request (no per-token contract filter) and returns all
    /// ERC-20 transfers. The caller filters by tracked token list.
    ///
    /// `etherscan_chain_id` is the numeric EVM chain ID injected as the
    /// `chainid` parameter required by the Etherscan v2 multi-chain endpoint.
    pub async fn fetch_token_transfers(
        &self,
        address: &str,
        etherscan_api_base: &str,
        api_key: Option<&str>,
        etherscan_chain_id: Option<u64>,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<EvmTokenTransferEntry>, String> {
        let addr_lower = address.to_lowercase();
        let safe_page = page.max(1);
        let safe_size = page_size.max(1).min(500);

        let mut url = format!(
            "{}/api?module=account&action=tokentx&address={}&page={}&offset={}&sort=desc",
            etherscan_api_base, addr_lower, safe_page, safe_size
        );
        if let Some(cid) = etherscan_chain_id {
            url = format!("{}&chainid={}", url, cid);
        }
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
// EIP-1559 transaction builder + signer
// ----------------------------------------------------------------

/// Build a signed EIP-1559 (type 2) transaction.
///
/// Returns the raw RLP-encoded transaction bytes, ready to be hex-encoded and
/// broadcast via `eth_sendRawTransaction`.
/// Optional overrides for EIP-1559 sends. Any `None` field falls back to the
/// default behavior (latest-nonce / recommended fee / estimated gas limit).
///
/// * `nonce` — reuse a stuck transaction's nonce to build a replacement. The
///   EIP-1559 replacement-by-fee rule requires the new tx to bump BOTH
///   `max_fee_per_gas_wei` and `max_priority_fee_per_gas_wei` by at least 10%
///   vs. the stuck one.
/// * `max_fee_per_gas_wei` / `max_priority_fee_per_gas_wei` — explicit fee
///   fields. If either is `None` we fetch `fetch_fee_estimate()` and fill
///   the missing one from the suggestion.
/// * `gas_limit` — pin the gas limit instead of calling `eth_estimateGas`.
#[derive(Debug, Clone, Default)]
pub struct EvmSendOverrides {
    pub nonce: Option<u64>,
    pub max_fee_per_gas_wei: Option<u128>,
    pub max_priority_fee_per_gas_wei: Option<u128>,
    pub gas_limit: Option<u64>,
}

/// Resolve (max_fee_per_gas, max_priority_fee_per_gas) from overrides plus
/// fallback `fetch_fee_estimate()` values. If both fields are set, no RPC
/// call is made.
async fn resolve_fees(
    client: &EvmClient,
    overrides: &EvmSendOverrides,
) -> Result<(u128, u128), String> {
    match (
        overrides.max_fee_per_gas_wei,
        overrides.max_priority_fee_per_gas_wei,
    ) {
        (Some(mf), Some(mp)) => Ok((mf, mp)),
        (mf_opt, mp_opt) => {
            let fee = client.fetch_fee_estimate().await?;
            Ok((
                mf_opt.unwrap_or(fee.max_fee_per_gas_wei),
                mp_opt.unwrap_or(fee.priority_fee_wei),
            ))
        }
    }
}

pub fn build_eip1559_tx(
    chain_id: u64,
    nonce: u64,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
    gas_limit: u64,
    to: &str,
    value_wei: u128,
    data: &[u8],
    private_key_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    // Decode `to` address.
    let to_bytes = decode_hex(to)?;
    if to_bytes.len() != 20 {
        return Err(format!("invalid EVM address length: {}", to_bytes.len()));
    }

    // --- RLP-encode the signing payload ---
    // EIP-1559 signing payload:
    //   0x02 || RLP([chain_id, nonce, max_priority_fee, max_fee, gas_limit,
    //                 to, value, data, access_list])
    let unsigned_rlp = rlp_encode_list(&[
        rlp_encode_u64(chain_id),
        rlp_encode_u64(nonce),
        rlp_encode_u128(max_priority_fee_per_gas),
        rlp_encode_u128(max_fee_per_gas),
        rlp_encode_u64(gas_limit),
        rlp_encode_bytes(&to_bytes),
        rlp_encode_u128(value_wei),
        rlp_encode_bytes(data),
        rlp_encode_list(&[]), // empty access list
    ]);

    let mut signing_payload = vec![0x02u8];
    signing_payload.extend_from_slice(&unsigned_rlp);

    // --- keccak256 hash ---
    let msg_hash = keccak256(&signing_payload);

    // --- secp256k1 sign ---
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg = Message::from_digest_slice(&msg_hash).map_err(|e| format!("msg: {e}"))?;
    let (rec_id, sig_bytes) = secp
        .sign_ecdsa_recoverable(&msg, &secret_key)
        .serialize_compact();

    let v: u64 = rec_id.to_i32() as u64; // 0 or 1 for EIP-1559
    let r = &sig_bytes[..32];
    let s = &sig_bytes[32..];

    // --- RLP-encode the full signed transaction ---
    let signed_rlp = rlp_encode_list(&[
        rlp_encode_u64(chain_id),
        rlp_encode_u64(nonce),
        rlp_encode_u128(max_priority_fee_per_gas),
        rlp_encode_u128(max_fee_per_gas),
        rlp_encode_u64(gas_limit),
        rlp_encode_bytes(&to_bytes),
        rlp_encode_u128(value_wei),
        rlp_encode_bytes(data),
        rlp_encode_list(&[]), // empty access list
        rlp_encode_u64(v),
        rlp_encode_bytes(r),
        rlp_encode_bytes(s),
    ]);

    let mut raw = vec![0x02u8];
    raw.extend_from_slice(&signed_rlp);
    Ok(raw)
}

// ----------------------------------------------------------------
// Minimal RLP encoder
// ----------------------------------------------------------------

fn rlp_encode_u64(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![0x80]; // RLP empty string = 0
    }
    let bytes = v.to_be_bytes();
    let trimmed: Vec<u8> = bytes.iter().copied().skip_while(|&b| b == 0).collect();
    rlp_encode_bytes(&trimmed)
}

fn rlp_encode_u128(v: u128) -> Vec<u8> {
    if v == 0 {
        return vec![0x80];
    }
    let bytes = v.to_be_bytes();
    let trimmed: Vec<u8> = bytes.iter().copied().skip_while(|&b| b == 0).collect();
    rlp_encode_bytes(&trimmed)
}

fn rlp_encode_bytes(data: &[u8]) -> Vec<u8> {
    if data.len() == 1 && data[0] < 0x80 {
        return vec![data[0]];
    }
    let mut out = rlp_length_prefix(data.len(), 0x80);
    out.extend_from_slice(data);
    out
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.iter().flat_map(|v| v.iter().copied()).collect();
    let mut out = rlp_length_prefix(payload.len(), 0xc0);
    out.extend_from_slice(&payload);
    out
}

fn rlp_length_prefix(len: usize, offset: u8) -> Vec<u8> {
    if len < 56 {
        vec![offset + len as u8]
    } else {
        let len_bytes = (len as u64).to_be_bytes();
        let trimmed: Vec<u8> = len_bytes.iter().copied().skip_while(|&b| b == 0).collect();
        let mut out = vec![offset + 55 + trimmed.len() as u8];
        out.extend_from_slice(&trimmed);
        out
    }
}

// ----------------------------------------------------------------
// keccak256
// ----------------------------------------------------------------

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut h = Keccak::v256();
    h.update(data);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

// ----------------------------------------------------------------
// Formatting
// ----------------------------------------------------------------

// ----------------------------------------------------------------
// ERC-20 ABI helpers
// ----------------------------------------------------------------

/// Encode a `balanceOf(address)` call: `0x70a08231 || pad20(holder)`.
pub(crate) fn encode_erc20_balance_of(holder: &str) -> Result<Vec<u8>, String> {
    let holder_bytes = decode_hex(holder)?;
    if holder_bytes.len() != 20 {
        return Err(format!("invalid EVM holder length: {}", holder_bytes.len()));
    }
    let mut out = Vec::with_capacity(4 + 32);
    out.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]); // balanceOf(address)
    out.extend_from_slice(&[0u8; 12]); // left pad 20-byte address to 32 bytes
    out.extend_from_slice(&holder_bytes);
    Ok(out)
}

/// Encode a `transfer(address,uint256)` call.
pub(crate) fn encode_erc20_transfer(to: &str, amount: u128) -> Result<Vec<u8>, String> {
    let to_bytes = decode_hex(to)?;
    if to_bytes.len() != 20 {
        return Err(format!("invalid EVM to length: {}", to_bytes.len()));
    }
    let mut out = Vec::with_capacity(4 + 32 + 32);
    out.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]); // transfer(address,uint256)
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&to_bytes);

    // 32-byte big-endian amount, left-padded with zeros.
    let mut amount_bytes = [0u8; 32];
    amount_bytes[16..].copy_from_slice(&amount.to_be_bytes());
    out.extend_from_slice(&amount_bytes);
    Ok(out)
}

/// Decode an ABI-encoded `string` return value, or fall back to a
/// `bytes32`-style null-terminated ASCII name (MKR, DAI-era tokens).
pub(crate) fn decode_abi_string_or_bytes32(hex_str: &str) -> Option<String> {
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

/// Derive the Ethereum address (checksum-cased) from an uncompressed public key.
pub fn pubkey_to_eth_address(pubkey_uncompressed: &[u8]) -> Result<String, String> {
    if pubkey_uncompressed.len() != 65 || pubkey_uncompressed[0] != 0x04 {
        return Err("expected 65-byte uncompressed public key".to_string());
    }
    let hash = keccak256(&pubkey_uncompressed[1..]);
    let addr_bytes = &hash[12..]; // last 20 bytes
    Ok(eip55_checksum(addr_bytes))
}

/// EIP-55 mixed-case checksum address.
pub fn eip55_checksum(addr_bytes: &[u8]) -> String {
    let hex = hex::encode(addr_bytes);
    let hash = keccak256(hex.as_bytes());
    let mut result = String::with_capacity(42);
    result.push_str("0x");
    for (i, c) in hex.chars().enumerate() {
        if c.is_ascii_alphabetic() {
            let nibble = (hash[i / 2] >> (if i % 2 == 0 { 4 } else { 0 })) & 0x0f;
            if nibble >= 8 {
                result.push(c.to_ascii_uppercase());
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ----------------------------------------------------------------
// Address validation
// ----------------------------------------------------------------

pub fn validate_evm_address(address: &str) -> bool {
    let s = address.strip_prefix("0x").unwrap_or(address);
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

// ----------------------------------------------------------------
// Chain configurations (factory helpers)
// ----------------------------------------------------------------

pub struct EvmChainConfig {
    pub name: &'static str,
    pub chain_id: u64,
    pub rpc_endpoints: Vec<String>,
    pub explorer_api_base: &'static str,
}

impl EvmChainConfig {
    /// Ethereum mainnet.
    pub fn ethereum(rpc_endpoints: Vec<String>) -> Self {
        Self {
            name: "ethereum",
            chain_id: 1,
            rpc_endpoints,
            explorer_api_base: "https://api.etherscan.io",
        }
    }

    /// Arbitrum One.
    pub fn arbitrum(rpc_endpoints: Vec<String>) -> Self {
        Self {
            name: "arbitrum",
            chain_id: 42161,
            rpc_endpoints,
            explorer_api_base: "https://api.arbiscan.io",
        }
    }

    /// Optimism.
    pub fn optimism(rpc_endpoints: Vec<String>) -> Self {
        Self {
            name: "optimism",
            chain_id: 10,
            rpc_endpoints,
            explorer_api_base: "https://api-optimistic.etherscan.io",
        }
    }

    /// Avalanche C-Chain.
    pub fn avalanche(rpc_endpoints: Vec<String>) -> Self {
        Self {
            name: "avalanche",
            chain_id: 43114,
            rpc_endpoints,
            explorer_api_base: "https://api.snowtrace.io",
        }
    }

    /// Base (Coinbase L2).
    pub fn base(rpc_endpoints: Vec<String>) -> Self {
        Self {
            name: "base",
            chain_id: 8453,
            rpc_endpoints,
            explorer_api_base: "https://api.basescan.org",
        }
    }

    /// Ethereum Classic.
    pub fn ethereum_classic(rpc_endpoints: Vec<String>) -> Self {
        Self {
            name: "ethereum_classic",
            chain_id: 61,
            rpc_endpoints,
            explorer_api_base: "https://blockscout.com/etc/mainnet",
        }
    }
}

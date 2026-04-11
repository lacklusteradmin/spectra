//! EVM-compatible chain client (Ethereum, Arbitrum, Optimism, Avalanche, Base,
//! Hyperliquid, Ethereum Classic, etc.)
//!
//! Implements JSON-RPC over HTTPS using the shared `HttpClient`. Builds and
//! signs EIP-1559 transactions in Rust using secp256k1 + RLP encoding.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

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
        let max_fee_per_gas_wei = 2 * base_fee_wei + priority_fee_wei;
        let estimated_fee_wei = max_fee_per_gas_wei * 21_000;

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
        let nonce = self.fetch_nonce(from_address).await?;
        let fee = self.fetch_fee_estimate().await?;
        let gas_limit = 21_000u64; // plain ETH transfer

        let raw_tx = build_eip1559_tx(
            self.chain_id,
            nonce,
            fee.max_fee_per_gas_wei,
            fee.priority_fee_wei,
            gas_limit,
            to_address,
            value_wei,
            &[],
            private_key_bytes,
        )?;

        let hex_tx = format!("0x{}", hex::encode(&raw_tx));
        let result = self
            .call("eth_sendRawTransaction", json!([hex_tx]))
            .await?;
        let txid = result
            .as_str()
            .ok_or("eth_sendRawTransaction: expected string")?
            .to_string();
        Ok(EvmSendResult { txid })
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
        Ok(EvmSendResult { txid })
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
}

// ----------------------------------------------------------------
// EIP-1559 transaction builder + signer
// ----------------------------------------------------------------

/// Build a signed EIP-1559 (type 2) transaction.
///
/// Returns the raw RLP-encoded transaction bytes, ready to be hex-encoded and
/// broadcast via `eth_sendRawTransaction`.
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

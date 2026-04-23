//! EVM-compatible chain client (Ethereum, Arbitrum, Optimism, Avalanche, Base,
//! Hyperliquid, Ethereum Classic, etc.)
//!
//! Implements JSON-RPC over HTTPS using the shared `HttpClient`. Builds and
//! signs EIP-1559 transactions in Rust using secp256k1 + RLP encoding.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use fetch::{
    decode_abi_string_or_bytes32, format_ether, format_evm_decimals, format_token_amount,
};
pub use send::{build_eip1559_tx, EvmSendOverrides};

// ----------------------------------------------------------------
// Internal helpers shared by derive/fetch/send
// ----------------------------------------------------------------

/// Strip the `0x` prefix from a hex string and decode to bytes.
pub(super) fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(stripped).map_err(|e| format!("hex decode: {e}"))
}

/// Parse a `0x`-prefixed hex integer (as returned by JSON-RPC) into u128.
pub(super) fn parse_hex_u128(s: &str) -> Result<u128, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u128::from_str_radix(stripped, 16).map_err(|e| format!("hex u128 parse: {e}"))
}

/// Parse a `0x`-prefixed hex integer into u64.
pub(super) fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(stripped, 16).map_err(|e| format!("hex u64 parse: {e}"))
}

/// Build a JSON-RPC 2.0 request body.
fn rpc(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    })
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
    pub(super) endpoints: Vec<String>,
    pub(super) chain_id: u64,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl EvmClient {
    pub fn new(endpoints: Vec<String>, chain_id: u64) -> Self {
        Self {
            endpoints,
            chain_id,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
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

    /// Bump a base fee by +10% (the minimum EIP-1559 replacement rule).
    /// Used by the UI to compute "speed up" / "cancel" suggested fees.
    pub fn bumped_for_replacement(&self, base: u128) -> u128 {
        // base * 110 / 100, saturating.
        base.saturating_mul(110) / 100
    }
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

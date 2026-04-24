//! Tron chain client.
//!
//! Uses the TronGrid / TronScan REST API.
//! Transactions are built using a protobuf-like manual encoding (Tron uses
//! protobuf for its RawData but the on-wire format for transfers is simple).
//! Signing uses secp256k1 with keccak256 (same key derivation as Ethereum,
//! but Tron addresses use Base58Check with version byte 0x41).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronBalance {
    /// SUN (1 TRX = 1_000_000 SUN).
    pub sun: u64,
    pub trx_display: String,
}

/// Unified history entry covering both native TRX and TRC-20 token transfers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronTransfer {
    pub txid: String,
    pub block_number: u64,
    /// Milliseconds since epoch (TronScan convention).
    pub timestamp_ms: u64,
    pub from: String,
    pub to: String,
    /// Human-readable amount string ("1.5", "10.0", …).
    pub amount_display: String,
    /// "TRX" for native, token abbreviation (e.g. "USDT") for TRC-20.
    pub symbol: String,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronSendResult {
    pub txid: String,
    /// Full signed transaction JSON for rebroadcast. Serialized as a JSON string.
    #[serde(default)]
    pub signed_tx_json: String,
}

/// TRC-20 balance payload. Mirrors `Erc20Balance` so the Swift-side decoder
/// can share a single response type if desired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Balance {
    pub contract: String,
    pub holder: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    pub symbol: String,
}

/// Lightweight TRC-20 metadata (symbol + decimals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Metadata {
    pub symbol: String,
    pub decimals: u8,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct TronClient {
    pub(crate) endpoints: Vec<String>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl TronClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let path = path.to_string();
        let body = body.clone();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = body.clone();
            async move { client.post_json(&url, &body, RetryProfile::ChainRead).await }
        })
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn get_json_path<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}
// Tron fetch paths: balance, latest block, unified TRX+TRC-20 history,
// TRC-20 balance, TRC-20 metadata.

use serde_json::json;


use crate::derivation::chains::tron::tron_base58_to_evm_hex;

impl TronClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<TronBalance, String> {
        let resp = self
            .post("/wallet/getaccount", &json!({"address": address, "visible": true}))
            .await?;
        let sun = resp.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(TronBalance {
            sun,
            trx_display: format_trx(sun),
        })
    }

    pub async fn fetch_latest_block(&self) -> Result<(u64, String), String> {
        let resp = self.post("/wallet/getnowblock", &json!({})).await?;
        let block_num = resp
            .pointer("/block_header/raw_data/number")
            .and_then(|v| v.as_u64())
            .ok_or("getnowblock: missing number")?;
        let _timestamp = resp
            .pointer("/block_header/raw_data/timestamp")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        // blockID is the hash; use its hex string.
        let block_hash = resp
            .get("blockID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok((block_num, block_hash))
    }

    /// Fetch up to `limit` recent transfers combining native TRX and TRC-20
    /// token transfers from TronScan. Results are sorted newest-first.
    pub async fn fetch_unified_history(
        &self,
        address: &str,
        api_base: &str,
        limit: usize,
    ) -> Result<Vec<TronTransfer>, String> {
        let limit = limit.min(50);

        // --- Native TRX transfers ---
        let trx_url = format!(
            "{}/api/transaction?sort=-timestamp&count=true&limit={}&address={}",
            api_base.trim_end_matches('/'),
            limit,
            address
        );
        let trx_resp: Value = self
            .client
            .get_json(&trx_url, RetryProfile::ChainRead)
            .await
            .unwrap_or(Value::Null);
        let trx_data = trx_resp
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut entries: Vec<TronTransfer> = trx_data
            .into_iter()
            .filter_map(|tx| {
                // Only include Transfer (contractType 1) transactions.
                let contract_type = tx.get("contractType").and_then(|v| v.as_u64()).unwrap_or(0);
                if contract_type != 1 {
                    return None;
                }
                let txid = tx.get("hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let block_number = tx.get("block").and_then(|v| v.as_u64()).unwrap_or(0);
                let timestamp_ms = tx.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
                let from = tx
                    .pointer("/contractData/owner_address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let to = tx
                    .pointer("/contractData/to_address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let amount_sun = tx
                    .pointer("/contractData/amount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let is_incoming = to.eq_ignore_ascii_case(address);
                let trx = amount_sun as f64 / 1_000_000.0;
                let amount_display = format_trx_f64(trx);
                Some(TronTransfer {
                    txid,
                    block_number,
                    timestamp_ms,
                    from,
                    to,
                    amount_display,
                    symbol: "TRX".to_string(),
                    is_incoming,
                })
            })
            .collect();

        // --- TRC-20 token transfers ---
        let trc20_url = format!(
            "{}/api/token_trc20/transfers?limit={}&start=0&sort=-timestamp&address={}",
            api_base.trim_end_matches('/'),
            limit,
            address
        );
        let trc20_resp: Value = self
            .client
            .get_json(&trc20_url, RetryProfile::ChainRead)
            .await
            .unwrap_or(Value::Null);
        let trc20_data = trc20_resp
            .get("token_transfers")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for tx in trc20_data {
            let txid = tx
                .get("transaction_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let block_number = tx.get("block").and_then(|v| v.as_u64()).unwrap_or(0);
            let timestamp_ms = tx.get("block_ts").and_then(|v| v.as_u64()).unwrap_or(0);
            let from = tx
                .get("from_address")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let to = tx
                .get("to_address")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_incoming = to.eq_ignore_ascii_case(address);
            let symbol = tx
                .pointer("/tokenInfo/tokenAbbr")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let decimals = tx
                .pointer("/tokenInfo/tokenDecimal")
                .and_then(|v| v.as_u64())
                .unwrap_or(6) as u32;
            let quant_str = tx.get("quant").and_then(|v| v.as_str()).unwrap_or("0");
            let quant: u128 = quant_str.parse().unwrap_or(0);
            let divisor = 10u128.pow(decimals);
            let whole = quant / divisor;
            let frac = quant % divisor;
            let amount_display = if frac == 0 || decimals == 0 {
                whole.to_string()
            } else {
                let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
                let trimmed = frac_str.trim_end_matches('0');
                format!("{}.{}", whole, trimmed)
            };

            entries.push(TronTransfer {
                txid,
                block_number,
                timestamp_ms,
                from,
                to,
                amount_display,
                symbol,
                is_incoming,
            });
        }

        // Sort newest-first by timestamp.
        entries.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
        entries.truncate(limit);
        Ok(entries)
    }

    /// Fetch a TRC-20 token balance.
    ///
    /// Issues three constant calls (`balanceOf`, `decimals`, `symbol`) against
    /// the provided contract via `/wallet/triggerconstantcontract`.
    pub async fn fetch_trc20_balance(
        &self,
        contract_base58: &str,
        holder_base58: &str,
    ) -> Result<Trc20Balance, String> {
        let raw = self.fetch_trc20_balance_of(contract_base58, holder_base58).await?;
        let metadata = self.fetch_trc20_metadata(contract_base58).await?;
        let balance_display = crate::fetch::chains::evm::format_token_amount(raw, metadata.decimals);
        Ok(Trc20Balance {
            contract: contract_base58.to_string(),
            holder: holder_base58.to_string(),
            balance_raw: raw.to_string(),
            balance_display,
            decimals: metadata.decimals,
            symbol: metadata.symbol,
        })
    }

    /// Raw `balanceOf(holder)` constant call.
    pub async fn fetch_trc20_balance_of(
        &self,
        contract_base58: &str,
        holder_base58: &str,
    ) -> Result<u128, String> {
        // TRC-20 uses the same 4-byte selector as ERC-20, but Tron addresses are
        // passed in their *hex* form (0x41... stripped to the last 20 bytes).
        let holder_hex = tron_base58_to_evm_hex(holder_base58)?;
        let parameter = format!("{:0>64}", holder_hex);

        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": holder_base58,
                    "contract_address": contract_base58,
                    "function_selector": "balanceOf(address)",
                    "parameter": parameter,
                    "visible": true
                }),
            )
            .await?;

        let hex_str = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("triggerconstantcontract balanceOf: missing result")?;

        // The result is a 32-byte big-endian integer hex string.
        parse_hex_u256_low_u128(hex_str)
    }

    /// Fetch token symbol + decimals.
    pub async fn fetch_trc20_metadata(
        &self,
        contract_base58: &str,
    ) -> Result<Trc20Metadata, String> {
        // decimals()
        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": contract_base58,
                    "contract_address": contract_base58,
                    "function_selector": "decimals()",
                    "parameter": "",
                    "visible": true
                }),
            )
            .await?;
        let decimals_hex = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("triggerconstantcontract decimals: missing result")?;
        let decimals = parse_hex_u256_low_u128(decimals_hex)? as u8;

        // symbol()
        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": contract_base58,
                    "contract_address": contract_base58,
                    "function_selector": "symbol()",
                    "parameter": "",
                    "visible": true
                }),
            )
            .await?;
        let symbol_hex = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("triggerconstantcontract symbol: missing result")?;
        let symbol = crate::fetch::chains::evm::decode_abi_string_or_bytes32(symbol_hex)
            .unwrap_or_default();

        Ok(Trc20Metadata { symbol, decimals })
    }
}

// ----------------------------------------------------------------
// TRC-20 helpers
// ----------------------------------------------------------------

/// Parse a 32-byte big-endian hex integer return value (no `0x` prefix) and
/// return its low 128 bits. TRC-20 balances and u256 decimals fit comfortably.
pub(crate) fn parse_hex_u256_low_u128(hex_str: &str) -> Result<u128, String> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    // Take last 32 hex chars (16 bytes = 128 bits). TRC-20 u256s that exceed
    // u128 are vanishingly rare for user balances, but we deliberately truncate
    // high bits rather than error out so the caller still gets *something*.
    let low = if stripped.len() > 32 {
        &stripped[stripped.len() - 32..]
    } else {
        stripped
    };
    u128::from_str_radix(low, 16).map_err(|e| format!("parse u128 hex: {e}"))
}

fn format_trx(sun: u64) -> String {
    let whole = sun / 1_000_000;
    let frac = sun % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

fn format_trx_f64(trx: f64) -> String {
    // Format with up to 6 decimal places, trimming trailing zeros.
    let sun = (trx * 1_000_000.0).round() as u64;
    format_trx(sun)
}

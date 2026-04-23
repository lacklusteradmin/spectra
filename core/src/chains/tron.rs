//! Tron chain client.
//!
//! Uses the TronGrid / TronScan REST API.
//! Transactions are built using a protobuf-like manual encoding (Tron uses
//! protobuf for its RawData but the on-wire format for transfers is simple).
//! Signing uses secp256k1 with keccak256 (same key derivation as Ethereum,
//! but Tron addresses use Base58Check with version byte 0x41).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl TronClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
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
    async fn get_json_path<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

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
            let timestamp_ms = tx
                .get("block_ts")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
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
            let quant_str = tx
                .get("quant")
                .and_then(|v| v.as_str())
                .unwrap_or("0");
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

    /// Create, sign, and broadcast a TRX transfer.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sun: u64,
        private_key_bytes: &[u8],
    ) -> Result<TronSendResult, String> {
        // Step 1: Create unsigned transaction via /wallet/createtransaction.
        let resp = self
            .post(
                "/wallet/createtransaction",
                &json!({
                    "owner_address": from_address,
                    "to_address": to_address,
                    "amount": amount_sun,
                    "visible": true
                }),
            )
            .await?;

        // Extract the raw_data_hex for signing.
        let _raw_data_hex = resp
            .get("raw_data_hex")
            .and_then(|v| v.as_str())
            .ok_or("createtransaction: missing raw_data_hex")?;
        let txid = resp
            .get("txID")
            .and_then(|v| v.as_str())
            .ok_or("createtransaction: missing txID")?
            .to_string();

        // Step 2: Sign txID (which is the sha256 of raw_data).
        let txid_bytes = hex::decode(&txid).map_err(|e| format!("txid hex: {e}"))?;
        let signature = sign_tron_hash(&txid_bytes, private_key_bytes)?;

        // Step 3: Broadcast.
        let mut broadcast_body = resp.clone();
        broadcast_body["signature"] = json!([signature]);
        let signed_tx_json = broadcast_body.to_string();
        let broadcast_resp = self
            .post("/wallet/broadcasttransaction", &broadcast_body)
            .await?;
        let result = broadcast_resp
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !result {
            let msg = broadcast_resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(format!("broadcast failed: {msg}"));
        }
        Ok(TronSendResult { txid, signed_tx_json })
    }

    // ----------------------------------------------------------------
    // TRC-20
    // ----------------------------------------------------------------

    /// Fetch a TRC-20 token balance.
    ///
    /// Issues three constant calls (`balanceOf`, `decimals`, `symbol`) against
    /// the provided contract via `/wallet/triggerconstantcontract`.
    pub async fn fetch_trc20_balance(
        &self,
        contract_base58: &str,
        holder_base58: &str,
    ) -> Result<Trc20Balance, String> {
        let raw = self
            .fetch_trc20_balance_of(contract_base58, holder_base58)
            .await?;
        let metadata = self.fetch_trc20_metadata(contract_base58).await?;
        let balance_display = crate::chains::evm::format_token_amount(raw, metadata.decimals);
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
        let symbol = crate::chains::evm::decode_abi_string_or_bytes32(symbol_hex)
            .unwrap_or_default();

        Ok(Trc20Metadata { symbol, decimals })
    }

    /// Build, sign, and broadcast a TRC-20 `transfer(to, amount)` via
    /// `triggersmartcontract` → `broadcasttransaction`.
    pub async fn sign_and_broadcast_trc20(
        &self,
        from_base58: &str,
        contract_base58: &str,
        to_base58: &str,
        amount_raw: u128,
        fee_limit_sun: u64,
        private_key_bytes: &[u8],
    ) -> Result<TronSendResult, String> {
        let to_hex = tron_base58_to_evm_hex(to_base58)?;
        let to_padded = format!("{:0>64}", to_hex);
        let amount_padded = format!("{:064x}", amount_raw);
        let parameter = format!("{}{}", to_padded, amount_padded);

        let resp = self
            .post(
                "/wallet/triggersmartcontract",
                &json!({
                    "owner_address": from_base58,
                    "contract_address": contract_base58,
                    "function_selector": "transfer(address,uint256)",
                    "parameter": parameter,
                    "fee_limit": fee_limit_sun,
                    "call_value": 0,
                    "visible": true
                }),
            )
            .await?;

        let tx_obj = resp
            .get("transaction")
            .ok_or("triggersmartcontract: missing transaction")?;
        let txid = tx_obj
            .get("txID")
            .and_then(|v| v.as_str())
            .ok_or("triggersmartcontract: missing txID")?
            .to_string();

        // Check for contract execution errors at the trigger step.
        if let Some(result_obj) = resp.get("result") {
            if !result_obj.get("result").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Some(msg) = result_obj.get("message").and_then(|v| v.as_str()) {
                    let decoded = hex::decode(msg).ok()
                        .and_then(|b| String::from_utf8(b).ok())
                        .unwrap_or_else(|| msg.to_string());
                    return Err(format!("trc20 trigger failed: {decoded}"));
                }
            }
        }

        // Sign txID.
        let txid_bytes = hex::decode(&txid).map_err(|e| format!("txid hex: {e}"))?;
        let signature = sign_tron_hash(&txid_bytes, private_key_bytes)?;

        // Attach signature and broadcast.
        let mut broadcast_body = tx_obj.clone();
        broadcast_body["signature"] = json!([signature]);
        let signed_tx_json = broadcast_body.to_string();
        let broadcast_resp = self
            .post("/wallet/broadcasttransaction", &broadcast_body)
            .await?;
        let ok = broadcast_resp
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !ok {
            let msg = broadcast_resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let decoded = hex::decode(msg).ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| msg.to_string());
            return Err(format!("trc20 broadcast failed: {decoded}"));
        }
        Ok(TronSendResult { txid, signed_tx_json })
    }

    /// Broadcast an already-signed transaction given as a JSON string.
    pub async fn broadcast_raw(&self, signed_tx_json: &str) -> Result<TronSendResult, String> {
        let body: serde_json::Value = serde_json::from_str(signed_tx_json)
            .map_err(|e| format!("broadcast_raw: invalid JSON: {e}"))?;
        let txid = body
            .get("txID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let broadcast_resp = self.post("/wallet/broadcasttransaction", &body).await?;
        let ok = broadcast_resp
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !ok {
            let msg = broadcast_resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let decoded = hex::decode(msg).ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| msg.to_string());
            return Err(format!("broadcast failed: {decoded}"));
        }
        Ok(TronSendResult { txid, signed_tx_json: signed_tx_json.to_string() })
    }
}

// ----------------------------------------------------------------
// TRC-20 helpers
// ----------------------------------------------------------------

/// Parse a 32-byte big-endian hex integer return value (no `0x` prefix) and
/// return its low 128 bits. TRC-20 balances and u256 decimals fit comfortably.
fn parse_hex_u256_low_u128(hex_str: &str) -> Result<u128, String> {
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

/// Convert a Tron base58 address (`T…`) to an EVM-style 20-byte hex string
/// (without `0x` prefix, without the Tron `0x41` version byte). This is the
/// format TronGrid expects inside ABI-encoded contract parameters.
pub fn tron_base58_to_evm_hex(address: &str) -> Result<String, String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("base58 decode: {e}"))?;
    if decoded.len() != 21 || decoded[0] != 0x41 {
        return Err(format!("invalid Tron address length/prefix: len={}", decoded.len()));
    }
    Ok(hex::encode(&decoded[1..]))
}

// ----------------------------------------------------------------
// Signing
// ----------------------------------------------------------------

fn sign_tron_hash(hash: &[u8], private_key_bytes: &[u8]) -> Result<String, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg = Message::from_digest_slice(hash).map_err(|e| format!("msg: {e}"))?;
    let (rec_id, sig) = secp
        .sign_ecdsa_recoverable(&msg, &secret_key)
        .serialize_compact();
    let mut out = sig.to_vec();
    out.push(rec_id.to_i32() as u8);
    Ok(hex::encode(&out))
}

// ----------------------------------------------------------------
// Address helpers
// ----------------------------------------------------------------

/// Derive a Tron address from a secp256k1 public key (uncompressed, 65 bytes).
pub fn pubkey_to_tron_address(pubkey_uncompressed: &[u8]) -> Result<String, String> {
    if pubkey_uncompressed.len() != 65 || pubkey_uncompressed[0] != 0x04 {
        return Err("expected 65-byte uncompressed public key".to_string());
    }
    let hash = keccak256(&pubkey_uncompressed[1..]);
    let addr_bytes = &hash[12..]; // last 20 bytes
    let mut versioned = vec![0x41u8]; // Tron mainnet prefix
    versioned.extend_from_slice(addr_bytes);
    Ok(bs58::encode(&versioned).with_check().into_string())
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut h = Keccak::v256();
    h.update(data);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

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

pub fn validate_tron_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && b[0] == 0x41)
        .unwrap_or(false)
}

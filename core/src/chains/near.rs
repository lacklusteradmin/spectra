//! NEAR Protocol chain client.
//!
//! Uses the NEAR JSON-RPC API for balance, nonce, block hash, history,
//! and transaction broadcast.
//! Transactions are BORSH-serialized and signed with Ed25519.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// JSON-RPC helper
// ----------------------------------------------------------------

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": "1", "method": method, "params": params })
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct NearBalance {
    /// yoctoNEAR (1 NEAR = 10^24 yoctoNEAR).
    pub yocto_near: String,
    pub near_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearHistoryEntry {
    pub txid: String,
    pub timestamp_ns: u64,
    pub signer_id: String,
    pub receiver_id: String,
    pub amount_yocto: String,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearSendResult {
    pub txid: String,
    /// Base64-encoded signed transaction — stored for rebroadcast.
    pub signed_tx_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearFtBalance {
    pub contract: String,
    pub holder: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearFtMetadata {
    pub spec: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct NearClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl NearClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

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
                    return Err(format!("near rpc error: {err}"));
                }
                resp.get("result")
                    .cloned()
                    .ok_or_else(|| "missing result".to_string())
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, account_id: &str) -> Result<NearBalance, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_account",
                    "finality": "final",
                    "account_id": account_id
                }),
            )
            .await?;
        let yocto = result
            .get("amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let display = format_near(&yocto);
        Ok(NearBalance {
            yocto_near: yocto,
            near_display: display,
        })
    }

    pub async fn fetch_access_key_nonce(&self, account_id: &str, public_key_b58: &str) -> Result<u64, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_access_key",
                    "finality": "final",
                    "account_id": account_id,
                    "public_key": format!("ed25519:{public_key_b58}")
                }),
            )
            .await?;
        result
            .get("nonce")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "view_access_key: missing nonce".to_string())
    }

    pub async fn fetch_latest_block_hash(&self) -> Result<String, String> {
        let result = self
            .call("block", json!({"finality": "final"}))
            .await?;
        result
            .pointer("/header/hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "block: missing hash".to_string())
    }

    /// Fetch transaction history via NEAR Explorer API (indexer).
    pub async fn fetch_history(
        &self,
        account_id: &str,
        indexer_base: &str,
    ) -> Result<Vec<NearHistoryEntry>, String> {
        let url = format!(
            "{}/accounts/{}/activity?limit=50",
            indexer_base.trim_end_matches('/'),
            account_id
        );
        let items: Vec<Value> = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await
            .unwrap_or_default();

        Ok(items
            .into_iter()
            .map(|item| {
                let txid = item.get("transaction_hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let timestamp_ns: u64 = item
                    .get("block_timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let signer_id = item.get("signer_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let receiver_id = item.get("receiver_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let amount_yocto = item
                    .pointer("/args/deposit")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                let is_incoming = receiver_id == account_id;
                NearHistoryEntry {
                    txid,
                    timestamp_ns,
                    signer_id,
                    receiver_id,
                    amount_yocto,
                    is_incoming,
                }
            })
            .collect())
    }

    /// Sign and broadcast a NEAR Transfer transaction.
    pub async fn sign_and_broadcast(
        &self,
        from_account_id: &str,
        to_account_id: &str,
        yocto_near: u128,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<NearSendResult, String> {
        let public_key_b58 = bs58::encode(public_key_bytes).into_string();
        let nonce = self
            .fetch_access_key_nonce(from_account_id, &public_key_b58)
            .await?
            + 1;
        let block_hash = self.fetch_latest_block_hash().await?;
        let block_hash_bytes = bs58::decode(&block_hash)
            .into_vec()
            .map_err(|e| format!("block hash decode: {e}"))?;
        if block_hash_bytes.len() != 32 {
            return Err("block hash wrong length".to_string());
        }
        let block_hash_arr: [u8; 32] = block_hash_bytes.try_into().unwrap();

        let tx_bytes = build_near_transfer_tx(
            from_account_id,
            public_key_bytes,
            nonce,
            to_account_id,
            yocto_near,
            &block_hash_arr,
            private_key_bytes,
        )?;

        use base64::Engine;
        let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

        let result = self
            .call("broadcast_tx_commit", json!([tx_b64]))
            .await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .ok_or("broadcast: missing hash")?
            .to_string();
        Ok(NearSendResult { txid, signed_tx_b64: tx_b64 })
    }

    /// Rebroadcast a pre-signed transaction (base64-encoded).
    pub async fn broadcast_signed_tx_b64(&self, tx_b64: &str) -> Result<NearSendResult, String> {
        let result = self
            .call("broadcast_tx_commit", json!([tx_b64]))
            .await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(NearSendResult { txid, signed_tx_b64: tx_b64.to_string() })
    }

    // ----------------------------------------------------------------
    // NEP-141 (fungible token) support
    // ----------------------------------------------------------------

    /// Call a view function on `contract` and return its decoded bytes.
    /// `args` is JSON that will be serialized, base64-encoded, and sent as
    /// `args_base64` per the NEAR `call_function` query type.
    async fn view_function(
        &self,
        contract: &str,
        method: &str,
        args: &Value,
    ) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let args_str = serde_json::to_string(args).map_err(|e| format!("args serialize: {e}"))?;
        let args_b64 = base64::engine::general_purpose::STANDARD.encode(args_str.as_bytes());
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "call_function",
                    "finality": "final",
                    "account_id": contract,
                    "method_name": method,
                    "args_base64": args_b64,
                }),
            )
            .await?;
        // `result.result` is a u8 array.
        let bytes = result
            .get("result")
            .and_then(|v| v.as_array())
            .ok_or("view_function: missing result bytes")?
            .iter()
            .filter_map(|n| n.as_u64().map(|n| n as u8))
            .collect::<Vec<u8>>();
        Ok(bytes)
    }

    pub async fn fetch_ft_balance_of(
        &self,
        contract: &str,
        account_id: &str,
    ) -> Result<u128, String> {
        let bytes = self
            .view_function(contract, "ft_balance_of", &json!({ "account_id": account_id }))
            .await?;
        // Response body is a JSON string like `"1000000"`.
        let s: String = serde_json::from_slice(&bytes)
            .map_err(|e| format!("ft_balance_of decode: {e}"))?;
        s.parse::<u128>()
            .map_err(|e| format!("ft_balance_of parse: {e}"))
    }

    pub async fn fetch_ft_metadata(&self, contract: &str) -> Result<NearFtMetadata, String> {
        let bytes = self.view_function(contract, "ft_metadata", &json!({})).await?;
        #[derive(Deserialize)]
        struct RawMeta {
            spec: String,
            name: String,
            symbol: String,
            decimals: u8,
        }
        let meta: RawMeta = serde_json::from_slice(&bytes)
            .map_err(|e| format!("ft_metadata decode: {e}"))?;
        Ok(NearFtMetadata {
            spec: meta.spec,
            name: meta.name,
            symbol: meta.symbol,
            decimals: meta.decimals,
        })
    }

    pub async fn fetch_ft_balance(
        &self,
        contract: &str,
        holder: &str,
    ) -> Result<NearFtBalance, String> {
        let raw = self.fetch_ft_balance_of(contract, holder).await?;
        let meta = self.fetch_ft_metadata(contract).await?;
        Ok(NearFtBalance {
            contract: contract.to_string(),
            holder: holder.to_string(),
            balance_raw: raw.to_string(),
            balance_display: format_ft_amount(raw, meta.decimals),
            decimals: meta.decimals,
            symbol: meta.symbol,
        })
    }

    /// Sign and broadcast a NEP-141 `ft_transfer` call.
    ///
    /// Gas defaults to 30 TGas; `deposit` is the NEP-141-required exactly-1
    /// yoctoNEAR. The receiver must already have a storage deposit on the
    /// token contract — Spectra does not auto-register.
    pub async fn sign_and_broadcast_ft_transfer(
        &self,
        from_account_id: &str,
        token_contract: &str,
        to_account_id: &str,
        amount_raw: u128,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<NearSendResult, String> {
        let public_key_b58 = bs58::encode(public_key_bytes).into_string();
        let nonce = self
            .fetch_access_key_nonce(from_account_id, &public_key_b58)
            .await?
            + 1;
        let block_hash = self.fetch_latest_block_hash().await?;
        let block_hash_bytes = bs58::decode(&block_hash)
            .into_vec()
            .map_err(|e| format!("block hash decode: {e}"))?;
        if block_hash_bytes.len() != 32 {
            return Err("block hash wrong length".to_string());
        }
        let block_hash_arr: [u8; 32] = block_hash_bytes.try_into().unwrap();

        let args = json!({
            "receiver_id": to_account_id,
            "amount": amount_raw.to_string(),
        });
        let args_bytes = serde_json::to_vec(&args)
            .map_err(|e| format!("args serialize: {e}"))?;

        let tx_bytes = build_near_function_call_tx(
            from_account_id,
            public_key_bytes,
            nonce,
            token_contract,
            "ft_transfer",
            &args_bytes,
            30_000_000_000_000,   // 30 TGas
            1u128,                 // exactly-1 yoctoNEAR (NEP-141 requirement)
            &block_hash_arr,
            private_key_bytes,
        )?;

        use base64::Engine;
        let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);
        let result = self.call("broadcast_tx_commit", json!([tx_b64])).await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .ok_or("broadcast: missing hash")?
            .to_string();
        Ok(NearSendResult { txid, signed_tx_b64: tx_b64 })
    }
}

// ----------------------------------------------------------------
// NEAR transaction builder (BORSH)
// ----------------------------------------------------------------

/// Build a signed NEAR Transfer transaction.
pub fn build_near_transfer_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    // BORSH-encode the transaction.
    let tx = borsh_encode_transfer(signer_id, public_key, nonce, receiver_id, yocto_amount, block_hash);

    // Hash the transaction for signing.
    let tx_hash: [u8; 32] = Sha256::digest(&tx).into();

    let signing_key = SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature
    // Signature in NEAR is: [key_type(4)] + [sig(64)]
    let mut signed = tx;
    signed.extend_from_slice(&0u32.to_le_bytes()); // key type = ED25519
    signed.extend_from_slice(signature.to_bytes().as_ref());

    Ok(signed)
}

/// BORSH-encode a NEAR Transfer transaction.
fn borsh_encode_transfer(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();

    // signer_id: string (u32 len + bytes)
    borsh_string(&mut out, signer_id);
    // public_key: key_type(u32) + bytes(32)
    out.extend_from_slice(&0u32.to_le_bytes()); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes()); // 1 action
    // Action::Transfer = variant 3
    out.push(3u8);
    // Transfer.deposit: u128
    out.extend_from_slice(&yocto_amount.to_le_bytes());

    out
}

/// Build a signed NEAR FunctionCall transaction (used for NEP-141 transfers).
#[allow(clippy::too_many_arguments)]
pub fn build_near_function_call_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    method_name: &str,
    args: &[u8],
    gas: u64,
    deposit: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let tx = borsh_encode_function_call(
        signer_id,
        public_key,
        nonce,
        receiver_id,
        method_name,
        args,
        gas,
        deposit,
        block_hash,
    );

    let tx_hash: [u8; 32] = Sha256::digest(&tx).into();
    let signing_key =
        SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature (key_type(4) + sig(64))
    let mut signed = tx;
    signed.extend_from_slice(&0u32.to_le_bytes()); // ED25519
    signed.extend_from_slice(signature.to_bytes().as_ref());
    Ok(signed)
}

#[allow(clippy::too_many_arguments)]
fn borsh_encode_function_call(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    method_name: &str,
    args: &[u8],
    gas: u64,
    deposit: u128,
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();

    // signer_id: string
    borsh_string(&mut out, signer_id);
    // public_key: key_type(u32) + bytes(32)
    out.extend_from_slice(&0u32.to_le_bytes()); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes());
    // Action::FunctionCall = variant 2
    out.push(2u8);
    // method_name: string
    borsh_string(&mut out, method_name);
    // args: Vec<u8> (u32 len + bytes)
    out.extend_from_slice(&(args.len() as u32).to_le_bytes());
    out.extend_from_slice(args);
    // gas: u64
    out.extend_from_slice(&gas.to_le_bytes());
    // deposit: u128
    out.extend_from_slice(&deposit.to_le_bytes());

    out
}

fn borsh_string(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_near(yocto: &str) -> String {
    // yocto is a 25-digit decimal; divide by 10^24 for NEAR.
    let n: u128 = yocto.parse().unwrap_or(0);
    let divisor: u128 = 1_000_000_000_000_000_000_000_000; // 10^24
    let whole = n / divisor;
    let frac = n % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:024}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

/// Format a fungible-token raw amount using its `decimals`, up to 6
/// fractional digits of display precision.
fn format_ft_amount(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let divisor: u128 = 10u128.pow(decimals as u32);
    let whole = raw / divisor;
    let frac = raw % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

pub fn validate_near_address(address: &str) -> bool {
    // NEAR accounts: named (alice.near, sub.alice.near) or implicit (64 hex chars).
    if address.len() == 64 && address.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    // Named account: 2-64 chars, alphanumeric, hyphen, underscore, dot.
    !address.is_empty()
        && address.len() <= 64
        && address
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

// ---------------------------------------------------------------
// History response parser (Swift calls this directly via UniFFI to
// decode raw NEAR indexer JSON into typed snapshots).
// ---------------------------------------------------------------

#[derive(Debug, Clone, uniffi::Record)]
pub struct NearHistoryParsedSnapshot {
    pub transaction_hash: String,
    /// "send" or "receive"
    pub kind: String,
    pub amount_near: f64,
    pub counterparty_address: String,
    /// Unix seconds (0 = fall back to "now" on the Swift side).
    pub created_at_unix_seconds: f64,
}

fn history_rows(value: &Value) -> Vec<Value> {
    if let Some(arr) = value.as_array() {
        return arr.clone();
    }
    let Some(obj) = value.as_object() else { return Vec::new(); };
    for key in ["txns", "transactions", "data", "result"] {
        if let Some(Value::Array(arr)) = obj.get(key) {
            return arr.clone();
        }
    }
    Vec::new()
}

fn history_string_value(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            if let Some(n) = v.as_i64() { return Some(n.to_string()); }
            if let Some(n) = v.as_u64() { return Some(n.to_string()); }
            if let Some(n) = v.as_f64() { return Some(n.to_string()); }
        }
    }
    None
}

fn history_deposit_text(row: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(s) = history_string_value(row, &["deposit", "amount"]) {
        if !s.is_empty() { return Some(s); }
    }
    if let Some(Value::Object(agg)) = row.get("actions_agg") {
        if let Some(s) = history_string_value(agg, &["deposit", "total_deposit", "amount"]) {
            if !s.is_empty() { return Some(s); }
        }
    }
    if let Some(Value::Array(actions)) = row.get("actions") {
        for action in actions {
            if let Some(action_obj) = action.as_object() {
                if let Some(s) = history_string_value(action_obj, &["deposit", "amount"]) {
                    if !s.is_empty() { return Some(s); }
                }
                if let Some(Value::Object(args)) = action_obj.get("args") {
                    if let Some(s) = history_string_value(args, &["deposit", "amount"]) {
                        if !s.is_empty() { return Some(s); }
                    }
                }
            }
        }
    }
    None
}

fn history_numeric_timestamp(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(n) = v.as_f64() { return Some(n); }
            if let Some(s) = v.as_str() { if let Ok(n) = s.parse::<f64>() { return Some(n); } }
        }
    }
    None
}

fn history_timestamp_seconds(row: &serde_json::Map<String, Value>) -> Option<f64> {
    let pick = |t: f64| -> Option<f64> {
        if t <= 0.0 { return None; }
        if t >= 1_000_000_000_000_000.0 { return Some(t / 1_000_000_000.0); }
        if t >= 1_000_000_000_000.0 { return Some(t / 1_000.0); }
        Some(t)
    };
    if let Some(t) = history_numeric_timestamp(row, &["block_timestamp", "timestamp", "included_in_block_timestamp"]) {
        if let Some(s) = pick(t) { return Some(s); }
    }
    for nested_key in ["block", "receipt_block", "included_in_block", "receipt"] {
        if let Some(Value::Object(nested)) = row.get(nested_key) {
            if let Some(t) = history_numeric_timestamp(nested, &["block_timestamp", "timestamp"]) {
                if let Some(s) = pick(t) { return Some(s); }
            }
        }
    }
    None
}

fn yocto_to_near(yocto: &str) -> f64 {
    // NEAR has 24 decimals. Use string manipulation to avoid u128 overflow for
    // pathological inputs.
    let s = yocto.trim();
    if s.is_empty() { return 0.0; }
    if let Ok(v) = s.parse::<u128>() {
        return v as f64 / 1e24;
    }
    if s.len() <= 24 {
        let padded = format!("{:0>24}", s);
        let frac = format!("0.{}", padded);
        return frac.parse::<f64>().unwrap_or(0.0);
    }
    let (int_part, frac_part) = s.split_at(s.len() - 24);
    format!("{}.{}", int_part, frac_part).parse::<f64>().unwrap_or(0.0)
}

#[uniffi::export]
pub fn near_parse_history_response(json: String, owner_address: String) -> Vec<NearHistoryParsedSnapshot> {
    let Ok(root): Result<Value, _> = serde_json::from_str(&json) else { return Vec::new(); };
    let owner = owner_address.trim().to_lowercase();
    history_rows(&root)
        .into_iter()
        .filter_map(|row| {
            let row_obj = row.as_object()?.clone();
            let hash = history_string_value(&row_obj, &["transaction_hash", "hash", "receipt_id"])?;
            if hash.is_empty() { return None; }
            let signer = history_string_value(&row_obj, &["signer_account_id", "predecessor_account_id", "signer_id", "signer"])
                .unwrap_or_default().trim().to_lowercase();
            let receiver = history_string_value(&row_obj, &["receiver_account_id", "receiver_id", "receiver"])
                .unwrap_or_default().trim().to_lowercase();
            let (kind, counterparty) = if signer == owner {
                ("send".to_string(), receiver)
            } else if receiver == owner {
                ("receive".to_string(), signer)
            } else if !signer.is_empty() {
                ("receive".to_string(), signer)
            } else {
                ("send".to_string(), receiver)
            };
            let yocto = history_deposit_text(&row_obj).unwrap_or_else(|| "0".to_string());
            let amount = yocto_to_near(&yocto);
            let created = history_timestamp_seconds(&row_obj).unwrap_or(0.0);
            Some(NearHistoryParsedSnapshot {
                transaction_hash: hash,
                kind,
                amount_near: amount,
                counterparty_address: counterparty,
                created_at_unix_seconds: created,
            })
        })
        .collect()
}

#[cfg(test)]
mod near_history_tests {
    use super::*;

    #[test]
    fn near_history_send_and_receive() {
        let json = r#"{"txns":[
            {"transaction_hash":"a","signer_id":"alice.near","receiver_id":"bob.near","deposit":"1000000000000000000000000","block_timestamp":"1700000000000000000"},
            {"transaction_hash":"b","signer_id":"bob.near","receiver_id":"alice.near","actions_agg":{"deposit":"2000000000000000000000000"},"block_timestamp":"1700000001000000000"}
        ]}"#;
        let out = near_parse_history_response(json.into(), "alice.near".into());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, "send");
        assert!((out[0].amount_near - 1.0).abs() < 1e-9);
        assert_eq!(out[0].counterparty_address, "bob.near");
        assert_eq!(out[1].kind, "receive");
        assert!((out[1].amount_near - 2.0).abs() < 1e-9);
    }

    #[test]
    fn near_history_empty_on_garbage() {
        assert!(near_parse_history_response("not json".into(), "alice".into()).is_empty());
        assert!(near_parse_history_response("{}".into(), "alice".into()).is_empty());
    }
}

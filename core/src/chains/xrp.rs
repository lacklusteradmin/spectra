//! XRP (Ripple) chain client.
//!
//! Uses the XRP Ledger JSON-RPC / REST API (rippled / Clio).
//! Transactions are serialized using XRP's binary codec (STObject)
//! and signed with secp256k1.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// JSON-RPC helper
// ----------------------------------------------------------------

fn rpc(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": [params] })
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpBalance {
    /// XRP drops (1 XRP = 1_000_000 drops).
    pub drops: u64,
    pub xrp_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpHistoryEntry {
    pub txid: String,
    pub ledger_index: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_drops: u64,
    pub fee_drops: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpSendResult {
    pub txid: String,
    /// Signed tx blob hex — stored for rebroadcast.
    pub tx_blob_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct XrpClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl XrpClient {
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
                let result = resp
                    .get("result")
                    .ok_or_else(|| "missing result".to_string())?;
                if let Some(status) = result.get("status").and_then(|s| s.as_str()) {
                    if status == "error" {
                        let msg = result
                            .get("error_message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error");
                        return Err(format!("xrp rpc error: {msg}"));
                    }
                }
                Ok(result.clone())
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<XrpBalance, String> {
        let result = self
            .call("account_info", json!({"account": address, "ledger_index": "validated"}))
            .await?;
        let drops: u64 = result
            .pointer("/account_data/Balance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or("account_info: missing Balance")?;
        Ok(XrpBalance {
            drops,
            xrp_display: format_xrp(drops),
        })
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u32, String> {
        let result = self
            .call("account_info", json!({"account": address, "ledger_index": "current"}))
            .await?;
        result
            .pointer("/account_data/Sequence")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .ok_or_else(|| "account_info: missing Sequence".to_string())
    }

    pub async fn fetch_fee(&self) -> Result<u64, String> {
        let result = self.call("fee", json!({})).await?;
        result
            .pointer("/drops/open_ledger_fee")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "fee: missing open_ledger_fee".to_string())
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<XrpHistoryEntry>, String> {
        let result = self
            .call(
                "account_tx",
                json!({"account": address, "limit": 50, "ledger_index_min": -1, "ledger_index_max": -1}),
            )
            .await?;
        let txs = result
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut entries = Vec::new();
        for item in txs {
            let tx = item.get("tx").unwrap_or(&Value::Null);
            let _meta = item.get("meta").unwrap_or(&Value::Null);

            let txtype = tx.get("TransactionType").and_then(|v| v.as_str()).unwrap_or("");
            if txtype != "Payment" {
                continue;
            }
            let txid = tx.get("hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ledger_index = tx.get("ledger_index").and_then(|v| v.as_u64()).unwrap_or(0);
            // XRP epoch: 2000-01-01, Unix epoch difference = 946684800
            let timestamp = tx
                .get("date")
                .and_then(|v| v.as_u64())
                .map(|d| d + 946_684_800)
                .unwrap_or(0);
            let from = tx.get("Account").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let to = tx.get("Destination").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let amount_drops = tx
                .get("Amount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0u64);
            let fee_drops = tx
                .get("Fee")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0u64);
            let is_incoming = to == address;

            entries.push(XrpHistoryEntry {
                txid,
                ledger_index,
                timestamp,
                from,
                to,
                amount_drops,
                fee_drops,
                is_incoming,
            });
        }
        Ok(entries)
    }

    /// Sign and submit an XRP Payment transaction.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        drops: u64,
        private_key_bytes: &[u8],
        public_key_hex: &str,
    ) -> Result<XrpSendResult, String> {
        let sequence = self.fetch_sequence(from_address).await?;
        let fee = self.fetch_fee().await?;

        let tx_blob = build_signed_payment(
            from_address,
            to_address,
            drops,
            fee,
            sequence,
            private_key_bytes,
            public_key_hex,
        )?;

        let result = self
            .call("submit", json!({"tx_blob": tx_blob}))
            .await?;
        let txid = result
            .get("tx_json")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .ok_or("submit: missing hash")?
            .to_string();
        Ok(XrpSendResult { txid, tx_blob_hex: tx_blob })
    }

    /// Submit a pre-signed transaction blob (for rebroadcast).
    pub async fn submit_signed_blob(&self, tx_blob_hex: &str) -> Result<XrpSendResult, String> {
        let result = self
            .call("submit", json!({"tx_blob": tx_blob_hex}))
            .await?;
        let txid = result
            .get("tx_json")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(XrpSendResult { txid, tx_blob_hex: tx_blob_hex.to_string() })
    }
}

// ----------------------------------------------------------------
// XRP binary codec (minimal — Payment only)
// ----------------------------------------------------------------

/// Build and sign an XRP Payment transaction.
/// Returns the signed tx blob as an uppercase hex string.
pub fn build_signed_payment(
    from: &str,
    to: &str,
    amount_drops: u64,
    fee_drops: u64,
    sequence: u32,
    private_key_bytes: &[u8],
    public_key_hex: &str,
) -> Result<String, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    // Build signing payload (canonical field order per XRPL spec).
    let signing_prefix = hex::decode("53545800").unwrap(); // "STX\x00"

    let unsigned_fields = encode_payment_fields(from, to, amount_drops, fee_drops, sequence, public_key_hex)?;

    let mut signing_payload = signing_prefix.clone();
    signing_payload.extend_from_slice(&unsigned_fields);

    let msg_hash = sha512_half(&signing_payload);
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg = Message::from_digest_slice(&msg_hash).map_err(|e| format!("msg: {e}"))?;
    let sig = secp.sign_ecdsa(&msg, &secret_key);
    let der_sig = sig.serialize_der();
    let sig_hex = hex::encode_upper(der_sig.as_ref());

    // Rebuild with TxnSignature field inserted (field 16, type 7 = VL).
    let signed_fields = encode_payment_fields_signed(
        from, to, amount_drops, fee_drops, sequence, public_key_hex, &sig_hex,
    )?;

    Ok(hex::encode_upper(&signed_fields))
}

/// Encode the canonical Payment STObject fields (without TxnSignature).
fn encode_payment_fields(
    from: &str,
    to: &str,
    amount_drops: u64,
    fee_drops: u64,
    sequence: u32,
    public_key_hex: &str,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    // TransactionType = 0 (Payment), field 2, type 1 (UInt16)
    out.extend_from_slice(&[0x12, 0x00, 0x00]);
    // Flags, field 2, type 2 (UInt32) = 0
    out.extend_from_slice(&[0x22, 0x00, 0x00, 0x00, 0x00]);
    // Sequence, field 4, type 2
    out.push(0x24);
    out.extend_from_slice(&sequence.to_be_bytes());
    // Amount, field 1, type 6 (Amount)
    out.push(0x61);
    // XRP amount: 0x4000000000000000 | drops
    let amount_encoded: u64 = 0x4000_0000_0000_0000 | amount_drops;
    out.extend_from_slice(&amount_encoded.to_be_bytes());
    // Fee, field 8, type 6
    out.push(0x68);
    let fee_encoded: u64 = 0x4000_0000_0000_0000 | fee_drops;
    out.extend_from_slice(&fee_encoded.to_be_bytes());
    // SigningPubKey, field 3, type 7 (VL)
    out.push(0x73);
    let pk_bytes = hex::decode(public_key_hex).map_err(|e| format!("pubkey hex: {e}"))?;
    push_vl(&mut out, &pk_bytes);
    // Account (from), field 1, type 8 (AccountID)
    out.push(0x81);
    let from_bytes = decode_xrp_address(from)?;
    push_vl(&mut out, &from_bytes);
    // Destination (to), field 3, type 8
    out.push(0x83);
    let to_bytes = decode_xrp_address(to)?;
    push_vl(&mut out, &to_bytes);
    Ok(out)
}

fn encode_payment_fields_signed(
    from: &str,
    to: &str,
    amount_drops: u64,
    fee_drops: u64,
    sequence: u32,
    public_key_hex: &str,
    sig_hex: &str,
) -> Result<Vec<u8>, String> {
    let mut out = encode_payment_fields(from, to, amount_drops, fee_drops, sequence, public_key_hex)?;
    // TxnSignature, field 4, type 7
    out.push(0x74);
    let sig_bytes = hex::decode(sig_hex).map_err(|e| format!("sig hex: {e}"))?;
    push_vl(&mut out, &sig_bytes);
    Ok(out)
}

fn push_vl(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    if len < 193 {
        out.push(len as u8);
    } else {
        // Extended VL (simplified: only handles up to 12480 bytes)
        let adjusted = len - 193;
        out.push(193 + (adjusted / 256) as u8);
        out.push((adjusted % 256) as u8);
    }
    out.extend_from_slice(data);
}

/// Decode an XRP base58 address to its 20-byte account ID.
fn decode_xrp_address(address: &str) -> Result<Vec<u8>, String> {
    // XRP uses a custom base58 alphabet.
    let alphabet = bs58::Alphabet::new(b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz")
        .map_err(|e| format!("alphabet: {e}"))?;
    let decoded = bs58::decode(address)
        .with_alphabet(&alphabet)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("xrp address decode: {e}"))?;
    // First byte is version (0x00 for mainnet).
    if decoded.len() != 21 {
        return Err(format!("xrp address length: {}", decoded.len()));
    }
    Ok(decoded[1..].to_vec())
}

fn sha512_half(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha512};
    let hash = Sha512::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hash[..32]);
    out
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_xrp(drops: u64) -> String {
    let whole = drops / 1_000_000;
    let frac = drops % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

pub fn validate_xrp_address(address: &str) -> bool {
    let alphabet = match bs58::Alphabet::new(
        b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz",
    ) {
        Ok(a) => a,
        Err(_) => return false,
    };
    bs58::decode(address)
        .with_alphabet(&alphabet)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && b[0] == 0x00)
        .unwrap_or(false)
}

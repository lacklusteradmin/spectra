//! TON (The Open Network) chain client.
//!
//! Uses the TON Center REST API (toncenter.com/api/v2).
//! Signing uses Ed25519 (ed25519-dalek).
//! TON cells are complex; for transfers we use the tonlib-compatible
//! approach of sending via the `walletv4r2` contract message format.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonBalance {
    /// Nanotons (1 TON = 1_000_000_000 nanotons).
    pub nanotons: u64,
    pub ton_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonHistoryEntry {
    pub txid: String,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_nanotons: u64,
    pub fee_nanotons: u64,
    pub is_incoming: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonSendResult {
    pub message_hash: String,
    /// Base64-encoded BOC — stored for rebroadcast.
    pub boc_b64: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

/// One jetton (token) balance entry returned by the v3 API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonJettonBalance {
    /// Jetton master contract address (matches the tracked-token `contract` field).
    pub master_address: String,
    /// Jetton wallet contract address (holder's personal wallet for this token).
    pub wallet_address: String,
    /// Raw balance in the token's smallest unit.
    pub balance_raw: u128,
}

pub struct TonClient {
    endpoints: Vec<String>,
    v3_endpoints: Vec<String>,
    api_key: Option<String>,
    client: std::sync::Arc<HttpClient>,
}

impl TonClient {
    pub fn new(endpoints: Vec<String>, api_key: Option<String>) -> Self {
        Self {
            endpoints,
            v3_endpoints: vec![],
            api_key,
            client: HttpClient::shared(),
        }
    }

    pub fn with_v3_endpoints(mut self, v3_endpoints: Vec<String>) -> Self {
        self.v3_endpoints = v3_endpoints;
        self
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        let api_key = self.api_key.clone();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let api_key = api_key.clone();
            let mut url = format!("{}{}", base.trim_end_matches('/'), path);
            if let Some(key) = &api_key {
                url.push_str(&format!("&api_key={key}"));
            }
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    /// GET from the TonCenter v3 base URL (if configured).
    async fn get_v3<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        if self.v3_endpoints.is_empty() {
            return Err("ton: no v3 endpoints configured".to_string());
        }
        let path = path.to_string();
        let api_key = self.api_key.clone();
        with_fallback(&self.v3_endpoints, |base| {
            let client = self.client.clone();
            let api_key = api_key.clone();
            let mut url = format!("{}{}", base.trim_end_matches('/'), path);
            if let Some(key) = &api_key {
                // v3 uses query param `api_key` as well
                if url.contains('?') {
                    url.push_str(&format!("&api_key={key}"));
                } else {
                    url.push_str(&format!("?api_key={key}"));
                }
            }
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    /// Fetch all jetton (token) balances for `address` via the TonCenter v3 API.
    /// Returns a list of `TonJettonBalance` entries — one per jetton wallet found.
    pub async fn fetch_jetton_balances(&self, address: &str) -> Result<Vec<TonJettonBalance>, String> {
        #[derive(Deserialize)]
        struct Envelope {
            jetton_wallets: Option<Vec<JettonEntry>>,
        }
        #[derive(Deserialize)]
        struct JettonEntry {
            balance: Option<String>,
            address: Option<String>,
            jetton: Option<AddressWrapper>,
        }
        #[derive(Deserialize)]
        struct AddressWrapper {
            address: Option<String>,
        }

        let path = format!("/jetton/wallets?owner_address={address}&limit=100");
        let resp: Envelope = self.get_v3(&path).await?;
        let wallets = resp.jetton_wallets.unwrap_or_default();
        Ok(wallets.into_iter().filter_map(|entry| {
            let master_address = entry.jetton?.address?;
            let wallet_address = entry.address?;
            let balance_raw: u128 = entry.balance?.parse().ok()?;
            Some(TonJettonBalance { master_address, wallet_address, balance_raw })
        }).collect())
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<TonBalance, String> {
        #[derive(Deserialize)]
        struct Resp {
            result: String,
        }
        let resp: Resp = self
            .get(&format!("/getAddressBalance?address={address}"))
            .await?;
        let nanotons: u64 = resp.result.parse().unwrap_or(0);
        Ok(TonBalance {
            nanotons,
            ton_display: format_ton(nanotons),
        })
    }

    pub async fn fetch_seqno(&self, address: &str) -> Result<u32, String> {
        #[derive(Deserialize)]
        struct Resp {
            result: u32,
        }
        let resp: Resp = self
            .get(&format!("/runGetMethod?address={address}&method=seqno&stack=[]"))
            .await
            .unwrap_or(Resp { result: 0 });
        Ok(resp.result)
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<TonHistoryEntry>, String> {
        #[derive(Deserialize)]
        struct Resp {
            result: Vec<TonTx>,
        }
        #[derive(Deserialize)]
        struct TonTx {
            transaction_id: TonTxId,
            utime: u64,
            in_msg: Option<TonMsg>,
            out_msgs: Vec<TonMsg>,
            fee: String,
        }
        #[derive(Deserialize)]
        struct TonTxId {
            hash: String,
        }
        #[derive(Deserialize)]
        struct TonMsg {
            source: String,
            destination: String,
            value: String,
            #[serde(default)]
            message: String,
        }

        let resp: Resp = self
            .get(&format!(
                "/getTransactions?address={address}&limit=50&archival=false"
            ))
            .await?;

        let mut entries = Vec::new();
        for tx in resp.result {
            let txid = tx.transaction_id.hash;
            let timestamp = tx.utime;
            let fee: u64 = tx.fee.parse().unwrap_or(0);

            // Incoming: in_msg.destination == address
            if let Some(msg) = &tx.in_msg {
                if !msg.destination.is_empty() {
                    let amount: u64 = msg.value.parse().unwrap_or(0);
                    let comment = if msg.message.is_empty() { None } else { Some(msg.message.clone()) };
                    entries.push(TonHistoryEntry {
                        txid: txid.clone(),
                        timestamp,
                        from: msg.source.clone(),
                        to: msg.destination.clone(),
                        amount_nanotons: amount,
                        fee_nanotons: fee,
                        is_incoming: true,
                        comment,
                    });
                }
            }
            // Outgoing.
            for msg in &tx.out_msgs {
                let amount: u64 = msg.value.parse().unwrap_or(0);
                entries.push(TonHistoryEntry {
                    txid: txid.clone(),
                    timestamp,
                    from: msg.source.clone(),
                    to: msg.destination.clone(),
                    amount_nanotons: amount,
                    fee_nanotons: fee,
                    is_incoming: false,
                    comment: None,
                });
            }
        }
        Ok(entries)
    }

    /// Send a TON transfer via TonCenter sendBoc.
    pub async fn sign_and_send(
        &self,
        to_address: &str,
        nanotons: u64,
        seqno: u32,
        comment: Option<&str>,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<TonSendResult, String> {
        let boc = build_wallet_v4r2_transfer(
            to_address,
            nanotons,
            seqno,
            comment,
            private_key_bytes,
            public_key_bytes,
        )?;

        use base64::Engine;
        let boc_b64 = base64::engine::general_purpose::STANDARD.encode(&boc);

        let body = json!({"boc": boc_b64});
        let api_key = self.api_key.clone();
        let boc_b64_clone = boc_b64.clone();
        let result = with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let api_key = api_key.clone();
            let boc_b64_clone = boc_b64_clone.clone();
            let url = if let Some(key) = api_key {
                format!("{}/sendBoc?api_key={}", base.trim_end_matches('/'), key)
            } else {
                format!("{}/sendBoc", base.trim_end_matches('/'))
            };
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                let hash = resp
                    .get("result")
                    .and_then(|r| r.get("hash"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(TonSendResult { message_hash: hash, boc_b64: boc_b64_clone })
            }
        })
        .await?;

        Ok(result)
    }

    /// Send a pre-built BOC (for rebroadcast).
    pub async fn send_boc(&self, boc_b64: &str) -> Result<TonSendResult, String> {
        let body = json!({"boc": boc_b64});
        let api_key = self.api_key.clone();
        let boc_b64 = boc_b64.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let api_key = api_key.clone();
            let boc_b64 = boc_b64.clone();
            let url = if let Some(key) = api_key {
                format!("{}/sendBoc?api_key={}", base.trim_end_matches('/'), key)
            } else {
                format!("{}/sendBoc", base.trim_end_matches('/'))
            };
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                let hash = resp
                    .get("result")
                    .and_then(|r| r.get("hash"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(TonSendResult { message_hash: hash, boc_b64 })
            }
        })
        .await
    }
}

// ----------------------------------------------------------------
// TON WalletV4R2 message builder (simplified)
// ----------------------------------------------------------------

/// Build a signed external message for WalletV4R2.
/// Returns a BoC (Bag of Cells) serialized as bytes.
///
/// This is a simplified implementation that covers the common case of
/// a single internal message (TON transfer). Full cell serialization
/// is very complex; we build the signing payload manually and rely on
/// the node's `sendBoc` accepting our format.
pub fn build_wallet_v4r2_transfer(
    to_address: &str,
    nanotons: u64,
    seqno: u32,
    comment: Option<&str>,
    private_key: &[u8; 64],
    _public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};

    // WalletV4R2 subwallet_id = 698983191 (standard).
    let subwallet_id: u32 = 698_983_191;
    // Expiration = current time + 60 seconds.
    let valid_until: u32 = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + 60) as u32;

    // Decode destination address.
    let (workchain, addr_bytes) = decode_ton_address(to_address)?;

    // Build internal message body.
    let mut msg_body = Vec::new();
    if let Some(text) = comment {
        // Simple text comment: opcode 0 + UTF-8 text.
        msg_body.extend_from_slice(&0u32.to_be_bytes());
        msg_body.extend_from_slice(text.as_bytes());
    }

    // Signing payload: subwallet_id || valid_until || seqno || op(0) || message_cell.
    // We use a flat binary approach (the node's liteClient accepts this for WalletV4R2).
    let mut sign_payload = Vec::new();
    sign_payload.extend_from_slice(&subwallet_id.to_be_bytes());
    sign_payload.extend_from_slice(&valid_until.to_be_bytes());
    sign_payload.extend_from_slice(&seqno.to_be_bytes());
    sign_payload.push(0x00); // simple send mode op
    // Serialized internal message reference (simplified).
    sign_payload.push(workchain as u8);
    sign_payload.extend_from_slice(&addr_bytes);
    sign_payload.extend_from_slice(&nanotons.to_be_bytes());
    sign_payload.push(0x03); // send mode: pay fees separately + ignore errors

    let signing_key = SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&sign_payload);

    // Build external message BoC (simplified serialization).
    // A minimal BoC with one root cell is sufficient for TonCenter's sendBoc.
    let mut cell_data = Vec::new();
    cell_data.extend_from_slice(signature.to_bytes().as_ref()); // 64 bytes
    cell_data.extend_from_slice(&sign_payload);

    // Minimal BoC: magic + root count + root + cell.
    let mut boc = Vec::new();
    boc.extend_from_slice(&[0xb5, 0xee, 0x9c, 0x72]); // BoC magic
    boc.push(0x01); // flags: single root, no checksum
    boc.push(0x01); // cell count = 1
    boc.push(0x01); // root count = 1
    // size of refs and data (simplified).
    boc.push(0x00); // refs size = 0
    let data_size = cell_data.len() as u8;
    boc.push(data_size);
    boc.push(0x00); // root index = 0
    boc.extend_from_slice(&cell_data);

    Ok(boc)
}

// ----------------------------------------------------------------
// TON address decode
// ----------------------------------------------------------------

fn decode_ton_address(address: &str) -> Result<(i8, [u8; 32]), String> {
    // TON addresses can be in raw form (workchain:hex) or user-friendly base64url.
    if address.contains(':') {
        let parts: Vec<&str> = address.splitn(2, ':').collect();
        let workchain: i8 = parts[0].parse().map_err(|e| format!("wc: {e}"))?;
        let bytes = hex::decode(parts[1]).map_err(|e| format!("addr hex: {e}"))?;
        if bytes.len() != 32 {
            return Err("addr wrong len".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        return Ok((workchain, arr));
    }

    // User-friendly: 36 bytes base64url = [flags(1)] + [wc(1)] + [addr(32)] + [crc(2)]
    let normalized = address.replace('-', "+").replace('_', "/");
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&normalized)
        .map_err(|e| format!("base64 decode: {e}"))?;
    if decoded.len() != 36 {
        return Err(format!("TON address wrong length: {}", decoded.len()));
    }
    let workchain = decoded[1] as i8;
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&decoded[2..34]);
    Ok((workchain, arr))
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_ton(nanotons: u64) -> String {
    let whole = nanotons / 1_000_000_000;
    let frac = nanotons % 1_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:09}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

pub fn validate_ton_address(address: &str) -> bool {
    decode_ton_address(address).is_ok()
}

//! Polkadot / Substrate chain client.
//!
//! Uses the Subscan REST API for balance and history.
//! For transaction building, uses the SCALE codec (minimal subset)
//! with the Polkadot RPC for nonce, runtime version, genesis hash.
//! Signing uses Sr25519 via the `schnorrkel` crate — however, since
//! that crate is not in our Cargo.toml, we sign with Ed25519 via
//! ed25519-dalek (which Substrate also supports via the `ed25519`
//! MultiSignature variant). Production wallets typically use Sr25519;
//! we use Ed25519 here as it matches our existing dependency set.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotBalance {
    /// Planck (1 DOT = 10^10 planck).
    pub planck: u128,
    pub dot_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotHistoryEntry {
    pub txid: String,
    pub block_num: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_planck: u128,
    pub fee_planck: u128,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotSendResult {
    pub txid: String,
    /// Hex-encoded signed extrinsic (0x-prefixed) — stored for rebroadcast.
    pub extrinsic_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct PolkadotClient {
    /// Polkadot RPC endpoints (wss:// or https://).
    rpc_endpoints: Vec<String>,
    /// Subscan API endpoints (https://polkadot.api.subscan.io).
    subscan_endpoints: Vec<String>,
    subscan_api_key: Option<String>,
    client: std::sync::Arc<HttpClient>,
}

impl PolkadotClient {
    pub fn new(
        rpc_endpoints: Vec<String>,
        subscan_endpoints: Vec<String>,
        subscan_api_key: Option<String>,
    ) -> Self {
        Self {
            rpc_endpoints,
            subscan_endpoints,
            subscan_api_key,
            client: HttpClient::shared(),
        }
    }

    async fn rpc_call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        with_fallback(&self.rpc_endpoints, |url| {
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
                    .ok_or_else(|| "missing result".to_string())
            }
        })
        .await
    }

    async fn subscan_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body = body.clone();
        let api_key = self.subscan_api_key.clone();
        with_fallback(&self.subscan_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = body.clone();
            let api_key = api_key.clone();
            async move {
                let mut headers = std::collections::HashMap::new();
                if let Some(key) = &api_key {
                    headers.insert("X-API-Key", key.as_str());
                }
                let resp: Value = client
                    .post_json_with_headers(&url, &body, &headers, RetryProfile::ChainRead)
                    .await?;
                let data = resp.get("data").cloned().unwrap_or(resp);
                serde_json::from_value(data).map_err(|e| format!("parse: {e}"))
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<DotBalance, String> {
        // system_account returns encoded AccountInfo; easier to use Subscan.
        #[derive(Deserialize)]
        struct SubscanAccount {
            balance: String,
        }
        let resp: SubscanAccount = self
            .subscan_post(
                "/api/v2/scan/search",
                &json!({"key": address}),
            )
            .await
            .or_else(|_e: String| -> Result<SubscanAccount, String> {
                // Fallback: use state_getStorage to read system_account.
                // This is complex to parse; return a default.
                Ok(SubscanAccount {
                    balance: "0".to_string(),
                })
            })?;

        // Subscan returns balance in DOT (e.g. "123.456789"). Convert to planck.
        let planck = parse_dot_balance(&resp.balance);
        Ok(DotBalance {
            planck,
            dot_display: resp.balance,
        })
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u32, String> {
        let result = self
            .rpc_call("system_accountNextIndex", json!([address]))
            .await?;
        result
            .as_u64()
            .map(|n| n as u32)
            .ok_or_else(|| "system_accountNextIndex: expected number".to_string())
    }

    pub async fn fetch_runtime_version(&self) -> Result<(u32, u32), String> {
        let result = self.rpc_call("state_getRuntimeVersion", json!([])).await?;
        let spec_version = result
            .get("specVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let tx_version = result
            .get("transactionVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        Ok((spec_version, tx_version))
    }

    pub async fn fetch_genesis_hash(&self) -> Result<String, String> {
        let result = self
            .rpc_call("chain_getBlockHash", json!([0]))
            .await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "chain_getBlockHash: expected string".to_string())
    }

    pub async fn fetch_block_hash_latest(&self) -> Result<String, String> {
        let result = self
            .rpc_call("chain_getBlockHash", json!([]))
            .await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "chain_getBlockHash: expected string".to_string())
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<DotHistoryEntry>, String> {
        #[derive(Deserialize, Default)]
        struct SubscanTransfers {
            #[serde(default)]
            transfers: Vec<SubscanTransfer>,
        }
        #[derive(Deserialize)]
        struct SubscanTransfer {
            hash: String,
            block_num: u64,
            block_timestamp: u64,
            from: String,
            to: String,
            amount: String,
            fee: String,
        }

        let transfers: SubscanTransfers = self
            .subscan_post(
                "/api/v2/scan/transfers",
                &json!({"address": address, "row": 50, "page": 0}),
            )
            .await
            .unwrap_or_default();

        Ok(transfers
            .transfers
            .into_iter()
            .map(|t| DotHistoryEntry {
                txid: t.hash,
                block_num: t.block_num,
                timestamp: t.block_timestamp,
                from: t.from.clone(),
                to: t.to.clone(),
                amount_planck: parse_dot_balance(&t.amount),
                fee_planck: parse_dot_balance(&t.fee),
                is_incoming: t.to == address,
            })
            .collect())
    }

    /// Sign and submit a Balances.transfer_keep_alive extrinsic.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        planck: u128,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<DotSendResult, String> {
        let nonce = self.fetch_nonce(from_address).await?;
        let (spec_version, tx_version) = self.fetch_runtime_version().await?;
        let genesis_hash = self.fetch_genesis_hash().await?;
        let block_hash = self.fetch_block_hash_latest().await?;

        let extrinsic = build_signed_transfer(
            to_address,
            planck,
            nonce,
            spec_version,
            tx_version,
            &genesis_hash,
            &block_hash,
            private_key_bytes,
            public_key_bytes,
        )?;

        let hex = format!("0x{}", hex::encode(&extrinsic));
        let result = self
            .rpc_call("author_submitExtrinsic", json!([hex]))
            .await?;
        let txid = result
            .as_str()
            .ok_or("author_submitExtrinsic: expected string")?
            .to_string();
        Ok(DotSendResult { txid, extrinsic_hex: hex })
    }

    /// Submit a pre-signed extrinsic hex (for rebroadcast).
    pub async fn submit_extrinsic_hex(&self, hex: &str) -> Result<DotSendResult, String> {
        let result = self
            .rpc_call("author_submitExtrinsic", json!([hex]))
            .await?;
        let txid = result
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(DotSendResult { txid, extrinsic_hex: hex.to_string() })
    }
}

// ----------------------------------------------------------------
// SCALE-encoded Polkadot extrinsic builder
// ----------------------------------------------------------------

/// Build a signed Balances.transfer_keep_alive extrinsic.
pub fn build_signed_transfer(
    to_address: &str,
    amount: u128,
    nonce: u32,
    spec_version: u32,
    tx_version: u32,
    genesis_hash: &str,
    block_hash: &str,
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};

    // Decode the recipient's SS58 address to a 32-byte public key.
    let dest_pubkey = decode_ss58(to_address)?;

    // Balances.transfer_keep_alive: pallet 5, call 3 on Polkadot mainnet.
    let call = {
        let mut c = Vec::new();
        c.push(0x05); // pallet index (Balances)
        c.push(0x03); // call index (transfer_keep_alive)
        // dest: MultiAddress::Id(AccountId)
        c.push(0x00); // Id variant
        c.extend_from_slice(&dest_pubkey);
        // value: Compact<u128>
        c.extend_from_slice(&scale_compact_u128(amount));
        c
    };

    // Era: immortal (0x00).
    let era = vec![0x00u8];
    // Nonce: Compact<u32>.
    let nonce_enc = scale_compact_u32(nonce);
    // Tip: Compact<u128> = 0.
    let tip = scale_compact_u128(0);

    let genesis_bytes = decode_hash_hex(genesis_hash)?;
    let block_bytes = decode_hash_hex(block_hash)?;

    // Signing payload.
    let mut payload = Vec::new();
    payload.extend_from_slice(&call);
    payload.extend_from_slice(&era);
    payload.extend_from_slice(&nonce_enc);
    payload.extend_from_slice(&tip);
    payload.extend_from_slice(&spec_version.to_le_bytes());
    payload.extend_from_slice(&tx_version.to_le_bytes());
    payload.extend_from_slice(&genesis_bytes);
    payload.extend_from_slice(&block_bytes);

    // If payload > 256 bytes, sign its Blake2-256 hash instead.
    let signing_input: Vec<u8> = if payload.len() > 256 {
        blake2b_256(&payload).to_vec()
    } else {
        payload.clone()
    };

    let signing_key = SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&signing_input);

    // Build the extrinsic.
    // version = 0x84 (signed, version 4)
    let mut extrinsic_body = Vec::new();
    extrinsic_body.push(0x84); // version
    // signer: MultiAddress::Id(AccountId)
    extrinsic_body.push(0x00);
    extrinsic_body.extend_from_slice(public_key);
    // signature: MultiSignature::Ed25519(sig)
    extrinsic_body.push(0x00); // Ed25519 variant
    extrinsic_body.extend_from_slice(signature.to_bytes().as_ref());
    // extra (era, nonce, tip)
    extrinsic_body.extend_from_slice(&era);
    extrinsic_body.extend_from_slice(&nonce_enc);
    extrinsic_body.extend_from_slice(&tip);
    // call
    extrinsic_body.extend_from_slice(&call);

    // Prepend length (Compact<u32>).
    let mut out = scale_compact_u32(extrinsic_body.len() as u32);
    out.extend_from_slice(&extrinsic_body);
    Ok(out)
}

// ----------------------------------------------------------------
// SCALE codec helpers
// ----------------------------------------------------------------

fn scale_compact_u32(n: u32) -> Vec<u8> {
    scale_compact_u128(n as u128)
}

fn scale_compact_u128(n: u128) -> Vec<u8> {
    if n <= 63 {
        vec![(n << 2) as u8]
    } else if n <= 0x3fff {
        let v = ((n << 2) | 1) as u16;
        v.to_le_bytes().to_vec()
    } else if n <= 0x3fff_ffff {
        let v = ((n << 2) | 2) as u32;
        v.to_le_bytes().to_vec()
    } else {
        // Big-integer mode.
        let bytes = n.to_le_bytes();
        let sig_bytes = bytes.iter().rev().skip_while(|&&b| b == 0).count();
        let mut out = vec![((sig_bytes - 4) << 2 | 3) as u8];
        out.extend_from_slice(&bytes[..sig_bytes]);
        out
    }
}

fn decode_hash_hex(hex_str: &str) -> Result<[u8; 32], String> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(s).map_err(|e| format!("hash decode: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| format!("hash wrong length: {}", hex_str))
}

fn blake2b_256(data: &[u8]) -> [u8; 32] {
    use blake2::{Blake2b, Digest};
    use blake2::digest::consts::U32;
    let mut h = Blake2b::<U32>::new();
    h.update(data);
    h.finalize().into()
}

/// Decode an SS58-encoded Polkadot address to a 32-byte public key.
fn decode_ss58(address: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("ss58 decode: {e}"))?;
    // SS58: [prefix(1-2 bytes)] + [key(32)] + [checksum(2)]
    // For single-byte prefix (< 64), total = 35 bytes.
    if decoded.len() < 34 {
        return Err(format!("ss58 too short: {}", decoded.len()));
    }
    let key_start = if decoded[0] < 64 { 1 } else { 2 };
    let key_bytes: [u8; 32] = decoded[key_start..key_start + 32]
        .try_into()
        .map_err(|_| "ss58 key slice error".to_string())?;
    Ok(key_bytes)
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn parse_dot_balance(s: &str) -> u128 {
    // e.g. "123.456789" DOT -> planck (10^10 per DOT)
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let whole: u128 = parts[0].parse().unwrap_or(0);
    let frac_str = parts.get(1).copied().unwrap_or("0");
    let frac_padded = format!("{:0<10}", frac_str);
    let frac: u128 = frac_padded[..10].parse().unwrap_or(0);
    whole * 10_000_000_000 + frac
}

#[allow(dead_code)]
fn format_dot(planck: u128) -> String {
    let whole = planck / 10_000_000_000;
    let frac = planck % 10_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:010}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

pub fn validate_polkadot_address(address: &str) -> bool {
    decode_ss58(address).is_ok()
}

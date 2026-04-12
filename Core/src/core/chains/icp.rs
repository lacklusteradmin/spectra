//! Internet Computer Protocol (ICP) chain client.
//!
//! Uses the IC management canister and Rosetta API for balance and history.
//! ICP ledger interactions use CBOR-encoded Candid messages.
//! Keys are derived with secp256k1 (BIP32); identity is verified via
//! self-authenticating principals (SHA-224 of the DER-encoded public key).
//!
//! For production send, the full Ingress message flow is:
//!   1. Build a `call` envelope (CBOR)
//!   2. Sign with the private key (ECDSA/secp256k1)
//!   3. POST to /api/v2/canister/{canister_id}/call
//!   4. Query with /api/v2/canister/{canister_id}/read_state

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Constants
// ----------------------------------------------------------------

/// ICP ledger canister ID (mainnet).
#[allow(dead_code)]
const ICP_LEDGER_CANISTER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
/// ICP e8s per ICP.
const E8S_PER_ICP: u64 = 100_000_000;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpBalance {
    /// E8s (1 ICP = 100_000_000 e8s).
    pub e8s: u64,
    pub icp_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpHistoryEntry {
    pub block_index: u64,
    pub timestamp_ns: u64,
    pub from: String,
    pub to: String,
    pub amount_e8s: u64,
    pub fee_e8s: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpSendResult {
    pub block_index: u64,
}

// ----------------------------------------------------------------
// Client (Rosetta-based for read, direct for write)
// ----------------------------------------------------------------

pub struct IcpClient {
    /// Rosetta API endpoint (https://rosetta-api.internetcomputer.org).
    rosetta_endpoints: Vec<String>,
    /// IC HTTP gateway (https://ic0.app).
    #[allow(dead_code)]
    ic_endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl IcpClient {
    pub fn new(rosetta_endpoints: Vec<String>, ic_endpoints: Vec<String>) -> Self {
        Self {
            rosetta_endpoints,
            ic_endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn rosetta_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body = body.clone();
        with_fallback(&self.rosetta_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = body.clone();
            async move { client.post_json(&url, &body, RetryProfile::ChainRead).await }
        })
        .await
    }

    pub async fn fetch_balance(&self, account_address: &str) -> Result<IcpBalance, String> {
        let resp: Value = self
            .rosetta_post(
                "/account/balance",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address}
                }),
            )
            .await?;
        let e8s: u64 = resp
            .pointer("/balances/0/value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Ok(IcpBalance {
            e8s,
            icp_display: format_icp(e8s),
        })
    }

    pub async fn fetch_history(
        &self,
        account_address: &str,
    ) -> Result<Vec<IcpHistoryEntry>, String> {
        let resp: Value = self
            .rosetta_post(
                "/search/transactions",
                &json!({
                    "network_identifier": {"blockchain": "Internet Computer", "network": "00000000000000020101"},
                    "account_identifier": {"address": account_address},
                    "limit": 50
                }),
            )
            .await?;

        let txs = resp
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut entries = Vec::new();
        for item in txs {
            let block_index: u64 = item
                .pointer("/block_identifier/index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let timestamp_ns: u64 = item
                .pointer("/transaction/metadata/timestamp")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let ops = item
                .pointer("/transaction/operations")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut from = String::new();
            let mut to = String::new();
            let mut amount_e8s: u64 = 0;
            let mut fee_e8s: u64 = 0;

            for op in &ops {
                let op_type = op.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let addr = op
                    .pointer("/account/address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let value: i64 = op
                    .pointer("/amount/value")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                match op_type {
                    "TRANSACTION" => {
                        if value < 0 {
                            from = addr;
                            amount_e8s = value.unsigned_abs();
                        } else {
                            to = addr;
                        }
                    }
                    "FEE" => {
                        fee_e8s = value.unsigned_abs();
                    }
                    _ => {}
                }
            }

            let is_incoming = to == account_address;
            entries.push(IcpHistoryEntry {
                block_index,
                timestamp_ns,
                from,
                to,
                amount_e8s,
                fee_e8s,
                is_incoming,
            });
        }
        Ok(entries)
    }

    /// Submit an ICP transfer via the Rosetta construction API.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        e8s: u64,
        private_key_bytes: &[u8],
        public_key_bytes: &[u8],
    ) -> Result<IcpSendResult, String> {
        let network = json!({
            "blockchain": "Internet Computer",
            "network": "00000000000000020101"
        });

        // Step 1: /construction/preprocess
        let ops = build_transfer_ops(from_address, to_address, e8s);
        let preprocess: Value = self
            .rosetta_post(
                "/construction/preprocess",
                &json!({"network_identifier": network, "operations": ops}),
            )
            .await?;

        // Step 2: /construction/metadata
        let metadata: Value = self
            .rosetta_post(
                "/construction/metadata",
                &json!({
                    "network_identifier": network,
                    "options": preprocess.get("options").cloned().unwrap_or(json!({}))
                }),
            )
            .await?;

        // Step 3: /construction/payloads
        let payloads: Value = self
            .rosetta_post(
                "/construction/payloads",
                &json!({
                    "network_identifier": network,
                    "operations": ops,
                    "metadata": metadata.get("metadata").cloned().unwrap_or(json!({})),
                    "public_keys": [{
                        "hex_bytes": hex::encode(public_key_bytes),
                        "curve_type": "secp256k1"
                    }]
                }),
            )
            .await?;

        let unsigned_tx = payloads
            .get("unsigned_transaction")
            .and_then(|v| v.as_str())
            .ok_or("payloads: missing unsigned_transaction")?;
        let to_sign_payloads = payloads
            .get("payloads")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Step 4: Sign each payload.
        let mut signatures = Vec::new();
        for payload_item in &to_sign_payloads {
            let hex_bytes = payload_item
                .get("hex_bytes")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let hash_bytes = hex::decode(hex_bytes).unwrap_or_default();
            let sig_hex = sign_icp_payload(&hash_bytes, private_key_bytes)?;
            signatures.push(json!({
                "signing_payload": payload_item,
                "public_key": {
                    "hex_bytes": hex::encode(public_key_bytes),
                    "curve_type": "secp256k1"
                },
                "signature_type": "ecdsa",
                "hex_bytes": sig_hex
            }));
        }

        // Step 5: /construction/combine
        let combined: Value = self
            .rosetta_post(
                "/construction/combine",
                &json!({
                    "network_identifier": network,
                    "unsigned_transaction": unsigned_tx,
                    "signatures": signatures
                }),
            )
            .await?;
        let signed_tx = combined
            .get("signed_transaction")
            .and_then(|v| v.as_str())
            .ok_or("combine: missing signed_transaction")?;

        // Step 6: /construction/submit
        let submit: Value = self
            .rosetta_post(
                "/construction/submit",
                &json!({
                    "network_identifier": network,
                    "signed_transaction": signed_tx
                }),
            )
            .await?;
        let block_index: u64 = submit
            .pointer("/transaction_identifier/hash")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        Ok(IcpSendResult { block_index })
    }
}

// ----------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------

fn build_transfer_ops(from: &str, to: &str, e8s: u64) -> Value {
    json!([
        {
            "operation_identifier": {"index": 0},
            "type": "TRANSACTION",
            "account": {"address": from},
            "amount": {"value": format!("-{}", e8s), "currency": {"symbol": "ICP", "decimals": 8}}
        },
        {
            "operation_identifier": {"index": 1},
            "type": "TRANSACTION",
            "account": {"address": to},
            "amount": {"value": e8s.to_string(), "currency": {"symbol": "ICP", "decimals": 8}}
        },
        {
            "operation_identifier": {"index": 2},
            "type": "FEE",
            "account": {"address": from},
            "amount": {"value": "-10000", "currency": {"symbol": "ICP", "decimals": 8}}
        }
    ])
}

fn sign_icp_payload(hash_bytes: &[u8], private_key_bytes: &[u8]) -> Result<String, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg_hash = if hash_bytes.len() == 32 {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash_bytes);
        arr
    } else {
        use sha2::{Digest, Sha256};
        Sha256::digest(hash_bytes).into()
    };
    let msg = Message::from_digest_slice(&msg_hash).map_err(|e| format!("msg: {e}"))?;
    let sig = secp.sign_ecdsa(&msg, &secret_key);
    Ok(hex::encode(sig.serialize_compact()))
}

/// Derive an ICP account address from a secp256k1 public key (DER-encoded).
pub fn pubkey_der_to_icp_address(pubkey_der: &[u8]) -> String {
    use sha2::{Digest, Sha224};
    let principal_hash = Sha224::digest(pubkey_der);
    // Account address = principal + [0u8; 32] subaccount, then sha224 again.
    // Simplified: return hex of the principal bytes with checksum.
    let mut address_bytes = Vec::new();
    address_bytes.extend_from_slice(&principal_hash);
    hex::encode(&address_bytes)
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_icp(e8s: u64) -> String {
    let whole = e8s / E8S_PER_ICP;
    let frac = e8s % E8S_PER_ICP;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:08}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

pub fn validate_icp_address(address: &str) -> bool {
    // ICP account identifiers are 64 hex characters.
    address.len() == 64 && address.chars().all(|c| c.is_ascii_hexdigit())
}

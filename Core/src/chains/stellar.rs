//! Stellar chain client.
//!
//! Uses the Horizon REST API for account info, history, fee stats,
//! and transaction submission. Signs with Ed25519 using ed25519-dalek.
//! XDR encoding is done manually (minimal subset for Payment operation).

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarBalance {
    /// Stroops (1 XLM = 10_000_000 stroops).
    pub stroops: i64,
    pub xlm_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarAssetBalance {
    pub asset_code: String,
    pub asset_issuer: String,
    /// Fixed 7-decimal stroop units (same precision as XLM).
    pub amount_stroops: i64,
    pub amount_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarHistoryEntry {
    pub txid: String,
    pub ledger: u64,
    pub timestamp: String,
    pub from: String,
    pub to: String,
    pub amount_stroops: i64,
    pub fee_charged: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarSendResult {
    pub txid: String,
    /// Base64-encoded signed XDR envelope — stored for rebroadcast.
    pub signed_xdr_b64: String,
}

// ----------------------------------------------------------------
// Horizon API response types
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct HorizonAccount {
    balances: Vec<HorizonBalance>,
    sequence: String,
}

#[derive(Debug, Deserialize)]
struct HorizonBalance {
    balance: String,
    asset_type: String,
    #[serde(default)]
    asset_code: String,
    #[serde(default)]
    asset_issuer: String,
}

#[derive(Debug, Deserialize)]
struct HorizonFeeStats {
    fee_charged: HorizonFeeCharged,
}

#[derive(Debug, Deserialize)]
struct HorizonFeeCharged {
    mode: String,
}

#[derive(Debug, Deserialize)]
struct HorizonPayments {
    #[serde(rename = "_embedded")]
    embedded: HorizonPaymentsEmbedded,
}

#[derive(Debug, Deserialize)]
struct HorizonPaymentsEmbedded {
    records: Vec<HorizonPaymentRecord>,
}

#[derive(Debug, Deserialize)]
struct HorizonPaymentRecord {
    #[allow(dead_code)]
    id: String,
    #[serde(rename = "type")]
    op_type: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    amount: String,
    created_at: String,
    transaction_hash: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct StellarClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl StellarClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<StellarBalance, String> {
        let account: HorizonAccount = self
            .get(&format!("/accounts/{address}"))
            .await?;
        let native = account
            .balances
            .iter()
            .find(|b| b.asset_type == "native")
            .ok_or("no native balance")?;
        // Stellar balances are decimal strings (e.g. "100.0000000")
        let stroops = parse_stellar_amount(&native.balance)?;
        Ok(StellarBalance {
            stroops,
            xlm_display: native.balance.clone(),
        })
    }

    /// Fetch a custom (issued) asset balance. `asset_code` is the alphanumeric
    /// asset code (e.g. "USDC"); `asset_issuer` is the G... issuer account.
    /// If the account has no trustline to this asset, returns a zero balance.
    pub async fn fetch_asset_balance(
        &self,
        address: &str,
        asset_code: &str,
        asset_issuer: &str,
    ) -> Result<StellarAssetBalance, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        let entry = account.balances.iter().find(|b| {
            b.asset_type != "native"
                && b.asset_code == asset_code
                && b.asset_issuer == asset_issuer
        });
        match entry {
            Some(b) => {
                let stroops = parse_stellar_amount(&b.balance)?;
                Ok(StellarAssetBalance {
                    asset_code: asset_code.to_string(),
                    asset_issuer: asset_issuer.to_string(),
                    amount_stroops: stroops,
                    amount_display: b.balance.clone(),
                })
            }
            None => Ok(StellarAssetBalance {
                asset_code: asset_code.to_string(),
                asset_issuer: asset_issuer.to_string(),
                amount_stroops: 0,
                amount_display: "0.0000000".to_string(),
            }),
        }
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u64, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        account
            .sequence
            .parse::<u64>()
            .map_err(|e| format!("sequence parse: {e}"))
    }

    pub async fn fetch_base_fee(&self) -> Result<u64, String> {
        let stats: HorizonFeeStats = self.get("/fee_stats").await?;
        stats
            .fee_charged
            .mode
            .parse::<u64>()
            .unwrap_or(100) // 100 stroops default
            .pipe(Ok)
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<StellarHistoryEntry>, String> {
        let payments: HorizonPayments = self
            .get(&format!(
                "/accounts/{address}/payments?limit=50&order=desc&include_failed=false"
            ))
            .await?;
        Ok(payments
            .embedded
            .records
            .into_iter()
            .filter(|r| r.op_type == "payment" || r.op_type == "create_account")
            .map(|r| {
                let amount_stroops = parse_stellar_amount(&r.amount).unwrap_or(0);
                let is_incoming = r.to == address;
                StellarHistoryEntry {
                    txid: r.transaction_hash,
                    ledger: 0,
                    timestamp: r.created_at,
                    from: r.from,
                    to: r.to,
                    amount_stroops,
                    fee_charged: 0,
                    is_incoming,
                }
            })
            .collect())
    }

    /// Sign and submit a native XLM Payment transaction.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        stroops: i64,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<StellarSendResult, String> {
        self.sign_and_submit_with_asset(
            from_address,
            to_address,
            stroops,
            StellarAsset::Native,
            private_key_bytes,
            public_key_bytes,
        )
        .await
    }

    /// Sign and submit a custom-asset (credit_alphanum4 / credit_alphanum12)
    /// Payment transaction.
    pub async fn sign_and_submit_asset(
        &self,
        from_address: &str,
        to_address: &str,
        stroops: i64,
        asset_code: &str,
        asset_issuer: &str,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<StellarSendResult, String> {
        let issuer_key = decode_stellar_address(asset_issuer)?;
        let code_len = asset_code.len();
        if code_len == 0 || code_len > 12 {
            return Err(format!("invalid asset code length: {code_len}"));
        }
        let asset = StellarAsset::Credit {
            code: asset_code.to_string(),
            issuer: issuer_key,
        };
        self.sign_and_submit_with_asset(
            from_address,
            to_address,
            stroops,
            asset,
            private_key_bytes,
            public_key_bytes,
        )
        .await
    }

    async fn sign_and_submit_with_asset(
        &self,
        from_address: &str,
        to_address: &str,
        stroops: i64,
        asset: StellarAsset,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<StellarSendResult, String> {
        let sequence = self.fetch_sequence(from_address).await? + 1;
        let base_fee = self.fetch_base_fee().await?;

        let network_passphrase = b"Public Global Stellar Network ; September 2015";
        let tx_xdr = build_signed_payment_xdr_with_asset(
            from_address,
            to_address,
            stroops,
            &asset,
            base_fee,
            sequence,
            network_passphrase,
            private_key_bytes,
            public_key_bytes,
        )?;

        // Submit via Horizon POST /transactions
        use base64::Engine;
        let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_xdr);

        let result = with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let tx_b64 = tx_b64.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: serde_json::Value = client
                    .post_json(
                        &url,
                        &serde_json::json!({"tx": tx_b64}),
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                let hash = resp
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .ok_or("submit: missing hash")?
                    .to_string();
                Ok(StellarSendResult { txid: hash, signed_xdr_b64: tx_b64.clone() })
            }
        })
        .await?;

        Ok(result)
    }

    /// Submit a pre-signed XDR envelope (for rebroadcast).
    pub async fn submit_envelope_b64(&self, tx_b64: &str) -> Result<StellarSendResult, String> {
        let tx_b64 = tx_b64.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let tx_b64 = tx_b64.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: serde_json::Value = client
                    .post_json(
                        &url,
                        &serde_json::json!({"tx": tx_b64}),
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                let hash = resp
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(StellarSendResult { txid: hash, signed_xdr_b64: tx_b64.clone() })
            }
        })
        .await
    }
}

// ----------------------------------------------------------------
// XDR transaction builder
// ----------------------------------------------------------------

/// Stellar Asset variants for Payment operations.
#[derive(Debug, Clone)]
pub enum StellarAsset {
    Native,
    /// `code` is 1-12 alphanumeric ASCII, `issuer` is the 32-byte ed25519 key.
    Credit { code: String, issuer: [u8; 32] },
}

/// Build a signed Stellar Payment transaction in XDR binary (native XLM).
pub fn build_signed_payment_xdr(
    from: &str,
    to: &str,
    stroops: i64,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    build_signed_payment_xdr_with_asset(
        from,
        to,
        stroops,
        &StellarAsset::Native,
        base_fee,
        sequence,
        network_passphrase,
        private_key,
        public_key,
    )
}

/// Build a signed Stellar Payment transaction with an arbitrary asset.
pub fn build_signed_payment_xdr_with_asset(
    from: &str,
    to: &str,
    stroops: i64,
    asset: &StellarAsset,
    base_fee: u64,
    sequence: u64,
    network_passphrase: &[u8],
    private_key: &[u8; 64],
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let _from_bytes = decode_stellar_address(from)?;
    let to_bytes = decode_stellar_address(to)?;

    // Network hash prefix for transaction signing.
    let network_hash: [u8; 32] = Sha256::digest(network_passphrase).into();

    // TransactionV0/Transaction XDR encoding (manual).
    let tx_xdr = encode_payment_tx(&to_bytes, stroops, asset, base_fee, sequence, public_key)?;

    // Signing payload: sha256(network_hash || ENVELOPE_TYPE_TX(2) || tx_xdr)
    let mut payload = Vec::new();
    payload.extend_from_slice(&network_hash);
    payload.extend_from_slice(&2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    payload.extend_from_slice(&tx_xdr);
    let sig_payload: [u8; 32] = Sha256::digest(&payload).into();

    let signing_key = SigningKey::from_bytes(
        &private_key[..32].try_into().map_err(|_| "privkey too short")?,
    );
    let signature = signing_key.sign(&sig_payload);

    // TransactionEnvelope: type=ENVELOPE_TYPE_TX(2), tx, signatures
    let mut envelope = Vec::new();
    envelope.extend_from_slice(&2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    envelope.extend_from_slice(&tx_xdr);
    // DecoratedSignature array (1 item)
    envelope.extend_from_slice(&1u32.to_be_bytes()); // array length
    // hint = last 4 bytes of public key
    envelope.extend_from_slice(&public_key[28..32]);
    // signature (VarOpaque, max 64)
    xdr_write_bytes(&mut envelope, signature.to_bytes().as_ref());

    Ok(envelope)
}

fn encode_payment_tx(
    to: &[u8; 32],
    stroops: i64,
    asset: &StellarAsset,
    base_fee: u64,
    sequence: u64,
    public_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    let mut tx = Vec::new();
    // sourceAccount: PUBLIC_KEY_TYPE_ED25519(0) + key
    tx.extend_from_slice(&0u32.to_be_bytes());
    tx.extend_from_slice(public_key);
    // fee (Uint32)
    tx.extend_from_slice(&(base_fee as u32).to_be_bytes());
    // seqNum (SequenceNumber = Int64)
    tx.extend_from_slice(&(sequence as i64).to_be_bytes());
    // timeBounds: optional=0 (none)
    tx.extend_from_slice(&0u32.to_be_bytes());
    // memo: MEMO_NONE=0
    tx.extend_from_slice(&0u32.to_be_bytes());
    // operations: array of 1
    tx.extend_from_slice(&1u32.to_be_bytes());
    // Operation: sourceAccount optional=0, type=PAYMENT(1)
    tx.extend_from_slice(&0u32.to_be_bytes()); // no source account override
    tx.extend_from_slice(&1u32.to_be_bytes()); // PAYMENT op type
    // PaymentOp: destination (PUBLIC_KEY_TYPE_ED25519 + key)
    tx.extend_from_slice(&0u32.to_be_bytes());
    tx.extend_from_slice(to);
    // asset
    encode_asset(&mut tx, asset)?;
    // amount: Int64
    tx.extend_from_slice(&stroops.to_be_bytes());
    // ext: 0
    tx.extend_from_slice(&0u32.to_be_bytes());
    Ok(tx)
}

/// Encode a Stellar Asset into XDR.
/// ASSET_TYPE_NATIVE=0, CREDIT_ALPHANUM4=1, CREDIT_ALPHANUM12=2.
fn encode_asset(tx: &mut Vec<u8>, asset: &StellarAsset) -> Result<(), String> {
    match asset {
        StellarAsset::Native => {
            tx.extend_from_slice(&0u32.to_be_bytes());
        }
        StellarAsset::Credit { code, issuer } => {
            let bytes = code.as_bytes();
            let len = bytes.len();
            if len == 0 || len > 12 {
                return Err(format!("asset code length {len} out of range"));
            }
            if !bytes.iter().all(|b| b.is_ascii_alphanumeric()) {
                return Err(format!("asset code contains non-alphanumeric: {code}"));
            }
            if len <= 4 {
                // ASSET_TYPE_CREDIT_ALPHANUM4 = 1
                tx.extend_from_slice(&1u32.to_be_bytes());
                // assetCode4: opaque[4] (fixed, right-padded with zeros)
                let mut code4 = [0u8; 4];
                code4[..len].copy_from_slice(bytes);
                tx.extend_from_slice(&code4);
            } else {
                // ASSET_TYPE_CREDIT_ALPHANUM12 = 2
                tx.extend_from_slice(&2u32.to_be_bytes());
                // assetCode12: opaque[12] (fixed, right-padded with zeros)
                let mut code12 = [0u8; 12];
                code12[..len].copy_from_slice(bytes);
                tx.extend_from_slice(&code12);
            }
            // issuer: AccountID (PUBLIC_KEY_TYPE_ED25519 + 32-byte key)
            tx.extend_from_slice(&0u32.to_be_bytes());
            tx.extend_from_slice(issuer);
        }
    }
    Ok(())
}

fn xdr_write_bytes(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    // XDR pads to 4-byte boundary
    let pad = (4 - (len % 4)) % 4;
    for _ in 0..pad {
        out.push(0);
    }
}

// ----------------------------------------------------------------
// Stellar address decode (strkey)
// ----------------------------------------------------------------

fn decode_stellar_address(address: &str) -> Result<[u8; 32], String> {
    // Stellar strkey uses RFC 4648 base32 (no padding) with CRC-16 checksum.
    let decoded = base32_decode_rfc4648(address.trim())
        .ok_or_else(|| format!("stellar base32 decode failed: {address}"))?;

    // Layout: [version_byte(1)] + [key(32)] + [checksum(2)]
    if decoded.len() != 35 {
        return Err(format!("stellar address wrong length: {}", decoded.len()));
    }
    let version = decoded[0];
    if version != 0x30 {
        // G-address = 6 << 3 = 0x30
        return Err(format!("stellar address wrong version: {version:#x}"));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded[1..33]);
    Ok(key)
}

/// Minimal RFC 4648 base32 decoder (no padding, uppercase alphabet).
fn base32_decode_rfc4648(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let s = s.to_uppercase();
    let mut bits: u32 = 0;
    let mut bit_count: u8 = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let val = ALPHABET.iter().position(|&b| b == c)? as u32;
        bits = (bits << 5) | val;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    Some(out)
}

// ----------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------

fn parse_stellar_amount(s: &str) -> Result<i64, String> {
    // "100.0000000" -> stroops
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let whole: i64 = parts[0].parse().map_err(|e| format!("amount parse: {e}"))?;
    let frac_str = parts.get(1).copied().unwrap_or("0");
    let frac_padded = format!("{:0<7}", frac_str);
    let frac: i64 = frac_padded[..7].parse().unwrap_or(0);
    Ok(whole * 10_000_000 + frac)
}

trait Pipe: Sized {
    fn pipe<F: FnOnce(Self) -> R, R>(self, f: F) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

pub fn validate_stellar_address(address: &str) -> bool {
    decode_stellar_address(address).is_ok()
}

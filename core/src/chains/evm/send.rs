//! EVM send: EIP-1559 RLP builder, secp256k1 signer, eth_sendRawTransaction
//! broadcast (native ETH + ERC-20 transfer paths, with optional override hooks
//! for replacement-by-fee / speed-up / cancel flows).

use serde_json::json;

use super::{decode_hex, EvmClient, EvmSendResult};

impl EvmClient {
    /// Sign and broadcast an EIP-1559 ETH transfer.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        value_wei: u128,
        private_key_bytes: &[u8],
    ) -> Result<EvmSendResult, String> {
        self.sign_and_broadcast_with_overrides(
            from_address,
            to_address,
            value_wei,
            private_key_bytes,
            EvmSendOverrides::default(),
        )
        .await
    }

    /// Sign and broadcast an EIP-1559 ETH transfer with optional overrides
    /// for nonce, gas limit, and fee fields. Used for "speed up" and "cancel"
    /// (replacement-by-fee) flows and power-user custom fee edits.
    pub async fn sign_and_broadcast_with_overrides(
        &self,
        from_address: &str,
        to_address: &str,
        value_wei: u128,
        private_key_bytes: &[u8],
        overrides: EvmSendOverrides,
    ) -> Result<EvmSendResult, String> {
        let nonce = match overrides.nonce {
            Some(n) => n,
            None => self.fetch_nonce(from_address).await?,
        };
        let (max_fee, max_priority) = resolve_fees(self, &overrides).await?;
        let gas_limit = overrides.gas_limit.unwrap_or(21_000);

        let raw_tx = build_eip1559_tx(
            self.chain_id,
            nonce,
            max_fee,
            max_priority,
            gas_limit,
            to_address,
            value_wei,
            &[],
            private_key_bytes,
        )?;

        let hex_tx = format!("0x{}", hex::encode(&raw_tx));
        let result = self
            .call("eth_sendRawTransaction", json!([hex_tx.clone()]))
            .await?;
        let txid = result
            .as_str()
            .ok_or("eth_sendRawTransaction: expected string")?
            .to_string();
        Ok(EvmSendResult {
            txid,
            nonce,
            raw_tx_hex: hex_tx,
            gas_limit,
            max_fee_per_gas_wei: max_fee.to_string(),
            max_priority_fee_per_gas_wei: max_priority.to_string(),
        })
    }

    /// Sign and broadcast an ERC-20 `transfer(to, amount)` from `from`.
    pub async fn sign_and_broadcast_erc20(
        &self,
        from_address: &str,
        contract: &str,
        to_address: &str,
        amount_raw: u128,
        private_key_bytes: &[u8],
    ) -> Result<EvmSendResult, String> {
        self.sign_and_broadcast_erc20_with_overrides(
            from_address,
            contract,
            to_address,
            amount_raw,
            private_key_bytes,
            EvmSendOverrides::default(),
        )
        .await
    }

    /// Sign and broadcast an ERC-20 transfer with fee/nonce overrides.
    pub async fn sign_and_broadcast_erc20_with_overrides(
        &self,
        from_address: &str,
        contract: &str,
        to_address: &str,
        amount_raw: u128,
        private_key_bytes: &[u8],
        overrides: EvmSendOverrides,
    ) -> Result<EvmSendResult, String> {
        let nonce = match overrides.nonce {
            Some(n) => n,
            None => self.fetch_nonce(from_address).await?,
        };
        let (max_fee, max_priority) = resolve_fees(self, &overrides).await?;

        let data = encode_erc20_transfer(to_address, amount_raw)?;
        let data_hex = format!("0x{}", hex::encode(&data));

        // Ask the node for the real gas limit unless the caller pinned one.
        let gas_limit = match overrides.gas_limit {
            Some(g) => g,
            None => self
                .estimate_gas(from_address, contract, 0u128, Some(&data_hex))
                .await
                .map(|g| g.saturating_add(g / 5)) // +20% buffer
                .unwrap_or(65_000),
        };

        let raw_tx = build_eip1559_tx(
            self.chain_id,
            nonce,
            max_fee,
            max_priority,
            gas_limit,
            contract,
            0u128,
            &data,
            private_key_bytes,
        )?;

        let hex_tx = format!("0x{}", hex::encode(&raw_tx));
        let result = self
            .call("eth_sendRawTransaction", json!([hex_tx.clone()]))
            .await?;
        let txid = result
            .as_str()
            .ok_or("eth_sendRawTransaction: expected string")?
            .to_string();
        Ok(EvmSendResult {
            txid,
            nonce,
            raw_tx_hex: hex_tx,
            gas_limit,
            max_fee_per_gas_wei: max_fee.to_string(),
            max_priority_fee_per_gas_wei: max_priority.to_string(),
        })
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
        Ok(EvmSendResult {
            txid,
            nonce: 0,
            raw_tx_hex: hex_tx.to_string(),
            gas_limit: 0,
            max_fee_per_gas_wei: String::new(),
            max_priority_fee_per_gas_wei: String::new(),
        })
    }
}

// ----------------------------------------------------------------
// Send overrides + fee resolution
// ----------------------------------------------------------------

/// Optional overrides for EIP-1559 sends. Any `None` field falls back to the
/// default behavior (latest-nonce / recommended fee / estimated gas limit).
///
/// * `nonce` — reuse a stuck transaction's nonce to build a replacement. The
///   EIP-1559 replacement-by-fee rule requires the new tx to bump BOTH
///   `max_fee_per_gas_wei` and `max_priority_fee_per_gas_wei` by at least 10%
///   vs. the stuck one.
/// * `max_fee_per_gas_wei` / `max_priority_fee_per_gas_wei` — explicit fee
///   fields. If either is `None` we fetch `fetch_fee_estimate()` and fill
///   the missing one from the suggestion.
/// * `gas_limit` — pin the gas limit instead of calling `eth_estimateGas`.
#[derive(Debug, Clone, Default)]
pub struct EvmSendOverrides {
    pub nonce: Option<u64>,
    pub max_fee_per_gas_wei: Option<u128>,
    pub max_priority_fee_per_gas_wei: Option<u128>,
    pub gas_limit: Option<u64>,
}

/// Resolve (max_fee_per_gas, max_priority_fee_per_gas) from overrides plus
/// fallback `fetch_fee_estimate()` values. If both fields are set, no RPC
/// call is made.
async fn resolve_fees(
    client: &EvmClient,
    overrides: &EvmSendOverrides,
) -> Result<(u128, u128), String> {
    match (
        overrides.max_fee_per_gas_wei,
        overrides.max_priority_fee_per_gas_wei,
    ) {
        (Some(mf), Some(mp)) => Ok((mf, mp)),
        (mf_opt, mp_opt) => {
            let fee = client.fetch_fee_estimate().await?;
            Ok((
                mf_opt.unwrap_or(fee.max_fee_per_gas_wei),
                mp_opt.unwrap_or(fee.priority_fee_wei),
            ))
        }
    }
}

// ----------------------------------------------------------------
// EIP-1559 transaction builder + signer
// ----------------------------------------------------------------

/// Build a signed EIP-1559 (type 2) transaction.
///
/// Returns the raw RLP-encoded transaction bytes, ready to be hex-encoded and
/// broadcast via `eth_sendRawTransaction`.
#[allow(clippy::too_many_arguments)]
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
    let msg_hash = super::derive::keccak256(&signing_payload);

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
// ERC-20 transfer ABI encoding (used by the send path)
// ----------------------------------------------------------------

/// Encode a `transfer(address,uint256)` call.
pub(crate) fn encode_erc20_transfer(to: &str, amount: u128) -> Result<Vec<u8>, String> {
    let to_bytes = decode_hex(to)?;
    if to_bytes.len() != 20 {
        return Err(format!("invalid EVM to length: {}", to_bytes.len()));
    }
    let mut out = Vec::with_capacity(4 + 32 + 32);
    out.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]); // transfer(address,uint256)
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&to_bytes);

    // 32-byte big-endian amount, left-padded with zeros.
    let mut amount_bytes = [0u8; 32];
    amount_bytes[16..].copy_from_slice(&amount.to_be_bytes());
    out.extend_from_slice(&amount_bytes);
    Ok(out)
}

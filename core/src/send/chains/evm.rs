//! EVM send: EIP-1559 RLP builder, secp256k1 signer, eth_sendRawTransaction
//! broadcast (native ETH + ERC-20 transfer paths, with optional override hooks
//! for replacement-by-fee / speed-up / cancel flows).

use serde_json::json;

use crate::fetch::chains::evm::{
    decode_hex, EvmClient, EvmSendResult, SEL_APPROVE, SEL_TRANSFER, SEL_TRANSFER_FROM,
};

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

        let native_data = overrides.calldata.as_deref().unwrap_or(&[]);
        let raw_tx = build_eip1559_tx(
            self.chain_id,
            nonce,
            max_fee,
            max_priority,
            gas_limit,
            to_address,
            value_wei,
            native_data,
            &overrides.access_list,
            private_key_bytes,
        )?;

        let hex_tx = format!("0x{}", hex::encode(&raw_tx));

        if overrides.sign_only {
            return Ok(EvmSendResult {
                txid: String::new(),
                nonce,
                raw_tx_hex: hex_tx,
                gas_limit,
                max_fee_per_gas_wei: max_fee.to_string(),
                max_priority_fee_per_gas_wei: max_priority.to_string(),
            });
        }

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
                .map(|g| g.saturating_add(g * overrides.gas_buffer_pct.unwrap_or(20) as u64 / 100))
                .unwrap_or(65_000),
        };

        // If calldata override is set, use it instead of the auto-encoded transfer.
        let call_data = match overrides.calldata {
            Some(ref cd) => cd.as_slice(),
            None => &data,
        };

        let raw_tx = build_eip1559_tx(
            self.chain_id,
            nonce,
            max_fee,
            max_priority,
            gas_limit,
            contract,
            0u128,
            call_data,
            &overrides.access_list,
            private_key_bytes,
        )?;

        let hex_tx = format!("0x{}", hex::encode(&raw_tx));

        if overrides.sign_only {
            return Ok(EvmSendResult {
                txid: String::new(),
                nonce,
                raw_tx_hex: hex_tx,
                gas_limit,
                max_fee_per_gas_wei: max_fee.to_string(),
                max_priority_fee_per_gas_wei: max_priority.to_string(),
            });
        }

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
/// * `calldata` — arbitrary calldata bytes. For the native send path, overrides
///   the default empty data field (e.g. attaching a memo). For the ERC-20
///   path, overrides the auto-encoded `transfer(to, amount)` calldata, enabling
///   arbitrary contract calls (approve, swap, multicall, etc.).
/// * `access_list` — EIP-2930 access list. Pre-warms storage slots to reduce
///   gas on contracts with known read patterns.
/// * `sign_only` — build and sign the transaction but do not broadcast it.
///   The signed raw transaction is returned in `EvmSendResult.raw_tx_hex`
///   and `txid` is left empty.
#[derive(Debug, Clone, Default)]
pub struct EvmSendOverrides {
    pub nonce: Option<u64>,
    pub max_fee_per_gas_wei: Option<u128>,
    pub max_priority_fee_per_gas_wei: Option<u128>,
    pub gas_limit: Option<u64>,
    pub calldata: Option<Vec<u8>>,
    pub access_list: Vec<AccessListEntry>,
    pub sign_only: bool,
    /// Percentage buffer added to the `eth_estimateGas` result when
    /// `gas_limit` is not pinned. Default `20` (i.e. +20%).
    pub gas_buffer_pct: Option<u32>,
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

// EIP-2930 access list entry: (address, storage_keys).
// An empty access list is the common case; power users can supply one
// to pre-warm storage slots and reduce gas cost on subsequent reads.
#[derive(Debug, Clone, Default, alloy_rlp::RlpEncodable)]
pub struct AccessListEntry {
    pub address: [u8; 20],
    pub storage_keys: Vec<[u8; 32]>,
}

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
    access_list: &[AccessListEntry],
    private_key_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let to_bytes = decode_hex(to)?;
    if to_bytes.len() != 20 {
        return Err(format!("invalid EVM address length: {}", to_bytes.len()));
    }
    let to_arr: [u8; 20] = to_bytes.try_into().unwrap();

    // EIP-1559 signing payload:
    //   0x02 || RLP([chain_id, nonce, max_priority_fee, max_fee, gas_limit,
    //                to, value, data, access_list])
    let mut signing_payload = vec![0x02u8];
    encode_eip1559_fields(
        chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas,
        gas_limit, &to_arr, value_wei, data, access_list,
        None, None, None,   // no v/r/s yet
        &mut signing_payload,
    );

    let msg_hash = crate::derivation::chains::evm::keccak256(&signing_payload);

    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg = Message::from_digest_slice(&msg_hash).map_err(|e| format!("msg: {e}"))?;
    let (rec_id, sig_bytes) = secp
        .sign_ecdsa_recoverable(&msg, &secret_key)
        .serialize_compact();

    let v: u64 = rec_id.to_i32() as u64; // 0 or 1 for EIP-1559
    let r: [u8; 32] = sig_bytes[..32].try_into().unwrap();
    let s: [u8; 32] = sig_bytes[32..].try_into().unwrap();

    let mut raw = vec![0x02u8];
    encode_eip1559_fields(
        chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas,
        gas_limit, &to_arr, value_wei, data, access_list,
        Some(v), Some(&r), Some(&s),
        &mut raw,
    );
    Ok(raw)
}

/// Encode the EIP-1559 field list into `out`. When `v/r/s` are `None`,
/// produces the signing payload (no signature fields); when `Some`, produces
/// the full signed transaction body. Called twice to avoid duplicating the
/// field list.
#[allow(clippy::too_many_arguments)]
fn encode_eip1559_fields(
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: u128,
    max_fee_per_gas: u128,
    gas_limit: u64,
    to: &[u8; 20],
    value_wei: u128,
    data: &[u8],
    access_list: &[AccessListEntry],
    v: Option<u64>,
    r: Option<&[u8; 32]>,
    s: Option<&[u8; 32]>,
    out: &mut Vec<u8>,
) {
    use alloy_rlp::{Encodable, Header};

    // Collect the payload first so we can write the list header.
    let mut payload = Vec::new();
    chain_id.encode(&mut payload);
    nonce.encode(&mut payload);
    max_priority_fee_per_gas.encode(&mut payload);
    max_fee_per_gas.encode(&mut payload);
    gas_limit.encode(&mut payload);
    // `to` is a fixed-length address, encoded as a 20-byte string.
    Header { list: false, payload_length: 20 }.encode(&mut payload);
    payload.extend_from_slice(to);
    value_wei.encode(&mut payload);
    data.encode(&mut payload);
    // Access list as an RLP list of entries.
    let mut al_buf = Vec::new();
    for entry in access_list {
        alloy_rlp::Encodable::encode(entry, &mut al_buf);
    }
    Header { list: true, payload_length: al_buf.len() }.encode(&mut payload);
    payload.extend_from_slice(&al_buf);

    if let (Some(v), Some(r), Some(s)) = (v, r, s) {
        v.encode(&mut payload);
        // r and s are 32-byte big integers — strip leading zeros per RLP spec.
        encode_uint256(r, &mut payload);
        encode_uint256(s, &mut payload);
    }

    Header { list: true, payload_length: payload.len() }.encode(out);
    out.extend_from_slice(&payload);
}

/// Encode a 32-byte big-endian integer as a minimal RLP byte string
/// (strip leading zero bytes, then apply string header).
fn encode_uint256(bytes: &[u8; 32], out: &mut Vec<u8>) {
    use alloy_rlp::Header;
    let trimmed = bytes.iter().copied().skip_while(|&b| b == 0).collect::<Vec<_>>();
    if trimmed.is_empty() {
        // Zero: RLP empty string 0x80
        out.push(0x80);
    } else if trimmed.len() == 1 && trimmed[0] < 0x80 {
        out.push(trimmed[0]);
    } else {
        Header { list: false, payload_length: trimmed.len() }.encode(out);
        out.extend_from_slice(&trimmed);
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
    out.extend_from_slice(&SEL_TRANSFER);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&to_bytes);
    let mut amount_bytes = [0u8; 32];
    amount_bytes[16..].copy_from_slice(&amount.to_be_bytes());
    out.extend_from_slice(&amount_bytes);
    Ok(out)
}

/// Encode an `approve(address spender, uint256 amount)` call.
/// Use this to grant a contract (DEX, bridge, etc.) permission to spend tokens.
pub fn encode_erc20_approve(spender: &str, amount: u128) -> Result<Vec<u8>, String> {
    let spender_bytes = decode_hex(spender)?;
    if spender_bytes.len() != 20 {
        return Err(format!("invalid EVM spender length: {}", spender_bytes.len()));
    }
    let mut out = Vec::with_capacity(4 + 32 + 32);
    out.extend_from_slice(&SEL_APPROVE);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&spender_bytes);
    let mut amount_bytes = [0u8; 32];
    amount_bytes[16..].copy_from_slice(&amount.to_be_bytes());
    out.extend_from_slice(&amount_bytes);
    Ok(out)
}

/// Encode a `transferFrom(address from, address to, uint256 amount)` call.
/// Used for allowance-based pulls (escrow, bridge withdrawal, etc.).
pub fn encode_erc20_transfer_from(
    from: &str,
    to: &str,
    amount: u128,
) -> Result<Vec<u8>, String> {
    let from_bytes = decode_hex(from)?;
    let to_bytes = decode_hex(to)?;
    if from_bytes.len() != 20 {
        return Err(format!("invalid EVM from length: {}", from_bytes.len()));
    }
    if to_bytes.len() != 20 {
        return Err(format!("invalid EVM to length: {}", to_bytes.len()));
    }
    let mut out = Vec::with_capacity(4 + 32 + 32 + 32);
    out.extend_from_slice(&SEL_TRANSFER_FROM);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&from_bytes);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&to_bytes);
    let mut amount_bytes = [0u8; 32];
    amount_bytes[16..].copy_from_slice(&amount.to_be_bytes());
    out.extend_from_slice(&amount_bytes);
    Ok(out)
}

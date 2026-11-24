//! EVM send: EIP-1559 RLP builder and secp256k1 signer (native ETH + ERC-20
//! transfer paths, with optional override hooks for replacement-by-fee /
//! speed-up / cancel flows).

use crate::send::error::SendError;

use serde_json::json;

use crate::api::evm_json_rpc::{EvmClient, SEL_TRANSFER, decode_hex};

/// The exact EIP-1559 fields reviewed before signing. Contains no key material.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreparedEvmTransaction {
    pub chain_id: u64,
    pub nonce: u64,
    #[serde(with = "crate::send::stages::units_u128")]
    pub max_fee_per_gas: u128,
    #[serde(with = "crate::send::stages::units_u128")]
    pub max_priority_fee_per_gas: u128,
    pub gas_limit: u64,
    pub to: String,
    #[serde(with = "crate::send::stages::units_u128")]
    pub value_wei: u128,
    pub data: Vec<u8>,
    pub access_list: Vec<AccessListEntry>,
    /// Budget for network charges outside EIP-1559 execution gas (OP Stack L1/operator fees).
    #[serde(with = "crate::send::stages::units_u128")]
    pub additional_fee_wei: u128,
}

impl PreparedEvmTransaction {
    pub(crate) fn maximum_fee_wei(&self) -> Result<u128, SendError> {
        u128::from(self.gas_limit)
            .checked_mul(self.max_fee_per_gas)
            .and_then(|fee| fee.checked_add(self.additional_fee_wei))
            .ok_or_else(|| SendError::invalid("EVM fee budget overflow"))
    }
    /// Whether the network signs this as a legacy (EIP-155 type 0)
    /// transaction. Its one gas price is the reviewed max fee, so the
    /// priority fee must equal it, and it carries no access list.
    fn legacy(&self) -> Result<bool, SendError> {
        let chain = crate::registry::Chain::from_evm_chain_id(self.chain_id)
            .ok_or_else(|| SendError::invalid("unknown EVM network"))?;
        if !chain.evm_legacy_transactions() {
            return Ok(false);
        }
        if self.max_priority_fee_per_gas != self.max_fee_per_gas {
            return Err(SendError::invalid(
                "a legacy transaction has one gas price, not separate max and priority fees",
            ));
        }
        if !self.access_list.is_empty() {
            return Err(SendError::invalid(
                "a legacy transaction carries no access list",
            ));
        }
        Ok(true)
    }

    /// Inspect the exact bytes whose hash will be signed.
    pub fn signing_payload(&self) -> Result<Vec<u8>, SendError> {
        let to: [u8; 20] = decode_hex(&self.to)?
            .try_into()
            .map_err(|_| SendError::Invalid("invalid EVM destination".into()))?;
        if self.legacy()? {
            return Ok(encode_legacy_fields(self, &to, None));
        }
        let mut payload = vec![2];
        encode_eip1559_fields(
            self.chain_id,
            self.nonce,
            self.max_priority_fee_per_gas,
            self.max_fee_per_gas,
            self.gas_limit,
            &to,
            self.value_wei,
            &self.data,
            &self.access_list,
            None,
            None,
            None,
            &mut payload,
        );
        Ok(payload)
    }

    /// Offline signing cannot refresh or replace any reviewed field.
    pub fn sign(&self, key: &[u8]) -> Result<Vec<u8>, SendError> {
        if self.legacy()? {
            let payload = self.signing_payload()?;
            let to: [u8; 20] = decode_hex(&self.to)?
                .try_into()
                .map_err(|_| SendError::Invalid("invalid EVM destination".into()))?;
            let (recovery, r, s) = sign_digest(&crate::derivation::evm::keccak256(&payload), key)?;
            // EIP-155: v = chain id × 2 + 35 + the recovery bit.
            let v = self
                .chain_id
                .checked_mul(2)
                .and_then(|v| v.checked_add(35 + recovery))
                .ok_or_else(|| SendError::invalid("EVM chain id out of range"))?;
            return Ok(encode_legacy_fields(self, &to, Some((v, &r, &s))));
        }
        build_eip1559_tx(
            self.chain_id,
            self.nonce,
            self.max_fee_per_gas,
            self.max_priority_fee_per_gas,
            self.gas_limit,
            &self.to,
            self.value_wei,
            &self.data,
            &self.access_list,
            key,
        )
    }
}

/// Resolve all network-dependent fields without reading signing material.
pub async fn prepare_transfer(
    client: &EvmClient,
    from: &str,
    to: &str,
    value_wei: u128,
    data: &[u8],
    overrides: &EvmSendOverrides,
) -> Result<PreparedEvmTransaction, SendError> {
    let nonce = match overrides.nonce {
        Some(nonce) => nonce,
        None => client.fetch_nonce(from).await?,
    };
    let (max_fee_per_gas, mut max_priority_fee_per_gas) = resolve_fees(client, overrides).await?;
    if max_fee_per_gas == 0 || max_priority_fee_per_gas > max_fee_per_gas {
        return Err(SendError::Invalid("invalid EIP-1559 fees".into()));
    }
    // A legacy transaction pays its one gas price, the max fee, in full.
    if client.chain()?.evm_legacy_transactions() {
        max_priority_fee_per_gas = max_fee_per_gas;
    }
    let data = overrides.calldata.as_deref().unwrap_or(data);
    let gas_limit = resolve_gas(
        client,
        from,
        to,
        value_wei,
        data,
        nonce,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        overrides,
    )
    .await?;
    let mut prepared = PreparedEvmTransaction {
        chain_id: client.chain_id,
        nonce,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas_limit,
        to: to.into(),
        value_wei,
        data: data.into(),
        access_list: overrides.access_list.clone(),
        additional_fee_wei: 0,
    };
    let payload = prepared.signing_payload()?;
    prepared.additional_fee_wei = client.fetch_rollup_fee(&payload, gas_limit).await?;
    prepared.maximum_fee_wei()?;
    Ok(prepared)
}

// ── Send overrides + fee resolution

/// Optional overrides for EIP-1559 sends. Any `None` field falls back to the
/// default behavior (pending-nonce / recommended fee / estimated gas limit).
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
#[derive(Debug, Clone, Default)]
pub struct EvmSendOverrides {
    pub nonce: Option<u64>,
    pub max_fee_per_gas_wei: Option<u128>,
    pub max_priority_fee_per_gas_wei: Option<u128>,
    pub gas_limit: Option<u64>,
    pub calldata: Option<Vec<u8>>,
    pub access_list: Vec<AccessListEntry>,
    /// Percentage buffer added to the `eth_estimateGas` result when
    /// `gas_limit` is not pinned. The default is the registry's network-specific margin.
    pub gas_buffer_pct: Option<u32>,
}

/// Estimate the final transaction, including calldata, fees and access list.
async fn resolve_gas(
    client: &EvmClient,
    from: &str,
    to: &str,
    value: u128,
    data: &[u8],
    nonce: u64,
    max_fee: u128,
    priority: u128,
    overrides: &EvmSendOverrides,
) -> Result<u64, SendError> {
    if let Some(gas) = overrides.gas_limit {
        return if gas > 0 {
            Ok(gas)
        } else {
            Err(SendError::Invalid("gas limit must be positive".into()))
        };
    }
    let access_list: Vec<_> = overrides.access_list.iter().map(|entry| json!({
        "address": format!("0x{}", hex::encode(entry.address)),
        "storageKeys": entry.storage_keys.iter().map(|key| format!("0x{}", hex::encode(key))).collect::<Vec<_>>(),
    })).collect();
    // A legacy-only node estimates the transaction it will be sent: one gas
    // price and no access list.
    let request = if client.chain()?.evm_legacy_transactions() {
        if !access_list.is_empty() {
            return Err(SendError::invalid(
                "a legacy transaction carries no access list",
            ));
        }
        json!({
            "from": from, "to": to, "value": format!("0x{value:x}"),
            "data": format!("0x{}", hex::encode(data)), "nonce": format!("0x{nonce:x}"),
            "gasPrice": format!("0x{max_fee:x}"),
        })
    } else {
        json!({
            "from": from, "to": to, "value": format!("0x{value:x}"),
            "data": format!("0x{}", hex::encode(data)), "nonce": format!("0x{nonce:x}"),
            "maxFeePerGas": format!("0x{max_fee:x}"),
            "maxPriorityFeePerGas": format!("0x{priority:x}"), "accessList": access_list,
        })
    };
    let gas = client.estimate_transaction_gas(request).await?;
    gas_limit_with_margin(
        client.chain()?,
        gas,
        data.is_empty() && access_list.is_empty(),
        overrides.gas_buffer_pct,
    )
}

pub(crate) fn gas_limit_with_margin(
    chain: crate::registry::Chain,
    gas: u64,
    plain_transfer: bool,
    override_pct: Option<u32>,
) -> Result<u64, SendError> {
    // A plain EOA transfer is exactly 21,000 gas. Padding it on Monad costs real funds.
    if gas == 21_000 && plain_transfer && override_pct.is_none() {
        return Ok(gas);
    }
    let margin = override_pct
        .map(|pct| u128::from(pct) * 100)
        .unwrap_or_else(|| u128::from(chain.evm_gas_buffer_bps()));
    let buffered = u128::from(gas) * (10_000 + margin);
    u64::try_from(buffered.div_ceil(10_000))
        .map_err(|_| SendError::Invalid("buffered gas limit out of range".into()))
}

/// Resolve (max_fee_per_gas, max_priority_fee_per_gas) from overrides plus
/// fallback `fetch_fee_estimate()` values. If both fields are set, no RPC
/// call is made.
async fn resolve_fees(
    client: &EvmClient,
    overrides: &EvmSendOverrides,
) -> Result<(u128, u128), SendError> {
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

// ── EIP-1559 transaction builder + signer

// EIP-2930 access list entry: (address, storage_keys).
// An empty access list is the common case; power users can supply one
// to pre-warm storage slots and reduce gas cost on subsequent reads.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, alloy_rlp::RlpEncodable)]
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
) -> Result<Vec<u8>, SendError> {
    let to_bytes = decode_hex(to)?;
    if to_bytes.len() != 20 {
        return Err(SendError::Invalid(
            format!("invalid EVM address length: {}", to_bytes.len()).into(),
        ));
    }
    let to_arr: [u8; 20] = to_bytes.try_into().unwrap();

    // EIP-1559 signing payload:
    //   0x02 || RLP([chain_id, nonce, max_priority_fee, max_fee, gas_limit,
    //                to, value, data, access_list])
    let mut signing_payload = vec![0x02u8];
    encode_eip1559_fields(
        chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        &to_arr,
        value_wei,
        data,
        access_list,
        None,
        None,
        None, // no v/r/s yet
        &mut signing_payload,
    );

    let msg_hash = crate::derivation::evm::keccak256(&signing_payload);
    // The y-parity, 0 or 1, is EIP-1559's `v`.
    let (v, r, s) = sign_digest(&msg_hash, private_key_bytes)?;

    let mut raw = vec![0x02u8];
    encode_eip1559_fields(
        chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        &to_arr,
        value_wei,
        data,
        access_list,
        Some(v),
        Some(&r),
        Some(&s),
        &mut raw,
    );
    Ok(raw)
}

/// A recoverable secp256k1 signature of a 32-byte digest: the recovery bit
/// (0 or 1), then `r` and `s`.
fn sign_digest(digest: &[u8], key: &[u8]) -> Result<(u64, [u8; 32], [u8; 32]), SendError> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secret_key = SecretKey::from_slice(key)
        .map_err(|e| SendError::Invalid(format!("invalid key: {e}").into()))?;
    let msg =
        Message::from_digest_slice(digest).map_err(|e| SendError::Internal(format!("msg: {e}")))?;
    let (rec_id, sig_bytes) = Secp256k1::new()
        .sign_ecdsa_recoverable(&msg, &secret_key)
        .serialize_compact();
    let r: [u8; 32] = sig_bytes[..32].try_into().expect("64-byte signature");
    let s: [u8; 32] = sig_bytes[32..].try_into().expect("64-byte signature");
    Ok((rec_id.to_i32() as u64, r, s))
}

/// A legacy transaction's RLP list. Unsigned, it is EIP-155's signing
/// payload, `[nonce, gasPrice, gas, to, value, data, chainId, 0, 0]`; signed,
/// the last three are `v`, `r` and `s`. The gas price is the max fee.
fn encode_legacy_fields(
    tx: &PreparedEvmTransaction,
    to: &[u8; 20],
    signature: Option<(u64, &[u8; 32], &[u8; 32])>,
) -> Vec<u8> {
    use alloy_rlp::{Encodable, Header};
    let mut payload = Vec::new();
    tx.nonce.encode(&mut payload);
    tx.max_fee_per_gas.encode(&mut payload);
    tx.gas_limit.encode(&mut payload);
    Header {
        list: false,
        payload_length: 20,
    }
    .encode(&mut payload);
    payload.extend_from_slice(to);
    tx.value_wei.encode(&mut payload);
    tx.data.as_slice().encode(&mut payload);
    match signature {
        None => {
            tx.chain_id.encode(&mut payload);
            0u8.encode(&mut payload);
            0u8.encode(&mut payload);
        }
        Some((v, r, s)) => {
            v.encode(&mut payload);
            encode_uint256(r, &mut payload);
            encode_uint256(s, &mut payload);
        }
    }
    let mut out = Vec::new();
    Header {
        list: true,
        payload_length: payload.len(),
    }
    .encode(&mut out);
    out.extend_from_slice(&payload);
    out
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
    Header {
        list: false,
        payload_length: 20,
    }
    .encode(&mut payload);
    payload.extend_from_slice(to);
    value_wei.encode(&mut payload);
    data.encode(&mut payload);
    // Access list as an RLP list of entries.
    let mut al_buf = Vec::new();
    for entry in access_list {
        alloy_rlp::Encodable::encode(entry, &mut al_buf);
    }
    Header {
        list: true,
        payload_length: al_buf.len(),
    }
    .encode(&mut payload);
    payload.extend_from_slice(&al_buf);

    if let (Some(v), Some(r), Some(s)) = (v, r, s) {
        v.encode(&mut payload);
        // r and s are 32-byte big integers — strip leading zeros per RLP spec.
        encode_uint256(r, &mut payload);
        encode_uint256(s, &mut payload);
    }

    Header {
        list: true,
        payload_length: payload.len(),
    }
    .encode(out);
    out.extend_from_slice(&payload);
}

/// Encode a 32-byte big-endian integer as a minimal RLP byte string
/// (strip leading zero bytes, then apply string header).
fn encode_uint256(bytes: &[u8; 32], out: &mut Vec<u8>) {
    use alloy_rlp::Header;
    let trimmed = bytes
        .iter()
        .copied()
        .skip_while(|&b| b == 0)
        .collect::<Vec<_>>();
    if trimmed.is_empty() {
        // Zero: RLP empty string 0x80
        out.push(0x80);
    } else if trimmed.len() == 1 && trimmed[0] < 0x80 {
        out.push(trimmed[0]);
    } else {
        Header {
            list: false,
            payload_length: trimmed.len(),
        }
        .encode(out);
        out.extend_from_slice(&trimmed);
    }
}

// ── ERC-20 transfer ABI encoding (used by the send path)

/// The call an EVM transfer makes: the network's coin as `value` to the
/// recipient, or a token's ERC-20 `transfer` to its contract with no value.
/// The preview prices, and the build signs, this one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferCall {
    pub to: String,
    pub value: u128,
    pub data: Vec<u8>,
}

/// The call a transfer of `amount`, a decimal as typed, to `recipient`
/// makes on `chain`; `token` is the contract and its decimals when a token
/// moves.
pub fn transfer_call(
    chain: crate::registry::Chain,
    recipient: &str,
    amount: &str,
    token: Option<(&str, u32)>,
) -> Result<TransferCall, crate::error::SpectraBridgeError> {
    if !chain.is_evm() {
        return Err(SendError::invalid("Not an EVM network").into());
    }
    let address = |text: &str| -> Result<String, SendError> {
        let text = text.trim().to_ascii_lowercase();
        match text.strip_prefix("0x").map(hex::decode) {
            Some(Ok(bytes)) if bytes.len() == 20 => Ok(text),
            _ => Err(SendError::invalid("Invalid EVM address")),
        }
    };
    let recipient = address(recipient)?;
    Ok(match token {
        Some((contract, decimals)) => TransferCall {
            to: address(contract)?,
            value: 0,
            data: encode_erc20_transfer(
                &recipient,
                crate::amount::input::parse_raw_amount(amount, decimals)?,
            )?,
        },
        None => TransferCall {
            to: recipient,
            value: crate::amount::input::parse_raw_amount(
                amount,
                u32::from(chain.native_decimals()),
            )?,
            data: Vec::new(),
        },
    })
}

/// Encode a `transfer(address,uint256)` call.
pub(crate) fn encode_erc20_transfer(to: &str, amount: u128) -> Result<Vec<u8>, SendError> {
    encode_address_amount(SEL_TRANSFER, to, amount)
}

/// Encode an `approve(address,uint256)` call: `amount` 0 revokes.
pub(crate) fn encode_erc20_approve(spender: &str, amount: u128) -> Result<Vec<u8>, SendError> {
    encode_address_amount(crate::api::evm_json_rpc::SEL_APPROVE, spender, amount)
}

// ── ERC-721 / ERC-1155 transfer ABI encoding

const SEL_SAFE_TRANSFER_721: [u8; 4] = [0x42, 0x84, 0x2e, 0x0e]; // safeTransferFrom(address,address,uint256)
const SEL_SAFE_TRANSFER_1155: [u8; 4] = [0xf2, 0x42, 0x43, 0x2a]; // safeTransferFrom(address,address,uint256,uint256,bytes)

/// The call that moves an NFT from `from` to `to`: ERC-721's
/// `safeTransferFrom(from, to, tokenId)`, or ERC-1155's
/// `safeTransferFrom(from, to, id, quantity, "")`. Both are the safe forms: a
/// contract recipient must accept the token or the call reverts, so a token
/// cannot be stranded in a contract that cannot move it. An ERC-721 token is
/// one of a kind, so its quantity is one.
pub(crate) fn encode_nft_transfer(
    standard: crate::registry::NftStandard,
    from: &str,
    to: &str,
    token_id: &str,
    quantity: &str,
) -> Result<Vec<u8>, SendError> {
    use crate::api::evm_nft::{address_word, parse_uint256, uint256_word};
    use crate::registry::NftStandard;
    let id = uint256_word(&parse_uint256(token_id)?);
    let quantity = parse_uint256(quantity)?;
    let mut out = Vec::with_capacity(4 + 6 * 32);
    match standard {
        NftStandard::Erc721 => {
            if quantity != num_bigint::BigUint::from(1u8) {
                return Err(SendError::invalid(
                    "An ERC-721 token is sent one at a time.",
                ));
            }
            out.extend(SEL_SAFE_TRANSFER_721);
            out.extend(address_word(from)?);
            out.extend(address_word(to)?);
            out.extend(id);
        }
        NftStandard::Erc1155 => {
            if quantity == num_bigint::BigUint::ZERO {
                return Err(SendError::invalid("The quantity must be at least one."));
            }
            out.extend(SEL_SAFE_TRANSFER_1155);
            out.extend(address_word(from)?);
            out.extend(address_word(to)?);
            out.extend(id);
            out.extend(uint256_word(&quantity));
            // The empty `bytes data`: its offset past the five head words,
            // then its length, zero.
            out.extend(uint256_word(&num_bigint::BigUint::from(5u8 * 32)));
            out.extend([0u8; 32]);
        }
    }
    Ok(out)
}

fn encode_address_amount(selector: [u8; 4], to: &str, amount: u128) -> Result<Vec<u8>, SendError> {
    let to_bytes = decode_hex(to)?;
    if to_bytes.len() != 20 {
        return Err(SendError::Invalid(
            format!("invalid EVM to length: {}", to_bytes.len()).into(),
        ));
    }
    let mut out = Vec::with_capacity(4 + 32 + 32);
    out.extend_from_slice(&selector);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&to_bytes);
    let mut amount_bytes = [0u8; 32];
    amount_bytes[16..].copy_from_slice(&amount.to_be_bytes());
    out.extend_from_slice(&amount_bytes);
    Ok(out)
}

#[cfg(test)]
mod transfer_call_tests {
    use super::*;
    use crate::registry::Chain;

    const TO: &str = "0x2222222222222222222222222222222222222222";
    const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";

    fn wei(amount: &str) -> Result<u128, crate::error::SpectraBridgeError> {
        transfer_call(Chain::Ethereum, TO, amount, None).map(|call| call.value)
    }

    /// A typed decimal becomes exactly its own integer, including the
    /// amounts an `f64` rounds and eighteen significant decimals.
    #[test]
    fn a_typed_decimal_becomes_exactly_its_own_integer() {
        for (typed, expected) in [
            ("1.1", 1_100_000_000_000_000_000),
            ("0.1", 100_000_000_000_000_000),
            ("1234.5678", 1_234_567_800_000_000_000_000),
            (" 2.25 ", 2_250_000_000_000_000_000),
            (".25", 250_000_000_000_000_000),
            ("1.234567890123456789", 1_234_567_890_123_456_789),
            ("0.000000000000000001", 1),
            ("0", 0),
        ] {
            assert_eq!(wei(typed).unwrap(), expected, "{typed}");
        }
        for refused in ["1e3", "1.1234567890123456789", "-1", "NaN", "", "1..0"] {
            assert!(wei(refused).is_err(), "{refused}");
        }
    }

    /// The coin moves as value to the recipient on every EVM mainnet; a token
    /// moves as its contract's `transfer`, at the contract's precision, with
    /// no value.
    #[test]
    fn the_coin_is_value_and_a_token_is_its_contract_call() {
        for chain in Chain::all().filter(|c| c.is_evm() && !c.is_testnet()) {
            let call = transfer_call(chain, TO, "1", None).unwrap();
            assert_eq!((call.to.as_str(), call.data.len()), (TO, 0), "{chain:?}");
        }
        let call = transfer_call(Chain::Ethereum, TO, "100", Some((USDC, 6))).unwrap();
        assert_eq!(call.to, USDC.to_ascii_lowercase());
        assert_eq!(call.value, 0);
        assert_eq!(call.data, encode_erc20_transfer(TO, 100_000_000).unwrap());
        assert!(transfer_call(Chain::Ethereum, TO, "1.1234567", Some((USDC, 6))).is_err());
        assert!(transfer_call(Chain::Ethereum, "alice.eth", "1", None).is_err());
        assert!(transfer_call(Chain::Ethereum, TO, "1", Some(("not-a-contract", 6))).is_err());
        assert!(transfer_call(Chain::Bitcoin, TO, "1", None).is_err());
    }
}

#[cfg(test)]
mod gas_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

    #[tokio::test]
    async fn signing_estimates_final_payload_and_refuses_failed_gas() {
        let address = "0x1111111111111111111111111111111111111111";
        for token in [false, true] {
            for estimate in [
                Some("0x7531"),
                None,
                Some("0x0"),
                Some("0xffffffffffffffff"),
            ] {
                let server = MockServer::start().await;
                Mock::given(any()).respond_with(move |req: &Request| {
                    let body: serde_json::Value = req.body_json().unwrap();
                    assert_eq!(body["method"], "eth_estimateGas", "must not broadcast after estimate failure");
                    let tx = &body["params"][0];
                    assert_eq!(tx["data"], "0xaabb");
                    assert_eq!(tx["to"], address);
                    assert_eq!(tx["value"], if token { "0x0" } else { "0x7" });
                    assert_eq!(tx["nonce"], "0x3");
                    assert_eq!(tx["maxFeePerGas"], "0xa");
                    assert_eq!(tx["accessList"][0]["address"], address);
                    assert_eq!(tx["accessList"][0]["storageKeys"][0], format!("0x{}", "22".repeat(32)));
                    ResponseTemplate::new(200).set_body_json(match estimate {
                        Some(gas) => json!({"jsonrpc":"2.0", "id":body["id"], "result":gas}),
                        None => json!({"jsonrpc":"2.0", "id":body["id"], "error":{"code":-32000,"message":"reverted"}}),
                    })
                }).mount(&server).await;
                let client = EvmClient::new(Arc::new(vec![server.uri()]), 1);
                let overrides = EvmSendOverrides {
                    nonce: Some(3),
                    max_fee_per_gas_wei: Some(10),
                    max_priority_fee_per_gas_wei: Some(1),
                    calldata: Some(vec![0xaa, 0xbb]),
                    access_list: vec![AccessListEntry {
                        address: [0x11; 20],
                        storage_keys: vec![[0x22; 32]],
                    }],
                    ..Default::default()
                };
                let data = if token {
                    encode_erc20_transfer(address, 7).unwrap()
                } else {
                    vec![]
                };
                let result = prepare_transfer(
                    &client,
                    address,
                    address,
                    if token { 0 } else { 7 },
                    &data,
                    &overrides,
                )
                .await;
                if estimate == Some("0x7531") {
                    let tx = result.unwrap();
                    assert_eq!(tx.gas_limit, 36002);
                    assert!(!tx.sign(&[1; 32]).unwrap().is_empty());
                } else {
                    assert!(result.is_err(), "{estimate:?}");
                }
                assert!(!server.received_requests().await.unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn explicit_gas_signs_offline_and_zero_is_refused() {
        let client = EvmClient::new(Arc::new(vec![]), 1);
        let address = "0x1111111111111111111111111111111111111111";
        for gas in [0, 21000] {
            let result = prepare_transfer(
                &client,
                address,
                address,
                1,
                &[],
                &EvmSendOverrides {
                    nonce: Some(0),
                    max_fee_per_gas_wei: Some(2),
                    max_priority_fee_per_gas_wei: Some(1),
                    gas_limit: Some(gas),
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(result.is_ok(), gas > 0);
        }
    }
}

#[cfg(test)]
mod nft_vectors {
    use super::*;
    use crate::api::evm_nft::{balance_of_call, owner_of_call, supports_interface_call};
    use crate::registry::NftStandard;

    /// Every call ethers.js 6.17.0 encodes for the same transfers and reads,
    /// token ids from zero to 2^256 − 1 and quantities to 2^128 − 1.
    #[test]
    fn calls_match_ethers() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/nft-transfer-vectors.json"
        ))
        .unwrap();
        let (from, to) = (
            vectors["from"].as_str().unwrap(),
            vectors["to"].as_str().unwrap(),
        );
        let hex = |bytes: Vec<u8>| format!("0x{}", hex::encode(bytes));
        for case in vectors["erc721"].as_array().unwrap() {
            let id = case["token_id"].as_str().unwrap();
            assert_eq!(
                hex(encode_nft_transfer(NftStandard::Erc721, from, to, id, "1").unwrap()),
                case["transfer"]
            );
            assert_eq!(hex(owner_of_call(id).unwrap()), case["owner_of"]);
            // One of a kind: never a quantity of an ERC-721 token.
            for quantity in ["0", "2"] {
                assert!(encode_nft_transfer(NftStandard::Erc721, from, to, id, quantity).is_err());
            }
        }
        for case in vectors["erc1155"].as_array().unwrap() {
            let id = case["token_id"].as_str().unwrap();
            let quantity = case["quantity"].as_str().unwrap();
            assert_eq!(
                hex(encode_nft_transfer(NftStandard::Erc1155, from, to, id, quantity).unwrap()),
                case["transfer"]
            );
            assert_eq!(hex(balance_of_call(from, id).unwrap()), case["balance_of"]);
            assert!(encode_nft_transfer(NftStandard::Erc1155, from, to, id, "0").is_err());
        }
        assert_eq!(
            hex(supports_interface_call(NftStandard::Erc721.interface_id())),
            vectors["supports"]["erc721"]
        );
        assert_eq!(
            hex(supports_interface_call(NftStandard::Erc1155.interface_id())),
            vectors["supports"]["erc1155"]
        );
    }

    /// The two transfers ethers signs with the same fields: one ERC-721
    /// token and three of an ERC-1155 id, byte for byte.
    #[test]
    fn signed_transfers_match_ethers() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/nft-transfer-vectors.json"
        ))
        .unwrap();
        let signed = &vectors["signed"];
        let (signer, recipient) = (
            signed["signer"].as_str().unwrap(),
            signed["recipient"].as_str().unwrap(),
        );
        for (standard, case, id, quantity) in [
            (NftStandard::Erc721, &signed["erc721"], "1234", "1"),
            (NftStandard::Erc1155, &signed["erc1155"], "7", "3"),
        ] {
            let prepared = PreparedEvmTransaction {
                chain_id: 1,
                nonce: case["nonce"].as_u64().unwrap(),
                max_fee_per_gas: 4_000_000_000,
                max_priority_fee_per_gas: 2_000_000_000,
                gas_limit: 108_000,
                to: case["to"].as_str().unwrap().into(),
                value_wei: 0,
                data: encode_nft_transfer(standard, signer, recipient, id, quantity).unwrap(),
                access_list: vec![],
                additional_fee_wei: 0,
            };
            assert_eq!(
                format!("0x{}", hex::encode(prepared.sign(&[1; 32]).unwrap())),
                case["raw"]
            );
        }
    }

    /// A token id or quantity is a whole number, never a decimal amount: a
    /// fraction, an exponent, a sign or 2^256 is refused rather than scaled.
    #[test]
    fn ids_and_quantities_are_never_amounts() {
        let (from, to) = (
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        );
        let past = (num_bigint::BigUint::from(1u8) << 256u32).to_string();
        for bad in ["1.0", "0.5", "1e18", "-1", "", " 1", past.as_str()] {
            assert!(
                encode_nft_transfer(NftStandard::Erc1155, from, to, bad, "1").is_err(),
                "id {bad}"
            );
            assert!(
                encode_nft_transfer(NftStandard::Erc1155, from, to, "1", bad).is_err(),
                "quantity {bad}"
            );
        }
    }
}

#[cfg(test)]
mod legacy_vectors {
    use super::*;
    use std::sync::Arc;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

    fn prepared(case: &serde_json::Value) -> PreparedEvmTransaction {
        let price: u128 = case["gas_price"].as_str().unwrap().parse().unwrap();
        PreparedEvmTransaction {
            chain_id: case["chain_id"].as_u64().unwrap(),
            nonce: case["nonce"].as_u64().unwrap(),
            max_fee_per_gas: price,
            max_priority_fee_per_gas: price,
            gas_limit: case["gas_limit"].as_u64().unwrap(),
            to: case["to"].as_str().unwrap().into(),
            value_wei: case["value"].as_str().unwrap().parse().unwrap(),
            data: decode_hex(case["data"].as_str().unwrap()).unwrap(),
            access_list: vec![],
            additional_fee_wei: 0,
        }
    }

    /// The EIP-155 signing payload and the signed transaction ethers.js 6.17.0
    /// produces for the same fields, byte for byte, on each legacy-only
    /// network: a native transfer, an ERC-20 transfer and a gas price past
    /// 2^32 wei.
    #[test]
    fn legacy_transactions_match_ethers() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/evm-legacy-transactions.json"
        ))
        .unwrap();
        let key = decode_hex(vectors["key"].as_str().unwrap()).unwrap();
        let hex = |bytes: Vec<u8>| format!("0x{}", hex::encode(bytes));
        for case in vectors["transactions"].as_array().unwrap() {
            let tx = prepared(case);
            assert!(
                crate::registry::Chain::from_evm_chain_id(tx.chain_id)
                    .unwrap()
                    .evm_legacy_transactions()
            );
            assert_eq!(
                hex(tx.signing_payload().unwrap()),
                case["unsigned"],
                "{}",
                case["name"]
            );
            let raw = tx.sign(&key).unwrap();
            assert_eq!(hex(raw.clone()), case["raw"], "{}", case["name"]);
            assert_eq!(
                hex(crate::derivation::evm::keccak256(&raw).to_vec()),
                case["hash"],
                "{}",
                case["name"]
            );
        }
    }

    /// A legacy transaction has one gas price and no access list: fields that
    /// say otherwise are refused before anything is signed, never dropped.
    #[test]
    fn legacy_fields_it_cannot_carry_are_refused() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/evm-legacy-transactions.json"
        ))
        .unwrap();
        let case = &vectors["transactions"][0];
        let key = [1u8; 32];
        let mut split = prepared(case);
        split.max_priority_fee_per_gas -= 1;
        let mut listed = prepared(case);
        listed.access_list = vec![AccessListEntry {
            address: [0x11; 20],
            storage_keys: vec![],
        }];
        for tx in [split, listed] {
            assert!(tx.signing_payload().is_err());
            assert!(tx.sign(&key).is_err());
        }
    }

    /// On a legacy-only network the estimate asks about the transaction that
    /// will be sent — one gas price, no fee-market fields — and the reviewed
    /// priority fee is that price, whatever was asked for.
    #[tokio::test]
    async fn a_legacy_network_estimates_and_prepares_one_gas_price() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                assert_eq!(body["method"], "eth_estimateGas");
                let tx = &body["params"][0];
                assert_eq!(tx["gasPrice"], "0xa");
                assert!(tx.get("maxFeePerGas").is_none() && tx.get("accessList").is_none());
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0", "id":body["id"], "result":"0x5208"}))
            })
            .mount(&server)
            .await;
        let client = EvmClient::new(Arc::new(vec![server.uri()]), 4200);
        let address = "0x2222222222222222222222222222222222222222";
        let overrides = EvmSendOverrides {
            nonce: Some(3),
            max_fee_per_gas_wei: Some(10),
            max_priority_fee_per_gas_wei: Some(1),
            ..Default::default()
        };
        let prepared = prepare_transfer(&client, address, address, 7, &[], &overrides)
            .await
            .unwrap();
        assert_eq!(
            (prepared.max_fee_per_gas, prepared.max_priority_fee_per_gas),
            (10, 10)
        );
        // An RLP list from its first byte: no typed-envelope prefix.
        assert!(prepared.signing_payload().unwrap()[0] >= 0xc0);
        let refused = EvmSendOverrides {
            access_list: vec![AccessListEntry {
                address: [0x11; 20],
                storage_keys: vec![],
            }],
            ..overrides
        };
        assert!(
            prepare_transfer(&client, address, address, 7, &[], &refused)
                .await
                .is_err()
        );
    }
}

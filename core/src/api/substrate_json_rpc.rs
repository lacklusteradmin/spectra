//! The Substrate JSON-RPC adapter, for Polkadot and Bittensor alike: the
//! `System.Account` balance, what signing needs, and extrinsic submission. A
//! node keeps no account history, and no keyless indexer is configured.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;

/// `twox128("System") ++ twox128("Account")`: the storage prefix of every
/// account record, fixed by the pallet and item names.
const SYSTEM_ACCOUNT_PREFIX: [u8; 32] = [
    0x26, 0xaa, 0x39, 0x4e, 0xea, 0x56, 0x30, 0xe0, 0x7c, 0x48, 0xae, 0x0c, 0x95, 0x58, 0xce, 0xf7,
    0xb9, 0x9d, 0x88, 0x0e, 0xc6, 0x81, 0x79, 0x9c, 0x0c, 0xf3, 0x0e, 0x88, 0x86, 0x37, 0x1d, 0xa9,
];

/// An account's `AccountData`, in the chain's smallest unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubstrateBalance {
    pub free: u128,
    pub reserved: u128,
    pub frozen: u128,
}

impl SubstrateBalance {
    /// What the account can move: `free` less the part `frozen` holds beyond
    /// what is already reserved. Staked funds are frozen, so they are not in it.
    pub fn transferable(self) -> u128 {
        self.free
            .saturating_sub(self.frozen.saturating_sub(self.reserved))
    }
}

/// `System.Account`'s key for `account`: the prefix, `blake2_128(account)`,
/// then the account itself (`Blake2_128Concat`).
fn system_account_key(account: &[u8; 32]) -> String {
    use blake2::digest::consts::U16;
    use blake2::{Blake2b, Digest};
    let mut key = SYSTEM_ACCOUNT_PREFIX.to_vec();
    key.extend_from_slice(&Blake2b::<U16>::digest(account));
    key.extend_from_slice(account);
    format!("0x{}", hex::encode(key))
}

/// `AccountInfo`: four `u32` counters (nonce, consumers, providers,
/// sufficients), then `free`, `reserved` and `frozen` of `balance_bytes` each
/// and a `u128` of flags, all little-endian. Any other length is refused.
fn decode_account_info(bytes: &[u8], balance_bytes: usize) -> Result<SubstrateBalance, ApiError> {
    if balance_bytes > 16 || bytes.len() != 16 + 3 * balance_bytes + 16 {
        return Err(ApiError::Decode(format!(
            "Substrate account record is {} bytes, not the expected layout",
            bytes.len()
        )));
    }
    let read = |index: usize| {
        let start = 16 + index * balance_bytes;
        let mut word = [0u8; 16];
        word[..balance_bytes].copy_from_slice(&bytes[start..start + balance_bytes]);
        u128::from_le_bytes(word)
    };
    Ok(SubstrateBalance {
        free: read(0),
        reserved: read(1),
        frozen: read(2),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateSendResult {
    pub txid: String,
    /// Hex-encoded signed extrinsic (0x-prefixed) — stored for rebroadcast.
    pub extrinsic_hex: String,
}

pub struct SubstrateClient {
    pub(crate) rpc_endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl SubstrateClient {
    pub fn new(rpc_endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            rpc_endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn rpc_call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            crate::EndpointApi::SubstrateJsonRpc,
            &self.client,
            &self.rpc_endpoints,
            method,
            params,
        )
        .await
    }

    /// The account's balance, zero when the chain has no record of it.
    /// `balance_bytes` is the chain's `Balance` width.
    pub async fn fetch_balance(
        &self,
        account: &[u8; 32],
        balance_bytes: usize,
    ) -> Result<SubstrateBalance, ApiError> {
        match self
            .rpc_call("state_getStorage", json!([system_account_key(account)]))
            .await?
        {
            Value::Null => Ok(SubstrateBalance {
                free: 0,
                reserved: 0,
                frozen: 0,
            }),
            Value::String(hex) => decode_account_info(
                &hex::decode(hex.trim_start_matches("0x"))
                    .map_err(|_| ApiError::Decode("Substrate account record is not hex".into()))?,
                balance_bytes,
            ),
            other => Err(ApiError::Decode(format!(
                "state_getStorage: unexpected {other}"
            ))),
        }
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u32, ApiError> {
        let result = self
            .rpc_call("system_accountNextIndex", json!([address]))
            .await?;
        result
            .as_u64()
            .map(|n| n as u32)
            .or_decode("system_accountNextIndex: expected number")
    }

    pub async fn fetch_runtime_version(&self) -> Result<(u32, u32), ApiError> {
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

    pub async fn fetch_genesis_hash(&self) -> Result<String, ApiError> {
        let result = self.rpc_call("chain_getBlockHash", json!([0])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .or_decode("chain_getBlockHash: expected string")
    }

    pub async fn fetch_block_hash_latest(&self) -> Result<String, ApiError> {
        let result = self.rpc_call("chain_getBlockHash", json!([])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .or_decode("chain_getBlockHash: expected string")
    }

    /// Submit a signed extrinsic, fresh or saved for rebroadcast.
    pub async fn submit_extrinsic_hex(&self, hex: &str) -> Result<SubstrateSendResult, ApiError> {
        let result = self
            .rpc_call("author_submitExtrinsic", json!([hex]))
            .await?;
        let txid = result.as_str().unwrap_or("").to_string();
        Ok(SubstrateSendResult {
            txid,
            extrinsic_hex: hex.to_string(),
        })
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;

    /// Records read from `rpc.polkadot.io` (the treasury, `u128` balances)
    /// and `entrypoint-finney.opentensor.ai` (`u64` balances), 2026-09-29.
    #[test]
    fn account_records_decode_at_each_chains_balance_width() {
        let treasury: [u8; 32] =
            hex::decode("6d6f646c70792f74727372790000000000000000000000000000000000000000")
                .unwrap()
                .try_into()
                .unwrap();
        assert_eq!(
            system_account_key(&treasury),
            "0x26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da95ecffd7b6c0f78751baa9d281e0bfa3a6d6f646c70792f74727372790000000000000000000000000000000000000000"
        );
        let polkadot = hex::decode("000000000000000001000000000000001a8ea401a31900000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000080").unwrap();
        assert_eq!(
            decode_account_info(&polkadot, 16).unwrap(),
            SubstrateBalance {
                free: 28_187_897_925_146,
                reserved: 0,
                frozen: 0
            }
        );
        let bittensor = hex::decode("88ff1000000000000100000000000000a006fe05000000000000000000000000000000000000000000000000000000000000000000000080").unwrap();
        assert_eq!(
            decode_account_info(&bittensor, 8).unwrap(),
            SubstrateBalance {
                free: 100_533_920,
                reserved: 0,
                frozen: 0
            }
        );
        // Read at the wrong width, either record is refused, not misread.
        assert!(decode_account_info(&polkadot, 8).is_err());
        assert!(decode_account_info(&bittensor, 16).is_err());
    }

    #[test]
    fn frozen_funds_beyond_the_reserve_are_not_transferable() {
        let staked = SubstrateBalance {
            free: 100,
            reserved: 10,
            frozen: 40,
        };
        assert_eq!(staked.transferable(), 70);
        let covered = SubstrateBalance {
            free: 100,
            reserved: 50,
            frozen: 40,
        };
        assert_eq!(covered.transferable(), 100);
    }
}
